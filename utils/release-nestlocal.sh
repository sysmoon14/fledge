#!/bin/sh
# Publishes this commit's nestlocal as a signed GitHub release of the fork
# `origin` points at, for the update entity (crates/nestlocal/src/update.rs)
# to offer:
#
#     utils/release-nestlocal.sh [--tag TAG] [--notes FILE] [--key FILE] [--dry-run]
#     utils/release-nestlocal.sh --init-key [--key FILE]
#
#   --init-key  make the signing key (Ed25519, kept on this machine only) and
#               print its public key, the value for nestlocal's -update-key
#   --key FILE  the signing key (default ~/.config/unfr0zen/release-signing.pem)
#   --tag TAG   the release's tag (default: the latest vX.Y.Z tag, its patch
#               number one more: v0.4.0, then v0.4.1, ...)
#   --notes FILE  what's new, in Markdown, to open the release notes; its
#               first paragraph (at most 255 characters) is the summary Home
#               Assistant's update entity shows. The changes since the last
#               release follow it.
#               A tag vX.Y.Z-rc.N (or -beta.N, -alpha.N) is published as a
#               GitHub pre-release, not marked latest: only units with Home
#               Assistant's Fledgeling pre-releases switch on are offered it.
#   --dry-run   build, check and sign, then publish nothing (the tag is removed)
#
# The commit must be clean and pushed. It is tagged first, so nestlocal reports
# the tag as its version (git describe), and built in the firmware build image
# on macOS, as utils/build-firmware.sh does (it needs one full build first).
# The release holds nestlocal-armv7, nestlocal-armv7.sig (a raw 64-byte
# Ed25519 signature of the file) and nestlocal-armv7.sha256. Publishing pushes
# the tag to origin and creates the release with gh as GitHub's active
# account, marked latest. Needs OpenSSL 3 (brew install openssl@3).
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
key=$HOME/.config/unfr0zen/release-signing.pem
tag= dry=0 init=0 notes=
while [ $# -gt 0 ]; do
    case $1 in
        --tag) [ $# -gt 1 ] || { echo "--tag needs a tag" >&2; exit 2; }; tag=$2; shift ;;
        --key) [ $# -gt 1 ] || { echo "--key needs a file" >&2; exit 2; }; key=$2; shift ;;
        --notes) [ $# -gt 1 ] || { echo "--notes needs a file" >&2; exit 2; }; notes=$2; shift ;;
        --dry-run) dry=1 ;;
        --init-key) init=1 ;;
        -h|--help) sed -n '2,/^set -eu/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown argument $1" >&2; exit 2 ;;
    esac
    shift
done
fail() { echo "FAILED: $*" >&2; exit 1; }

# An OpenSSL that signs with Ed25519 (macOS's own LibreSSL does not).
openssl=
for candidate in "${OPENSSL:-}" openssl /opt/homebrew/opt/openssl@3/bin/openssl /usr/local/opt/openssl@3/bin/openssl; do
    [ -n "$candidate" ] && command -v "$candidate" > /dev/null || continue
    if "$candidate" list -public-key-algorithms 2>/dev/null | grep -qi ed25519; then openssl=$candidate; break; fi
done
[ -n "$openssl" ] || fail "no OpenSSL with Ed25519: brew install openssl@3"
public_key() { "$openssl" pkey -in "$key" -pubout -outform DER | tail -c 32 | base64; }

if [ $init = 1 ]; then
    [ ! -e "$key" ] || fail "$key exists; nestlocal trusts its public key: $(public_key)"
    mkdir -p "$(dirname -- "$key")"
    (umask 077 && "$openssl" genpkey -algorithm ed25519 -out "$key")
    echo "Made $key. Keep it private and backed up: a unit trusts only releases it signs."
    echo "Public key, for nestlocal's -update-key: $(public_key)"
    exit 0
fi
[ -f "$key" ] || fail "no signing key $key: make one with --init-key"
[ -z "$notes" ] || [ -f "$notes" ] || fail "no notes file $notes"

# --- the commit ----------------------------------------------------------------
git update-index -q --refresh
git diff-index --quiet HEAD -- || fail "the tree has uncommitted changes"
branch=$(git symbolic-ref --short HEAD) || fail "not on a branch"
git fetch -q origin "$branch" || fail "cannot fetch origin $branch"
[ "$(git rev-parse HEAD)" = "$(git rev-parse FETCH_HEAD)" ] || fail "HEAD is not origin/$branch: push it first"
repo=$(git remote get-url origin | sed -E 's#^(https://github.com/|git@github.com:)##; s#\.git$##')
case $repo in */*) ;; *) fail "origin is not a GitHub repository: $(git remote get-url origin)" ;; esac
for existing in $(git tag --points-at HEAD); do
    case $existing in v[0-9]*) fail "this commit is already released as $existing" ;; esac
done
previous=$(git describe --tags --abbrev=0 2>/dev/null) || fail "no release tag below this commit"
if [ -z "$tag" ]; then
    latest=$(git tag -l 'v*' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -n 1)
    [ -n "$latest" ] || fail "no vX.Y.Z tag to follow: give --tag"
    tag=$(echo "$latest" | awk -F. '{ printf "%s.%s.%d", $1, $2, $3 + 1 }')
fi
git rev-parse -q --verify "refs/tags/$tag" > /dev/null && fail "tag $tag exists"
git ls-remote --exit-code --tags origin "refs/tags/$tag" > /dev/null 2>&1 && fail "origin has tag $tag"

out=.work/release/$tag
rm -rf "$out"
mkdir -p "$out"
log=$out/build.log
echo "Releasing $(git rev-parse --short HEAD) on $branch as $tag ($repo), since $previous"

git tag -a "$tag" -m "Fledge $tag"
published=0
untag() { [ $published = 1 ] || git tag -d "$tag" > /dev/null 2>&1 || true; }
trap untag EXIT

# --- build -----------------------------------------------------------------------
echo "Building nestlocal (log: $log)"
if [ "$(uname -s)" = Darwin ]; then
    docker build -q -t unfr0zen-build - < utils/ci/Dockerfile >> "$log" 2>&1 || { tail -n 20 "$log" >&2; fail "docker build"; }
    docker run --rm -v "$root:$root" -w "$root" --user "$(id -u):$(id -g)" -e HOME=/tmp \
        -e CARGO_HOME="$root/.work/cargo" -e UNFR0ZEN_BUILD_VERSION="$tag" unfr0zen-build \
        sh platform/common/radio/thread/nestlocal.sh >> "$log" 2>&1 || { tail -n 20 "$log" >&2; fail "nestlocal build"; }
else
    UNFR0ZEN_BUILD_VERSION=$tag sh platform/common/radio/thread/nestlocal.sh >> "$log" 2>&1 ||
        { tail -n 20 "$log" >&2; fail "nestlocal build"; }
fi
binary=$out/nestlocal-armv7
cp build/nestlocal-armv7 "$binary"
grep -a -q -- "nestlocal/$tag" "$binary" || grep -a -q -- "$tag" "$binary" || fail "the build does not report $tag"
! grep -a -q -- "$tag-dirty" "$binary" || fail "the build reports $tag-dirty"

# --- sign --------------------------------------------------------------------------
"$openssl" pkeyutl -sign -rawin -inkey "$key" -in "$binary" -out "$binary.sig"
[ "$(wc -c < "$binary.sig" | tr -d ' ')" = 64 ] || fail "the signature is not 64 bytes"
"$openssl" pkey -in "$key" -pubout -out "$out/public.pem"
"$openssl" pkeyutl -verify -rawin -pubin -inkey "$out/public.pem" -in "$binary" -sigfile "$binary.sig" > /dev/null ||
    fail "the signature does not verify"
rm -f "$out/public.pem"
if command -v sha256sum > /dev/null; then (cd "$out" && sha256sum nestlocal-armv7 > nestlocal-armv7.sha256)
else (cd "$out" && shasum -a 256 nestlocal-armv7 > nestlocal-armv7.sha256); fi
{
    if [ -n "$notes" ]; then
        cat "$notes"
    else
        echo "Fledge $tag: nestlocal for the thermostat. The Home Assistant integration is released apart, from sysmoon14/fledge-homeassistant."
    fi
    echo
    echo "## Changes since $previous"
    echo
    git log --no-merges --format='- %s' "$previous..$tag"
    echo
    echo "Signed with Ed25519 public key \`$(public_key)\`."
} > "$out/notes.md"
echo "nestlocal: $(wc -c < "$binary" | tr -d ' ') bytes, $(cut -d' ' -f1 "$binary.sha256"), signed"

if [ $dry = 1 ]; then
    echo "Dry run: $out holds the release; nothing was published and the tag is removed"
    echo "(build/nestlocal-armv7 now reports $tag; the next build reports this checkout again)"
    exit 0
fi

# --- publish ------------------------------------------------------------------------
git push -q origin "refs/tags/$tag" || fail "cannot push tag $tag to origin"
published=1
case $tag in
    *-rc.*|*-beta.*|*-alpha.*) channel=--prerelease ;;
    *) channel=--latest ;;
esac
gh release create "$tag" --repo "$repo" --verify-tag "$channel" --title "Fledge $tag" \
    --notes-file "$out/notes.md" "$binary" "$binary.sig" "$binary.sha256" ||
    fail "the tag is pushed but the release was not created: gh release create $tag --repo $repo ..."
echo "Released: https://github.com/$repo/releases/tag/$tag"
