#!/bin/sh
# Pushes a development build of nestlocal and/or the UNFR0ZEN screen to a
# unit already running unfr0zen, over its SSH (--ssh), without USB:
#
#     utils/push-over-ssh.sh [--nestlocal] [--screen] [--build] [--dry-run] [--log FILE]
#                            [--wake mqtt://[USER@]BROKER[:PORT]] [--wake-topic TOPIC] root@HOST
#
#   --nestlocal  push build/nestlocal-armv7 to /nestlabs/sbin/nestlocal, and
#                crates/nestlocal/rootfs/nestlocal.init to /etc/init.d/nestlocal
#   --screen     patch the unit's own stock nlclient (/etc/unfr0zen/nlclient.original,
#                fetched once into .work/push/) with this checkout's screen, and push
#                it with its undo journal
#   --build      rebuild what is pushed first: nestlocal, or the screen object and the
#                app that links it (in the firmware build image on macOS, as
#                utils/build-firmware.sh does; it needs one full build first)
#   --dry-run    build and check everything, then change nothing on the unit
#   --log FILE   where the log goes (default .work/push-over-ssh.log)
#   --wake URL   wake the unit through the MQTT broker nestlocal uses, with
#                mosquitto_pub and mosquitto_sub (brew install mosquitto); default
#                $UNFR0ZEN_WAKE. The password, if any, is read from
#                $UNFR0ZEN_WAKE_PASSWORD_FILE, else ~/.config/unfr0zen/mqtt.password
#   --wake-topic the unit's topic (nestlocal/SERIAL); by default the one unit whose
#                availability the broker keeps under nestlocal/
#
# Each file is streamed to a temporary name beside its destination (the unit's
# dropbear has neither scp nor sftp), checked against its hash, and renamed
# over the old one; the old one is kept in /tmp (lost on reboot) until the
# service is back. nestlocal restarts under monit; nlclient is stopped and
# started as the Doom launcher does. A service that does not stay up is put
# back as it was. One SSH connection serves the whole run, so a password is
# asked once. The bootloader, kernel, radio firmware and recovery still need USB.
#
# The unit sleeps when left alone, and asleep it drops the connection. So
# everything is built before connecting, and each command on the unit first
# touches /var/run/unfr0zen/push-awake, which nestlocal reads as a hold: it
# declines sleep for 2 minutes after each touch. Connecting is tried for about
# a minute. With --wake, set/stay_awake is published first and before each try:
# the broker's message wakes the unit, and nestlocal holds it awake until the
# run ends. Without it, turn the unit's ring when asked.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
work=.work/push
log=.work/push-over-ssh.log
nestlocal=0 screen=0 build=0 dry=0 host=
wake=${UNFR0ZEN_WAKE:-} wake_topic=
while [ $# -gt 0 ]; do
    case $1 in
        --nestlocal) nestlocal=1 ;;
        --screen) screen=1 ;;
        --build) build=1 ;;
        --dry-run) dry=1 ;;
        --log) [ $# -gt 1 ] || { echo "--log needs a file" >&2; exit 2; }; log=$2; shift ;;
        --wake) [ $# -gt 1 ] || { echo "--wake needs mqtt://[USER@]BROKER[:PORT]" >&2; exit 2; }; wake=$2; shift ;;
        --wake-topic) [ $# -gt 1 ] || { echo "--wake-topic needs a topic" >&2; exit 2; }; wake_topic=$2; shift ;;
        -h|--help) sed -n '2,/^set -eu/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
        -*) echo "unknown option $1" >&2; exit 2 ;;
        *) [ -z "$host" ] || { echo "one host only" >&2; exit 2; }; host=$1 ;;
    esac
    shift
done
[ -n "$host" ] || { echo "usage: $0 [--nestlocal] [--screen] [--build] [--dry-run] [--log FILE] root@HOST" >&2; exit 2; }
[ $((nestlocal + screen)) -gt 0 ] || { echo "nothing to push: give --nestlocal and/or --screen" >&2; exit 2; }
mkdir -p "$work" "$(dirname -- "$log")"
: > "$log"
say() { printf '%s\n' "$*" | tee -a "$log"; }
fail() { say "FAILED: $*"; exit 1; }
# Output of a command goes to the log only; its last lines to the terminal on failure.
quiet() { "$@" >> "$log" 2>&1 || { tail -n 30 "$log" >&2; fail "$*"; }; }
say "push-over-ssh $(date -u +%Y-%m-%dT%H:%M:%SZ) to $host from $(git describe --always --dirty 2>/dev/null || echo unknown)"

if command -v sha256sum > /dev/null; then hash() { sha256sum "$1" | cut -d' ' -f1; }
else hash() { shasum -a 256 "$1" | cut -d' ' -f1; }; fi

in_image() {  # the firmware build image on macOS, as utils/build-firmware.sh runs it
    if [ "$(uname -s)" = Darwin ]; then
        quiet docker build -q -t unfr0zen-build - < utils/ci/Dockerfile
        quiet docker run --rm -v "$root:$root" -w "$root" --user "$(id -u):$(id -g)" \
            -e HOME=/tmp -e CARGO_HOME="$root/.work/cargo" "unfr0zen-build" "$@"
    else
        quiet "$@"
    fi
}

# --- build locally, before connecting ------------------------------------------
if [ $nestlocal = 1 ]; then
    if [ $build = 1 ]; then
        say "Building nestlocal (log: $log)"
        in_image sh platform/common/radio/thread/nestlocal.sh
    fi
    [ -f build/nestlocal-armv7 ] || fail "build/nestlocal-armv7 is missing: run utils/build-firmware.sh, or --build"
    nestlocal_hash=$(hash build/nestlocal-armv7)
    say "nestlocal: $(wc -c < build/nestlocal-armv7 | tr -d ' ') bytes, sha256 $nestlocal_hash"
fi

stock=$work/nlclient.original
# patch_screen: the stock nlclient, patched with this checkout's screen.
patch_screen() {
    say "Patching the stock nlclient with this checkout's screen"
    rm -f "$work/nlclient" "$work/nlclient.undo.gz" "$work/nlclient.json"
    quiet cargo run --quiet --release -p unfr0zen-gui -- nlclient "$stock" --out "$work/nlclient"
}
if [ $screen = 1 ]; then
    if [ $build = 1 ]; then
        say "Building the screen object (log: $log)"
        in_image sh platform/common/nlclient/build.sh build
    fi
    say "Building the patcher (log: $log)"
    quiet cargo build --quiet --release -p unfr0zen-gui
    # The unit's stock nlclient from an earlier run; checked against the unit below.
    [ ! -f "$stock" ] || patch_screen
fi

# --- wake --------------------------------------------------------------------
if [ -n "$wake" ]; then
    command -v mosquitto_pub > /dev/null && command -v mosquitto_sub > /dev/null ||
        fail "--wake needs mosquitto_pub and mosquitto_sub: brew install mosquitto"
    broker=${wake#mqtt://}
    case $broker in *@*) wake_user=${broker%%@*}; broker=${broker#*@} ;; *) wake_user= ;; esac
    case $broker in *:*) wake_port=${broker##*:}; broker=${broker%:*} ;; *) wake_port=1883 ;; esac
    password_file=${UNFR0ZEN_WAKE_PASSWORD_FILE:-$HOME/.config/unfr0zen/mqtt.password}
    wake_pass=
    [ ! -f "$password_file" ] || wake_pass=$(head -n 1 "$password_file")
    # mq TOOL ARGS...: a mosquitto client on the broker.
    mq() {
        tool=$1; shift
        [ -z "$wake_pass" ] || set -- -P "$wake_pass" "$@"
        [ -z "$wake_user" ] || set -- -u "$wake_user" "$@"
        "$tool" -h "$broker" -p "$wake_port" "$@"
    }
    if [ -z "$wake_topic" ]; then
        units=$(mq mosquitto_sub -t 'nestlocal/+/availability' -v -W 3 2>/dev/null | sed 's#/availability .*##' | sort -u || true)
        case $(printf '%s' "$units" | grep -c .) in
            1) wake_topic=$units ;;
            0) fail "no nestlocal/+/availability on $broker: check --wake, or give --wake-topic" ;;
            *) fail "more than one unit on $broker; give --wake-topic, one of: $(echo $units)" ;;
        esac
    fi
    # wake_unit SECONDS: wakes the unit and holds it awake that long; 0 releases.
    wake_unit() { mq mosquitto_pub -q 1 -t "$wake_topic/set/stay_awake" -m "$1" 2>> "$log"; }
    wake_unit 900 || fail "cannot publish to $broker: check --wake and the password file"
    say "Waking $wake_topic through $broker"
fi

# --- connect -----------------------------------------------------------------
# One connection for the run: the password, if any, is asked once.
# /tmp: a socket path under macOS's TMPDIR can exceed the 104-byte limit.
control=$(mktemp -d /tmp/unfr0zen-ssh.XXXXXX)
ssh_opts="-o ControlMaster=auto -o ControlPath=$control/c -o ControlPersist=600 -o ServerAliveInterval=15 -o ConnectTimeout=10"
awake=/var/run/unfr0zen/push-awake
connected=0
cleanup() {
    [ $connected = 0 ] || ssh $ssh_opts "$host" "rm -f $awake" > /dev/null 2>&1 || true
    [ -z "$wake" ] || wake_unit 0 > /dev/null 2>&1 || true
    ssh $ssh_opts -O exit "$host" > /dev/null 2>&1 || true
    rm -rf "$control"
}
trap cleanup EXIT
# on COMMAND: run on the unit, renewing the hold that keeps it awake first.
on() { ssh $ssh_opts "$host" "mkdir -p ${awake%/*} 2>/dev/null; touch $awake 2>/dev/null; $*"; }
# A sleeping unit does not answer: try for about a minute.
tries=0
until on true 2>> "$log"; do
    tries=$((tries + 1))
    [ $tries -lt 6 ] || fail "cannot reach $host over SSH: wake the unit (turn its ring) and run again"
    [ $tries -gt 1 ] || if [ -n "$wake" ]; then say "Waiting for $host to wake"; else say "Waiting for $host: turn the unit's ring to wake it"; fi
    [ -z "$wake" ] || wake_unit 900 || true
    sleep 2
done
connected=1
on 'command -v sha256sum > /dev/null && command -v monit > /dev/null' ||
    fail "the unit lacks sha256sum or monit"
remote_hash() { on "sha256sum '$1' 2>/dev/null | cut -d' ' -f1"; }

# --- check against the unit ----------------------------------------------------
if [ $screen = 1 ]; then
    original_hash=$(remote_hash /etc/unfr0zen/nlclient.original)
    [ -n "$original_hash" ] || fail "the unit has no /etc/unfr0zen/nlclient.original: patch it once with the app"
    if [ ! -f "$stock" ] || [ "$(hash "$stock")" != "$original_hash" ]; then
        say "Fetching the unit's stock nlclient"
        on "cat /etc/unfr0zen/nlclient.original" > "$stock.partial"
        [ "$(hash "$stock.partial")" = "$original_hash" ] || fail "the stock nlclient did not arrive intact"
        mv "$stock.partial" "$stock"
        patch_screen
    fi
    nlclient_hash=$(hash "$work/nlclient")
    journal_hash=$(hash "$work/nlclient.undo.gz")
    grep -q "\"$original_hash\"" "$work/nlclient.json" || fail "the manifest is not for the unit's stock nlclient"
    say "nlclient: sha256 $nlclient_hash"
    [ "$(remote_hash /nestlabs/sbin/nlclient)" != "$nlclient_hash" ] || say "nlclient: the unit already runs this build"
fi

# --- on the unit ---------------------------------------------------------------
# stage FILE DEST HASH: FILE beside DEST as DEST.unfr0zen-new, checked.
stage() {
    size=$(wc -c < "$1" | tr -d ' ')
    free=$(on "df -k '$(dirname -- "$2")' | tail -n 1" | awk '{print $(NF-2)}')
    [ -n "$free" ] || fail "lost $host while checking free space: wake the unit and run again"
    [ $((free * 1024)) -gt $((size + 1048576)) ] || fail "$(dirname -- "$2") has ${free}K free, $2 needs $((size / 1024))K"
    on "cat > '$2.unfr0zen-new'" < "$1"
    [ "$(remote_hash "$2.unfr0zen-new")" = "$3" ] || { on "rm -f '$2.unfr0zen-new'"; fail "$2 did not arrive intact"; }
    on "chmod --reference='$2' '$2.unfr0zen-new' 2>/dev/null || chmod $4 '$2.unfr0zen-new'"
}
# swap DEST: keep DEST in /tmp, rename the staged file over it.
swap() {
    name=$(basename -- "$1")
    on "cp -p '$1' '/tmp/$name.unfr0zen-old' && mv -f '$1.unfr0zen-new' '$1' && sync"
}
restore() { on "mv -f '/tmp/$(basename -- "$1").unfr0zen-old' '$1' && sync"; }
pid_of() { on "pidof $1" || true; }
# stays_up PROCESS SECONDS [OLD_PID]: a pid other than OLD_PID, the same one
# alive for the last 10 of SECONDS: a process that never restarted is no proof.
stays_up() {
    on "end=\$((\$(date +%s) + $2)); last=; steady=0
        while [ \$(date +%s) -lt \$end ]; do
            pid=\$(pidof $1) || pid=
            [ \"\$pid\" != '${3:-}' ] || pid=
            if [ -n \"\$pid\" ] && [ \"\$pid\" = \"\$last\" ]; then steady=\$((steady + 1)); else steady=0; fi
            last=\$pid
            [ \$steady -ge 10 ] && exit 0
            sleep 1
        done
        exit 1"
}
stop_nlclient() {  # as the Doom launcher does
    on 'monit unmonitor nlclient > /dev/null 2>&1
        for i in 1 2 3 4 5 6 7 8 9 10; do
            pid=$(pidof nlclient) || break
            if [ "$i" -lt 6 ]; then kill $pid; else kill -9 $pid; fi
            sleep 1
        done
        ! pidof nlclient > /dev/null'
}
start_nlclient() { on 'monit monitor nlclient > /dev/null 2>&1; monit start nlclient > /dev/null 2>&1'; }

if [ $dry = 1 ]; then
    say "Dry run: built and checked; nothing on the unit was changed"
    exit 0
fi

if [ $nestlocal = 1 ]; then
    # The init script rolls back an update nestlocal installed (update.rs);
    # the app installs it only when it patches the unit.
    init=crates/nestlocal/rootfs/nestlocal.init
    init_hash=$(hash "$init")
    if [ "$(remote_hash /etc/init.d/nestlocal)" != "$init_hash" ]; then
        say "nestlocal.init: copying"
        stage "$init" /etc/init.d/nestlocal "$init_hash" 755
        on "mv -f /etc/init.d/nestlocal.unfr0zen-new /etc/init.d/nestlocal && sync"
    fi
    if [ "$(remote_hash /nestlabs/sbin/nestlocal)" = "$nestlocal_hash" ]; then
        say "nestlocal: the unit already runs this build"
    else
        say "nestlocal: copying"
        stage build/nestlocal-armv7 /nestlabs/sbin/nestlocal "$nestlocal_hash" 755
        swap /nestlabs/sbin/nestlocal
        # This build replaces any update still pending; it is not rolled back.
        on "rm -f /etc/nestlocal/update.pending && sync"
        say "nestlocal: restarting"
        old=$(pid_of nestlocal)
        on 'monit restart nestlocal'
        if stays_up nestlocal 40 "$old"; then
            say "nestlocal: running the new build"
        else
            say "nestlocal: did not stay up; putting the previous build back"
            restore /nestlabs/sbin/nestlocal
            on 'monit restart nestlocal'
            stays_up nestlocal 40 || say "nestlocal: the previous build did not stay up either; check the unit"
            fail "nestlocal did not start; the unit's log: ssh $host logread | grep nestlocal"
        fi
    fi
fi

if [ $screen = 1 ] && [ "$(remote_hash /nestlabs/sbin/nlclient)" != "$nlclient_hash" ]; then
    say "nlclient: copying it and its undo journal"
    stage "$work/nlclient" /nestlabs/sbin/nlclient "$nlclient_hash" 755
    stage "$work/nlclient.undo.gz" /etc/unfr0zen/nlclient.undo.gz "$journal_hash" 644
    say "nlclient: stopping it, as the Doom launcher does (the heating holds its state)"
    if ! stop_nlclient; then
        start_nlclient
        on "rm -f /nestlabs/sbin/nlclient.unfr0zen-new /etc/unfr0zen/nlclient.undo.gz.unfr0zen-new"
        fail "nlclient would not stop; nothing was swapped"
    fi
    swap /nestlabs/sbin/nlclient
    swap /etc/unfr0zen/nlclient.undo.gz
    # A fault in the screen's factory is remembered until reboot; a new build starts afresh.
    on "rm -f /var/run/unfr0zen-screen.attempt; printf '%s\n' 'unfr0zen $(git describe --always --dirty 2>/dev/null || echo dev) (ssh)' > /etc/unfr0zen/version.unfr0zen-new && mv -f /etc/unfr0zen/version.unfr0zen-new /etc/unfr0zen/version && sync"
    start_nlclient
    if stays_up nlclient 60; then
        say "nlclient: running the new build"
    else
        say "nlclient: did not stay up; putting the previous build back"
        stop_nlclient || true
        restore /nestlabs/sbin/nlclient
        restore /etc/unfr0zen/nlclient.undo.gz
        start_nlclient
        stays_up nlclient 60 || say "nlclient: the previous build did not stay up either; recover over USB with the app"
        fail "the new nlclient did not stay up"
    fi
fi
say "Done. Log: $log"
