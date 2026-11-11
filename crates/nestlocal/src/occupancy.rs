//! Occupancy: someone has been about in the last HOLD. motion.rs reads the
//! unit's sensors and reports each Motion or Someone close event here
//! (Bridge::sensor_event), so the Occupancy entity, Matter's and Zigbee's
//! occupancy all follow the same events as the Motion and Someone close
//! sensors, held longer.
//!
//! nlclient's event log also notes motion, a line for each 30-second slot
//! with some, when logged then when detected (UTC):
//!
//!   2026-11-10T00:40:48 RealTimeOccupancy 2026-11-10T00:40:45
//!
//! motion.rs follows that log too and takes these lines as Motion.

use std::sync::Mutex;

/// The event log nlclient writes, rotated by renaming: a new file takes the name.
pub const EVENT_LOG: &str = "/nestlabs/var/log/nlevent";
/// How long an event keeps the unit occupied, seconds.
pub const HOLD: i64 = 15 * 60;

type Watcher = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct Occupancy {
    /// The last event, seconds since 1970.
    last: Mutex<Option<i64>>,
    /// What the watchers were last told.
    shown: Mutex<bool>,
    watchers: Mutex<Vec<Watcher>>,
}

impl Occupancy {
    /// Whether the last event is under HOLD old.
    pub fn occupied(&self) -> bool {
        self.occupied_at(chrono::Utc::now().timestamp())
    }

    fn occupied_at(&self, now: i64) -> bool {
        self.last.lock().unwrap().is_some_and(|t| now - t < HOLD)
    }

    /// Calls f whenever occupied changes.
    pub fn on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        self.watchers.lock().unwrap().push(Box::new(f));
    }

    /// An event at t, seconds since 1970: a later one than the last moves
    /// the hold on.
    pub fn sensed(&self, t: i64) {
        {
            let mut last = self.last.lock().unwrap();
            if last.is_none_or(|l| t > l) {
                *last = Some(t);
            }
        }
        self.update(chrono::Utc::now().timestamp());
    }

    /// Tells the watchers when occupied has changed, as of now: motion.rs
    /// calls it as it polls, so the hold runs out on time while awake.
    pub fn update(&self, now: i64) {
        let occupied = self.occupied_at(now);
        let changed = std::mem::replace(&mut *self.shown.lock().unwrap(), occupied) != occupied;
        if changed {
            for w in self.watchers.lock().unwrap().iter() {
                w();
            }
        }
    }
}

/// When a RealTimeOccupancy line's motion was detected.
pub fn detection(line: &str) -> Option<i64> {
    let mut words = line.split_whitespace();
    let _logged = words.next()?;
    if words.next()? != "RealTimeOccupancy" {
        return None;
    }
    let t = chrono::NaiveDateTime::parse_from_str(words.next()?, "%Y-%m-%dT%H:%M:%S").ok()?;
    Some(t.and_utc().timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn detections_are_read_from_realtimeoccupancy_lines_only() {
        assert_eq!(detection("2026-11-10T00:40:48 RealTimeOccupancy 2026-11-10T00:40:45\n"), Some(1_794_271_245));
        assert_eq!(detection("2026-11-10T00:40:48 %RealTimeOccupancy { \"fields\": [] }"), None);
        assert_eq!(detection("2026-11-10T00:40:00 BufferedPassiveInfrared 2026-11-10T00:40:00 100 19"), None);
        assert_eq!(detection("2026-11-10T00:40:48 RealTimeOccupancy garbage"), None);
        assert_eq!(detection(""), None);
    }

    #[test]
    fn occupied_until_the_hold_after_the_last_event() {
        let o = Occupancy::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        o.on_change(move || { c.fetch_add(1, Ordering::SeqCst); });
        let now = chrono::Utc::now().timestamp();
        assert!(!o.occupied_at(now));
        o.sensed(now);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "told at once");
        assert!(o.occupied_at(now + HOLD - 1) && !o.occupied_at(now + HOLD));
        // An older event, as a late log line gives, changes nothing.
        o.sensed(now - 600);
        o.update(now + HOLD - 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        o.update(now + HOLD);
        assert_eq!(calls.load(Ordering::SeqCst), 2, "the hold ran out");
        o.update(now + HOLD + 60);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
