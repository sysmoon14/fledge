//! Motion and near presence, from the sensors on the backplate: nlclient
//! keeps them to itself but logs each event in /media/log/nlclient, and
//! nestlocal follows that log.
//!
//! - "publishing pir msg true": the PIR, which sees motion across the room
//!   (what lights Farsight). nlclient only looks while Farsight is on.
//! - "publishing tp - maxl: .. maxud: N": the thermopile's approach reading,
//!   about once a second while someone is close, N growing as they come
//!   nearer (nlclient's wake on approach). Not "tp crossings": those come
//!   with any movement, near or far.
//! - "pir N; px1 ..; alv A, alir I": a reading every few seconds while
//!   awake, with the ambient light sensor's visible (A) and infrared (I).
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
    /// The near-field proximity sensor: someone in front of the unit.
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
            Sensor::Near => "Near presence",
        }
    }

    pub fn device_class(self) -> &'static str {
        match self {
            Sensor::Motion => "motion",
            Sensor::Near => "occupancy",
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
}

/// The sensor event a line of nlclient's log reports, if any.
#[cfg(test)]
fn event(line: &str) -> Option<Sensor> {
    match reading(line)? {
        Event::Sensed(sensor) => Some(sensor),
        Event::Approach(_) => Some(Sensor::Near),
        Event::Light { .. } => None,
    }
}

/// The number after `key` in `line`.
fn number(line: &str, key: &str) -> Option<u32> {
    let rest = &line[line.find(key)? + key.len()..];
    let digits: String = rest.trim_start().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
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
    } else {
        None
    }
}

/// Whether a line of nlclient's log says it shows one of its alerts.
pub fn nest_alert(line: &str) -> bool {
    line.contains("alert manager: showing alert:")
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
    bridge.has_sensors();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            let mut seen = Vec::new();
            let lines = log.lines();
            if lines.iter().any(|l| nest_alert(l)) {
                logf!("motion: the Nest shows an alert");
                crate::ui::nest_alert();
            }
            let mut light = None;
            for event in lines.iter().filter_map(|l| reading(l)) {
                match event {
                    Event::Light { visible, infrared } => light = Some((visible, infrared)),
                    Event::Approach(level) => logf!("motion: approach {level}"),
                    Event::Sensed(_) => {}
                }
                let sensor = match event {
                    Event::Sensed(sensor) => sensor,
                    Event::Approach(_) => Sensor::Near,
                    Event::Light { .. } => continue,
                };
                if !seen.contains(&sensor) {
                    seen.push(sensor);
                }
            }
            for sensor in seen {
                bridge.sensor_event(sensor);
                crate::ui::sensed(sensor);
            }
            if let Some((visible, infrared)) = light {
                bridge.light(visible, infrared);
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
    fn the_pir_is_motion_and_the_approach_reading_near_presence() {
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
    fn a_nest_alert_showing_is_seen_and_one_queued_isnt() {
        assert!(nest_alert("2026-11-09 12:00:00.000000 09AB01AC36230301 nlclient[500:main] (2) alert manager: showing alert: heatlinkbattery"));
        assert!(!nest_alert("alert manager: queuing alert type:heatlinkbattery"));
        assert!(!nest_alert(PIR));
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
