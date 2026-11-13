//! Heating control by Home Assistant: "Call for heat".
//!
//! Off (the default), the Nest runs its own schedule as it always has. On,
//! something in Home Assistant (radiator valves, a zoning integration, an
//! automation) decides when the house needs heat, and the Nest only fires
//! the boiler:
//!
//!   set/heat_control   ON/OFF: who decides when to heat, kept across
//!                      restarts
//!   set/call_for_heat  {"heat": true, "lease": 600, "rooms": ["Office"]},
//!                      or ON/OFF with the default lease. The Fledge
//!                      integration sends it again every minute while heat
//!                      is wanted, and while it isn't.
//!
//! An ask holds for its lease. While one holds, the Nest's schedule is moved
//! aside into this module's file, with learning (or it would learn a
//! schedule nobody chose) and its mode, and the Nest is put in heat at HEAT
//! to heat, or off not to; the unit's own safety temperature (which holds
//! while it is off) and cycle limits still apply.
//!
//! When the lease runs out (Home Assistant, the broker or the network gone)
//! the schedule and learning go back and the Nest runs on its own until heat
//! is asked for again: the safety net is the schedule it already had. With
//! no schedule to go back to, it holds FALLBACK.
//!
//! The dial stays the owner's: turned while an ask holds (or the target or
//! mode set from the climate entity), the Nest heats to it itself until the moved
//! aside schedule's next change, or OVERRIDE with none, then follows Home
//! Assistant again. A schedule written while the schedule is aside
//! (set/heating_times, clear_schedule) changes the one aside.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fields;
use crate::json::{number, sprint, truthy, Fields};

/// Where it lives on the unit.
pub const FILE: &str = "/media/data/fledge-heat-control";
/// The target while heat is asked for: above any room, so the boiler runs.
pub const HEAT: f64 = 30.0;
/// The target when the lease runs out with no schedule to go back to.
pub const FALLBACK: f64 = 17.0;
/// A lease, seconds: the default and the range an ask may name.
pub const LEASE: i64 = 600;
const LEASE_RANGE: (i64, i64) = (60, 3600);
/// How long a turn of the dial holds with no schedule change ahead.
const OVERRIDE: i64 = 2 * 3600;
/// How far the target may drift from what was written before it is a turn
/// of the dial, °C.
const TOLERANCE: f64 = 0.25;

/// A bucket change: its type ("schedule", "device", "shared") and fields.
pub type Change = (String, Fields);

/// What the file keeps.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Kept {
    enabled: bool,
    /// The schedule and learning moved aside while an ask holds.
    #[serde(default)]
    aside: Option<Aside>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Aside {
    days: Value,
    learning: bool,
    /// The mode it was in (target_temperature_type), to put back.
    #[serde(default = "heat_mode")]
    mode: String,
}

fn heat_mode() -> String {
    "heat".into()
}

#[derive(Default)]
struct Inner {
    kept: Kept,
    /// When the last ask's lease runs out, seconds since 1970.
    lease_until: Option<i64>,
    heat: bool,
    rooms: Vec<String>,
    /// A turn of the dial holds until then.
    override_until: Option<i64>,
    /// What was last written: heat at HEAT, or off.
    written: Option<bool>,
}

/// Where heating control stands, as Home Assistant and the screen show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The Nest decides: heating control is off.
    Nest,
    /// On, but nothing asks: the Nest runs its own schedule.
    OnItsOwn,
    /// Home Assistant asks for heat.
    Heating,
    /// Home Assistant asks for none.
    Idle,
    /// The dial was turned: the Nest heats to it for now.
    Dial,
}

impl Phase {
    pub fn key(self) -> &'static str {
        match self {
            Phase::Nest => "nest",
            Phase::OnItsOwn => "on_its_own",
            Phase::Heating => "heating",
            Phase::Idle => "idle",
            Phase::Dial => "dial",
        }
    }
}

/// An ask, as set/call_for_heat carries it.
#[derive(Debug, PartialEq)]
pub struct Ask {
    pub heat: bool,
    pub lease: i64,
    pub rooms: Vec<String>,
}

impl Ask {
    pub fn parse(arg: &str) -> Result<Ask, String> {
        let arg = arg.trim();
        match arg.to_ascii_uppercase().as_str() {
            "ON" | "1" | "TRUE" => return Ok(Ask { heat: true, lease: LEASE, rooms: vec![] }),
            "OFF" | "0" | "FALSE" => return Ok(Ask { heat: false, lease: LEASE, rooms: vec![] }),
            _ => {}
        }
        let bad = |e: &str| Err(format!("expected ON, OFF or {{\"heat\": true, \"lease\": 600, \"rooms\": [...]}}: {e}"));
        let Ok(Value::Object(o)) = serde_json::from_str::<Value>(arg) else {
            return bad("not ON, OFF or a JSON object");
        };
        let Some(heat) = o.get("heat").and_then(Value::as_bool) else {
            return bad("heat must be true or false");
        };
        let lease = match o.get("lease") {
            None | Some(Value::Null) => LEASE,
            Some(v) => match number(Some(v)) {
                Some(s) if (LEASE_RANGE.0 as f64..=LEASE_RANGE.1 as f64).contains(&s) => s as i64,
                _ => return bad(&format!("lease must be {}-{} seconds", LEASE_RANGE.0, LEASE_RANGE.1)),
            },
        };
        let rooms = o.get("rooms").and_then(Value::as_array).map_or(vec![], |l| {
            l.iter().filter_map(Value::as_str).map(str::to_string).take(12).collect()
        });
        Ok(Ask { heat, lease, rooms })
    }
}

/// The local time a turn of the dial is measured from: the day (0 Monday)
/// and seconds after midnight.
#[derive(Clone, Copy, Debug)]
pub struct Local {
    pub day: usize,
    pub seconds: f64,
}

impl Local {
    pub fn now() -> Local {
        use chrono::{Datelike, Timelike};
        let t = chrono::Local::now();
        Local { day: t.weekday().num_days_from_monday() as usize, seconds: t.num_seconds_from_midnight() as f64 }
    }
}

pub struct HeatControl {
    file: Mutex<PathBuf>,
    inner: Mutex<Inner>,
}

impl Default for HeatControl {
    fn default() -> HeatControl {
        HeatControl::at(PathBuf::from(FILE))
    }
}

fn empty_week() -> Value {
    let mut days = Fields::new();
    for d in 0..7 {
        days.insert(d.to_string(), Value::Object(Fields::new()));
    }
    Value::Object(days)
}

/// Whether a schedule's days hold any setpoint.
fn has_setpoints(days: &Value) -> bool {
    days.as_object().is_some_and(|d| d.values().any(|day| {
        day.as_object().is_some_and(|e| e.values().any(|e| sprint(e.get("entry_type")) != "continuation"))
    }))
}

/// Seconds from `at` until the schedule's next setpoint, in the week ahead.
fn next_setpoint(days: &Value, at: Local) -> Option<i64> {
    let mut best: Option<f64> = None;
    for ahead in 0..8 {
        let day = (at.day + ahead) % 7;
        let Some(entries) = days.get(day.to_string()).and_then(Value::as_object) else { continue };
        for e in entries.values() {
            if sprint(e.get("entry_type")) == "continuation" {
                continue;
            }
            let Some(time) = number(e.get("time")) else { continue };
            let wait = ahead as f64 * 86400.0 + time - at.seconds;
            if wait > 0.0 && best.is_none_or(|b| wait < b) {
                best = Some(wait);
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.map(|b| b as i64)
}

impl HeatControl {
    /// Kept at `file`, as found there.
    pub fn at(file: PathBuf) -> HeatControl {
        let kept = std::fs::read_to_string(&file).ok()
            .and_then(|t| serde_json::from_str::<Kept>(&t).ok())
            .unwrap_or_default();
        HeatControl { file: Mutex::new(file), inner: Mutex::new(Inner { kept, ..Default::default() }) }
    }

    /// Kept at `file` from now on, as found there (tests: never the unit's).
    #[cfg(test)]
    pub fn move_to(&self, file: PathBuf) {
        *self.inner.lock().unwrap() = HeatControl::at(file.clone()).inner.into_inner().unwrap();
        *self.file.lock().unwrap() = file;
    }

    fn save(&self, kept: &Kept) {
        let file = self.file.lock().unwrap().clone();
        if let Err(e) = std::fs::write(&file, serde_json::to_string(kept).unwrap_or_default()) {
            crate::logf!("heat control: {}: {e}", file.display());
        }
    }

    pub fn enabled(&self) -> bool {
        self.inner.lock().unwrap().kept.enabled
    }

    /// Whether the Nest's schedule is aside: Home Assistant decides.
    #[cfg(test)]
    pub fn holding(&self) -> bool {
        self.inner.lock().unwrap().kept.aside.is_some()
    }

    pub fn phase(&self, now: i64) -> Phase {
        let i = self.inner.lock().unwrap();
        if !i.kept.enabled {
            Phase::Nest
        } else if i.kept.aside.is_none() {
            Phase::OnItsOwn
        } else if i.override_until.is_some_and(|u| now < u) {
            Phase::Dial
        } else if i.heat {
            Phase::Heating
        } else {
            Phase::Idle
        }
    }

    /// When a turn of the dial gives way to Home Assistant again.
    pub fn dial_until(&self, now: i64) -> Option<i64> {
        self.inner.lock().unwrap().override_until.filter(|u| now < *u)
    }

    /// Turns heating control on or off; off puts the schedule back.
    pub fn set_enabled(&self, on: bool) -> Vec<Change> {
        let mut i = self.inner.lock().unwrap();
        let mut changes = Vec::new();
        if !on {
            changes = release(&mut i);
        }
        i.kept.enabled = on;
        let kept = i.kept.clone();
        drop(i);
        self.save(&kept);
        changes
    }

    /// An ask from Home Assistant: the first moves the schedule aside.
    pub fn ask(&self, ask: Ask, now: i64, schedule: Option<&Fields>, device: &Fields, shared: &Fields) -> Result<Vec<Change>, String> {
        let mut i = self.inner.lock().unwrap();
        if !i.kept.enabled {
            return Err("heating control is the Nest's: turn heat_control on first".into());
        }
        i.lease_until = Some(now + ask.lease);
        i.heat = ask.heat;
        i.rooms = ask.rooms;
        let mut changes = Vec::new();
        if i.kept.aside.is_none() {
            let days = schedule.and_then(|s| s.get("days")).cloned().unwrap_or_else(empty_week);
            let mode = Some(sprint(shared.get("target_temperature_type"))).filter(|m| !m.is_empty() && m != "off")
                .unwrap_or_else(heat_mode);
            i.kept.aside = Some(Aside { days, learning: truthy(device.get("learning_mode")), mode });
            changes.push(("schedule".into(), fields!{"days" => empty_week()}));
            changes.push(("device".into(), fields!{"learning_mode" => false}));
            let kept = i.kept.clone();
            drop(i);
            self.save(&kept);
            i = self.inner.lock().unwrap();
        }
        changes.extend(steer(&mut i, now, shared));
        Ok(changes)
    }

    /// Runs every few seconds: lets a lease or a turn of the dial run out,
    /// and keeps the target where Home Assistant asked.
    pub fn tick(&self, now: i64, shared: &Fields) -> Vec<Change> {
        let mut i = self.inner.lock().unwrap();
        if i.kept.aside.is_none() {
            return vec![];
        }
        if i.lease_until.is_none_or(|u| now >= u) {
            crate::logf!("heat control: no ask from Home Assistant in time; the Nest's own schedule is back");
            let changes = release(&mut i);
            let kept = i.kept.clone();
            drop(i);
            self.save(&kept);
            return changes;
        }
        steer(&mut i, now, shared)
    }

    /// The unit changed itself: a turn of the dial (or its mode), unless it
    /// is what was written.
    pub fn saw_target(&self, now: i64, at: Local, shared: &Fields) {
        let i = self.inner.lock().unwrap();
        let (Some(_), Some(heating)) = (i.kept.aside.as_ref(), i.written) else { return };
        if i.override_until.is_some_and(|u| now < u) {
            return;
        }
        let mode = sprint(shared.get("target_temperature_type"));
        let target = number(shared.get("target_temperature"));
        let as_written = if heating {
            mode == "heat" && target.is_some_and(|t| (t - HEAT).abs() <= TOLERANCE)
        } else {
            mode == "off"
        };
        if !as_written {
            drop(i);
            self.dial(now, at);
        }
    }

    /// A turn of the dial, or a target or mode set from the climate entity:
    /// the owner's until the schedule's next change. Whether it took.
    pub fn dial(&self, now: i64, at: Local) -> bool {
        let mut i = self.inner.lock().unwrap();
        let Some(aside) = i.kept.aside.as_ref() else { return false };
        let hold = next_setpoint(&aside.days, at).unwrap_or(OVERRIDE);
        if !i.override_until.is_some_and(|u| now < u) {
            crate::logf!("heat control: the dial; Home Assistant waits {} min", hold / 60);
        }
        i.override_until = Some(now + hold);
        true
    }

    /// Whether Home Assistant decides right now (not the dial).
    pub fn deciding(&self, now: i64) -> bool {
        matches!(self.phase(now), Phase::Heating | Phase::Idle)
    }

    /// The schedule while it is aside, as its bucket would hold it.
    pub fn aside_schedule(&self, bucket: Option<&Fields>) -> Option<Fields> {
        let i = self.inner.lock().unwrap();
        let aside = i.kept.aside.as_ref()?;
        let mut s = bucket.cloned().unwrap_or_default();
        s.insert("days".into(), aside.days.clone());
        Some(s)
    }

    /// A schedule written while the schedule is aside changes the one aside.
    pub fn edit_aside(&self, fields: &Fields) -> bool {
        let mut i = self.inner.lock().unwrap();
        let Some(aside) = i.kept.aside.as_mut() else { return false };
        if let Some(days) = fields.get("days") {
            aside.days = days.clone();
        }
        // A schedule someone wrote: learning stays off when it comes back.
        aside.learning = false;
        let kept = i.kept.clone();
        drop(i);
        self.save(&kept);
        true
    }

    /// For Home Assistant's state.
    pub fn shown(&self, now: i64) -> Fields {
        let phase = self.phase(now);
        let i = self.inner.lock().unwrap();
        let rfc = |t: i64| chrono::DateTime::from_timestamp(t, 0).map_or(Value::Null, |d| d.to_rfc3339().into());
        fields!{
            "heat_control" => i.kept.enabled,
            "heat_control_phase" => phase.key(),
            "heat_control_rooms" => i.rooms.clone(),
            "heat_control_lease_until" => i.lease_until.filter(|_| i.kept.aside.is_some()).map_or(Value::Null, rfc),
            "heat_control_dial_until" => i.override_until.filter(|u| now < *u).map_or(Value::Null, rfc),
        }
    }
}

/// What Home Assistant's ask means: heat at HEAT, or off. Written if the
/// unit has something else.
fn steer(i: &mut Inner, now: i64, shared: &Fields) -> Vec<Change> {
    if i.override_until.is_some_and(|u| now < u) {
        return vec![];
    }
    if i.override_until.take().is_some() {
        crate::logf!("heat control: back to Home Assistant after the dial");
    }
    i.written = Some(i.heat);
    let mode = sprint(shared.get("target_temperature_type"));
    let target = number(shared.get("target_temperature"));
    if i.heat {
        if mode == "heat" && target.is_some_and(|t| (t - HEAT).abs() <= TOLERANCE) {
            return vec![];
        }
        vec![("shared".into(), fields!{"target_temperature_type" => "heat", "target_temperature" => HEAT,
                                        "target_change_pending" => true})]
    } else {
        if mode == "off" {
            return vec![];
        }
        vec![("shared".into(), fields!{"target_temperature_type" => "off", "target_change_pending" => true})]
    }
}

/// The schedule and learning back as they were; with no schedule, FALLBACK.
fn release(i: &mut Inner) -> Vec<Change> {
    i.lease_until = None;
    i.override_until = None;
    i.written = None;
    i.heat = false;
    i.rooms.clear();
    let Some(aside) = i.kept.aside.take() else { return vec![] };
    let mut shared = fields!{"target_temperature_type" => aside.mode.clone(), "target_change_pending" => true};
    if !has_setpoints(&aside.days) {
        shared.insert("target_temperature".into(), FALLBACK.into());
    }
    vec![
        ("schedule".into(), fields!{"days" => aside.days.clone()}),
        ("device".into(), fields!{"learning_mode" => aside.learning}),
        ("shared".into(), shared),
    ]
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    pub fn temp_file() -> PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("nestlocal-heat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("heat{}", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)))
    }

    fn obj(v: Value) -> Fields {
        v.as_object().unwrap().clone()
    }

    fn week() -> Fields {
        obj(json!({"days": {"0": {"0": {"entry_type": "continuation", "time": 0, "temp": 17},
                                   "1": {"entry_type": "setpoint", "time": 23400, "temp": 20}},
                            "1": {}, "2": {}, "3": {}, "4": {}, "5": {}, "6": {}}}))
    }

    fn changed<'a>(changes: &'a [Change], bucket: &str) -> Option<&'a Fields> {
        changes.iter().rev().find(|(b, _)| b == bucket).map(|(_, f)| f)
    }

    #[test]
    fn asks_parse() {
        assert_eq!(Ask::parse("ON"), Ok(Ask { heat: true, lease: LEASE, rooms: vec![] }));
        assert_eq!(Ask::parse(" off "), Ok(Ask { heat: false, lease: LEASE, rooms: vec![] }));
        assert_eq!(Ask::parse(r#"{"heat": true, "lease": 120, "rooms": ["Office", 3, "Snug"]}"#),
                   Ok(Ask { heat: true, lease: 120, rooms: vec!["Office".into(), "Snug".into()] }));
        assert!(Ask::parse(r#"{"heat": true, "lease": 5}"#).is_err());
        assert!(Ask::parse(r#"{"lease": 600}"#).is_err());
        assert!(Ask::parse("maybe").is_err());
    }

    #[test]
    fn off_it_ignores_asks() {
        let c = HeatControl::at(temp_file());
        assert_eq!(c.phase(0), Phase::Nest);
        assert!(c.ask(Ask::parse("ON").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).is_err());
        assert!(!c.holding());
    }

    #[test]
    fn an_ask_moves_the_schedule_aside_and_a_lapse_puts_it_back() {
        let file = temp_file();
        let c = HeatControl::at(file.clone());
        c.set_enabled(true);
        assert_eq!(c.phase(0), Phase::OnItsOwn);
        let dev = obj(json!({"learning_mode": true}));
        let sh = obj(json!({"target_temperature": 20.0, "target_temperature_type": "heat"}));
        let changes = c.ask(Ask::parse("ON").unwrap(), 1000, Some(&week()), &dev, &sh).unwrap();
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], empty_week());
        assert_eq!(changed(&changes, "device").unwrap()["learning_mode"], json!(false));
        assert_eq!(changed(&changes, "shared").unwrap()["target_temperature"], json!(HEAT));
        assert_eq!(c.phase(1000), Phase::Heating);
        assert!(HeatControl::at(file.clone()).holding(), "kept across a restart");

        // Renewed: nothing more to change while it heats at HEAT.
        let sh = obj(json!({"target_temperature": HEAT, "target_temperature_type": "heat"}));
        assert!(c.tick(1500, &sh).is_empty());
        // No heat wanted: off.
        let changes = c.ask(Ask::parse("OFF").unwrap(), 1500, None, &dev, &sh).unwrap();
        assert_eq!(changes, vec![("shared".into(), fields!{"target_temperature_type" => "off", "target_change_pending" => true})]);
        let off = obj(json!({"target_temperature": HEAT, "target_temperature_type": "off"}));
        c.saw_target(1500, Local { day: 0, seconds: 0.0 }, &off);
        assert_eq!(c.phase(1500), Phase::Idle, "off as written: not the dial");

        // No ask for the lease: the schedule, learning and heat come back.
        let changes = c.tick(1500 + LEASE, &off);
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], week()["days"]);
        assert_eq!(changed(&changes, "device").unwrap()["learning_mode"], json!(true));
        let shared = changed(&changes, "shared").unwrap();
        assert_eq!(shared["target_temperature_type"], json!("heat"));
        assert!(shared.get("target_temperature").is_none(), "it has a schedule to run");
        assert_eq!(c.phase(1500 + LEASE), Phase::OnItsOwn);
        assert!(!HeatControl::at(file).holding());
    }

    #[test]
    fn with_no_schedule_to_go_back_to_it_holds_the_fallback() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true);
        c.ask(Ask::parse("ON").unwrap(), 0, None, &Fields::new(), &Fields::new()).unwrap();
        let changes = c.tick(LEASE, &Fields::new());
        assert_eq!(changed(&changes, "shared").unwrap()["target_temperature"], json!(FALLBACK));
    }

    #[test]
    fn a_restart_while_holding_gives_the_schedule_back() {
        let file = temp_file();
        let c = HeatControl::at(file.clone());
        c.set_enabled(true);
        c.ask(Ask::parse("ON").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).unwrap();
        let after = HeatControl::at(file);
        let changes = after.tick(1, &Fields::new());
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], week()["days"]);
    }

    #[test]
    fn turning_it_off_gives_the_schedule_back() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true);
        c.ask(Ask::parse("ON").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).unwrap();
        let changes = c.set_enabled(false);
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], week()["days"]);
        assert_eq!(c.phase(0), Phase::Nest);
    }

    #[test]
    fn the_dial_holds_until_the_schedules_next_change() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true);
        c.ask(Ask::parse("OFF").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).unwrap();
        // What was written: not the dial.
        c.saw_target(10, Local { day: 0, seconds: 3600.0 }, &obj(json!({"target_temperature_type": "off"})));
        assert_eq!(c.phase(10), Phase::Idle);
        // Turned to 21 at 01:00 Monday: holds to the 06:30 setpoint.
        c.saw_target(10, Local { day: 0, seconds: 3600.0 },
                     &obj(json!({"target_temperature": 21.0, "target_temperature_type": "heat"})));
        assert_eq!(c.phase(10), Phase::Dial);
        assert_eq!(c.dial_until(10), Some(10 + 23400 - 3600));
        assert!(c.tick(20, &obj(json!({"target_temperature": 21.0}))).is_empty(), "left alone");
        c.ask(Ask::parse("OFF").unwrap(), 30, None, &Fields::new(), &Fields::new()).unwrap();
        let after = 10 + 23400 - 3600;
        let changes = c.tick(after, &obj(json!({"target_temperature": 21.0})));
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], week()["days"], "the lease ran out first");
    }

    #[test]
    fn the_dial_gives_way_after_a_while_with_no_schedule() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true);
        c.ask(Ask { heat: false, lease: 3600, rooms: vec![] }, 0, None, &Fields::new(), &Fields::new()).unwrap();
        assert!(c.dial(0, Local { day: 2, seconds: 0.0 }));
        assert_eq!(c.dial_until(0), Some(OVERRIDE));
        c.ask(Ask { heat: false, lease: 3600, rooms: vec![] }, OVERRIDE - 60, None, &Fields::new(), &Fields::new()).unwrap();
        let changes = c.tick(OVERRIDE, &obj(json!({"target_temperature": 22.0, "target_temperature_type": "heat"})));
        assert_eq!(changed(&changes, "shared").unwrap()["target_temperature_type"], json!("off"));
        assert_eq!(c.phase(OVERRIDE), Phase::Idle);
    }

    #[test]
    fn a_schedule_written_while_aside_changes_the_one_aside() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true);
        assert!(!c.edit_aside(&fields!{"days" => empty_week()}), "nothing aside");
        c.ask(Ask::parse("ON").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).unwrap();
        let new = obj(json!({"days": {"0": {"1": {"entry_type": "setpoint", "time": 3600, "temp": 19}}}}));
        assert!(c.edit_aside(&new));
        assert_eq!(c.aside_schedule(None).unwrap()["days"], new["days"]);
        let changes = c.set_enabled(false);
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], new["days"]);
    }

    #[test]
    fn next_setpoints_wrap_the_week() {
        let w = week();
        assert_eq!(next_setpoint(&w["days"], Local { day: 0, seconds: 0.0 }), Some(23400));
        assert_eq!(next_setpoint(&w["days"], Local { day: 0, seconds: 30000.0 }), Some(7 * 86400 - 30000 + 23400));
        assert_eq!(next_setpoint(&empty_week(), Local { day: 3, seconds: 0.0 }), None);
    }
}
