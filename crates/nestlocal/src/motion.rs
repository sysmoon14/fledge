//! Motion and someone close, from the sensors on the backplate: nlclient
//! keeps them to itself but logs each event in /media/log/nlclient, and
//! nestlocal follows that log. This is the one place nestlocal learns of
//! people about: each event also keeps Occupancy on (occupancy.rs).
//!
//! - "publishing pir msg true": the PIR, which sees motion across the room
//!   (what lights Farsight). nlclient only looks while Farsight is on.
//! - "publishing tp - maxl: .. maxud: N": the thermopile's approach reading,
//!   about once a second while someone is close, N growing as they come
//!   nearer (nlclient's wake on approach). Not "tp crossings": those come
//!   with any movement, near or far.
//! - "pir N; px1 ..; alv A, alir I": a reading every few seconds while
//!   awake, with the ambient light sensor's visible (A) and infrared (I).
//! - "msg: *HVAC power [init]  vusb U vin I vout O" and "state .., vi IV, vo
//!   OV; ..": the backplate's power, on each wake and every ~30 s, and an
//!   "init" one at once when power comes back to its terminals. vin is in
//!   10 mV, vout in mV. Measured on Ben's unit with an 11 V supply on the
//!   terminals: on, vi 10.97 V and vo 4.64 V; off, vi 0.24-0.33 V and vo
//!   0.001-0.002 V. vo is the backplate powering the display: all or
//!   nothing, so it says whether the unit has external power.
//!
//! - nlclient's event log (occupancy::EVENT_LOG) "RealTimeOccupancy" lines:
//!   motion too, a line per 30-second slot with some, so possibly late:
//!   with Farsight off nlclient writes them, and asleep it writes them only
//!   at its next wake. One under Motion's hold old counts as Motion; older
//!   ones only keep Occupancy on. Neither wakes the screen: the PIR line
//!   above does that on time.
//!
//! Each event is published as ON; Home Assistant clears the sensor itself
//! (off_delay) after its hold without another, so a sensor clears on time
//! even while the unit sleeps. The unit sleeps between wakes and nestlocal
//! with it: a sensor event wakes it, and the log is read every POLL while
//! awake. nlclient rotates the log by replacing the file, which a changed
//! inode or a shorter file shows.

use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::bridge::Bridge;
use crate::logf;

const LOG: &str = "/media/log/nlclient";

/// How often the log is read while the unit is awake.
const POLL: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sensor {
    /// The PIR: motion anywhere in view.
    Motion,
    /// The thermopile's approach reading: someone close to the unit.
    Near,
}

impl Sensor {
    pub const ALL: [Sensor; 2] = [Sensor::Motion, Sensor::Near];

    pub fn key(self) -> &'static str {
        match self {
            Sensor::Motion => "motion",
            Sensor::Near => "near",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sensor::Motion => "Motion",
            Sensor::Near => "Someone close",
        }
    }

    pub fn device_class(self) -> &'static str {
        match self {
            Sensor::Motion => "motion",
            Sensor::Near => "presence",
        }
    }

    /// Seconds a sensor stays on after its last event. The PIR reports
    /// motion every few seconds while someone moves about; the approach
    /// reading every second while someone is close.
    pub fn hold(self) -> u64 {
        match self {
            Sensor::Motion => 30,
            Sensor::Near => 10,
        }
    }
}

/// What a line of nlclient's log reports, if anything.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Sensed(Sensor),
    /// The approach reading: how strongly the thermopile sees someone.
    Approach(u32),
    /// The ambient light sensor: visible and infrared.
    Light { visible: u32, infrared: u32 },
    /// The backplate's power: the voltage on its terminals and what it
    /// supplies the display, in volts.
    Power { input: f32, output: f32 },
}

/// The display's supply from the backplate above which the unit has
/// external power: it reads 4.6 V with power on the terminals, 0.002 V
/// without.
pub const EXTERNAL_V: f32 = 1.0;

/// The sensor event a line of nlclient's log reports, if any.
#[cfg(test)]
fn event(line: &str) -> Option<Sensor> {
    match reading(line)? {
        Event::Sensed(sensor) => Some(sensor),
        Event::Approach(_) => Some(Sensor::Near),
        Event::Light { .. } | Event::Power { .. } => None,
    }
}

/// The number after `key` in `line`.
fn number(line: &str, key: &str) -> Option<u32> {
    let rest = &line[line.find(key)? + key.len()..];
    let digits: String = rest.trim_start().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The volts after `key` in `line`, written like "11.09V".
fn volts(line: &str, key: &str) -> Option<f32> {
    let rest = &line[line.find(key)? + key.len()..];
    let n: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    rest.trim_start()[n.len()..].starts_with('V').then_some(())?;
    n.parse().ok()
}

/// What a line of nlclient's log reports.
pub fn reading(line: &str) -> Option<Event> {
    let (_, said) = line.split_once("backplate controller: ")?;
    if said.starts_with("publishing pir msg true") {
        Some(Event::Sensed(Sensor::Motion))
    } else if said.starts_with("publishing tp - ") {
        Some(Event::Approach(number(said, "maxud:")?))
    } else if said.starts_with("pir ") && said.contains("alv ") {
        Some(Event::Light { visible: number(said, "alv ")?, infrared: number(said, "alir ")? })
    } else if said.starts_with("msg: *HVAC power") {
        Some(Event::Power { input: number(said, " vin ")? as f32 / 100.0, output: number(said, " vout ")? as f32 / 1000.0 })
    } else if said.starts_with("state ") {
        Some(Event::Power { input: volts(said, " vi ")?, output: volts(said, " vo ")? })
    } else {
        None
    }
}

/// Whether a line of nlclient's log says it shows one of its alerts.
pub fn nest_alert(line: &str) -> bool {
    line.contains("alert manager: showing alert:")
}

/// Whether an event log line's motion, sensed at `at`, is recent enough to
/// be Motion now: within Motion's own hold. Older ones, written at a wake
/// long after (Farsight off, the unit asleep), only keep Occupancy on.
fn fresh(at: i64, now: i64) -> bool {
    now - at <= Sensor::Motion.hold() as i64
}

/// Reads what was appended to a log since the last read, whole lines only,
/// starting from wherever the log ended when it was first opened.
struct Follower {
    path: PathBuf,
    inode: u64,
    pos: u64,
    partial: String,
}

impl Follower {
    fn open(path: PathBuf) -> Option<Follower> {
        let m = std::fs::metadata(&path).ok()?;
        Some(Follower { path, inode: m.ino(), pos: m.len(), partial: String::new() })
    }

    fn lines(&mut self) -> Vec<String> {
        let Ok(m) = std::fs::metadata(&self.path) else { return Vec::new() };
        if m.ino() != self.inode || m.len() < self.pos {
            // Rotated: the new file is read from its start.
            self.inode = m.ino();
            self.pos = 0;
            self.partial.clear();
        }
        if m.len() == self.pos {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        let read = std::fs::File::open(&self.path).and_then(|mut f| {
            f.seek(SeekFrom::Start(self.pos))?;
            f.take(m.len() - self.pos).read_to_end(&mut bytes)
        });
        let Ok(n) = read else { return Vec::new() };
        self.pos += n as u64;
        self.partial.push_str(&String::from_utf8_lossy(&bytes));
        let complete = self.partial.rfind('\n').map_or(0, |i| i + 1);
        let lines = self.partial[..complete].lines().map(str::to_string).collect();
        self.partial.drain(..complete);
        lines
    }
}

/// Reports the unit's sensor events to the bridge for as long as nestlocal
/// runs, if nlclient's log is there to follow.
pub fn start(bridge: Arc<Bridge>) {
    let Some(mut log) = Follower::open(LOG.into()) else {
        logf!("motion: no {LOG} to follow");
        return;
    };
    logf!("motion: following {LOG}");
    // Opened as it appears: nlclient makes it on its first event.
    let mut events: Option<Follower> = None;
    bridge.has_sensors();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            bridge.occupancy.update(chrono::Utc::now().timestamp());
            let events = match &mut events {
                Some(follower) => Some(follower),
                None => {
                    events = Follower::open(crate::occupancy::EVENT_LOG.into());
                    events.as_mut()
                }
            };
            let late = events.map(|f| f.lines()).unwrap_or_default().iter()
                .filter_map(|l| crate::occupancy::detection(l)).max();
            let mut seen = Vec::new();
            let lines = log.lines();
            if lines.iter().any(|l| nest_alert(l)) {
                logf!("motion: the Nest shows an alert");
                crate::ui::nest_alert();
            }
            let mut light = None;
            let mut power = None;
            for event in lines.iter().filter_map(|l| reading(l)) {
                match event {
                    Event::Light { visible, infrared } => light = Some((visible, infrared)),
                    Event::Power { input, output } => power = Some((input, output)),
                    Event::Approach(level) => logf!("motion: approach {level}"),
                    Event::Sensed(_) => {}
                }
                let sensor = match event {
                    Event::Sensed(sensor) => sensor,
                    Event::Approach(_) => Sensor::Near,
                    Event::Light { .. } | Event::Power { .. } => continue,
                };
                if !seen.contains(&sensor) {
                    seen.push(sensor);
                }
            }
            let now = chrono::Utc::now().timestamp();
            for &sensor in &seen {
                bridge.sensor_event(sensor, now);
                crate::ui::sensed(sensor);
            }
            if let Some(at) = late.filter(|_| !seen.contains(&Sensor::Motion)) {
                if fresh(at, now) {
                    bridge.sensor_event(Sensor::Motion, at);
                } else {
                    // Written at a wake long after: someone was about, not now.
                    bridge.occupancy.sensed(at);
                }
            }
            if let Some((visible, infrared)) = light {
                bridge.light(visible, infrared);
            }
            if let Some((input, output)) = power {
                bridge.power(input, output);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIR: &str = "2026-11-06 23:10:38.134457 09AB01AC36230301 nlclient[500:bckpla] (2) backplate controller: publishing pir msg true (0x0001 0x7c00), threshold 100";
    const TP: &str = "2026-11-06 23:12:07.146671 09AB01AC36230301 nlclient[500:bckpla] (2) backplate controller: tp crossings - 22";
    const APPROACH: &str = "2026-11-09 13:21:28.998656 09AB01AC36230301 nlclient[16444:bckpla] (2) backplate controller: publishing tp - maxl: 377 minl: 0 maxr: 377 minr: 0 maxud: 435 minud: 0 lrmax: 100";
    const READING: &str = "2026-11-06 23:12:08.476949 09AB01AC36230301 nlclient[500:bckpla] (2) backplate controller: pir 7954; px1 65389/0, px2 65389/0; alv 0, alir 0";

    #[test]
    fn the_pir_is_motion_and_the_approach_reading_someone_close() {
        assert_eq!(event(PIR), Some(Sensor::Motion));
        assert_eq!(event(APPROACH), Some(Sensor::Near));
        assert_eq!(reading(APPROACH), Some(Event::Approach(435)));
        assert_eq!(event(TP), None, "crossings come with any movement, near or far");
        assert_eq!(event(READING), None, "a raw reading is not an event");
        assert_eq!(reading(READING), Some(Event::Light { visible: 0, infrared: 0 }));
        assert_eq!(reading("2026-11-09 13:25:07.694777 S nlclient[16444:bckpla] (2) backplate controller: pir 8209; px1 65240/15, px2 65240/15; alv 12, alir 19"),
                   Some(Event::Light { visible: 12, infrared: 19 }));
        assert_eq!(event("czcomms: promoting hot water schedule"), None);
    }

    #[test]
    fn the_backplate_power_from_both_of_its_lines() {
        let on = reading("2026-11-10 11:56:09.893029 09AB01AC36230301 nlclient[31184:bckpla] (2) backplate controller: msg: *HVAC power  vusb 10 vin 1097 vout 4647");
        assert_eq!(on, Some(Event::Power { input: 10.97, output: 4.647 }));
        let back = reading("2026-11-10 12:02:39.861540 09AB01AC36230301 nlclient[31184:bckpla] (2) backplate controller: msg: *HVAC power init  vusb 5 vin 1095 vout 4642");
        assert_eq!(back, Some(Event::Power { input: 10.95, output: 4.642 }));
        let off = reading("2026-11-10 11:58:51.207836 09AB01AC36230301 nlclient[31184:bckpla] (2) backplate controller: msg: *HVAC power  vusb 14 vin 33 vout 1");
        assert_eq!(off, Some(Event::Power { input: 0.33, output: 0.001 }));
        let state = reading("2026-11-10 11:58:24.600037 09AB01AC36230301 nlclient[31184:bckpla] (2) backplate controller: state 1, flags 14, vi 11.09V, vo 4.636V; px0 0a, vb 3.92V; sw 00/00, p2 00; voc 0.17V; pins 15, wires 15");
        assert_eq!(state, Some(Event::Power { input: 11.09, output: 4.636 }));
        let state_off = reading("2026-11-10 11:58:54.685756 09AB01AC36230301 nlclient[31184:bckpla] (2) backplate controller: state 1, flags 14, vi 0.24V, vo 0.002V; px0 0a, vb 3.848V; sw 00/00, p2 00; voc 0.13V; pins 15, wires 15");
        assert_eq!(state_off, Some(Event::Power { input: 0.24, output: 0.002 }));
        for (event, external) in [(on, true), (back, true), (state, true), (off, false), (state_off, false)] {
            let Some(Event::Power { output, .. }) = event else { unreachable!() };
            assert_eq!(output >= EXTERNAL_V, external);
        }
        assert_eq!(reading("backplate controller: got power message detect: 1 wire: 2 current: 30mA"), None);
    }

    #[test]
    fn a_nest_alert_showing_is_seen_and_one_queued_isnt() {
        assert!(nest_alert("2026-11-09 12:00:00.000000 09AB01AC36230301 nlclient[500:main] (2) alert manager: showing alert: heatlinkbattery"));
        assert!(!nest_alert("alert manager: queuing alert type:heatlinkbattery"));
        assert!(!nest_alert(PIR));
    }

    #[test]
    fn only_a_recent_event_log_line_is_motion_now() {
        assert!(fresh(1000, 1000) && fresh(1000, 1030));
        assert!(!fresh(1000, 1031), "75 s late, as the unit woke: Occupancy only");
    }

    #[test]
    fn the_follower_reads_new_whole_lines_and_restarts_on_rotation() {
        let dir = std::env::temp_dir().join(format!("nestlocal-motion-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nlclient");
        std::fs::write(&path, "old line\n").unwrap();
        let mut f = Follower::open(path.clone()).unwrap();
        assert!(f.lines().is_empty(), "what was there before is skipped");

        let append = |s: &str| {
            use std::io::Write;
            std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(s.as_bytes()).unwrap();
        };
        append(&format!("{PIR}\nhalf a li"));
        assert_eq!(f.lines(), vec![PIR.to_string()]);
        append("ne\n");
        assert_eq!(f.lines(), vec!["half a line".to_string()]);

        let rotated = dir.join("nlclient.new");
        std::fs::write(&rotated, format!("{TP}\n")).unwrap();
        std::fs::rename(&rotated, &path).unwrap();
        assert_eq!(f.lines(), vec![TP.to_string()]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
