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
//! The ask is the only say while it holds: whatever else changes the target
//! or mode (Nest's own dial, the climate entity, the unit itself) is put back
//! to the ask within a tick, and commands that would change them are refused
//! (commands.rs). To set the Nest by hand, turn heating control off.
//!
//! When the lease runs out (Home Assistant, the broker or the network gone)
//! the schedule and learning go back, with the target at the schedule's
//! setpoint for now (not HEAT, left over from an ask), and the Nest runs on
//! its own until heat is asked for again: the safety net is the schedule it
//! already had. With no schedule to go back to, it holds FALLBACK. A
//! schedule written while the schedule is aside (set/heating_times,
//! clear_schedule) changes the one aside.

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
/// How far the target may sit from HEAT and still be as asked, °C.
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
}

impl Phase {
    pub fn key(self) -> &'static str {
        match self {
            Phase::Nest => "nest",
            Phase::OnItsOwn => "on_its_own",
            Phase::Heating => "heating",
            Phase::Idle => "idle",
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

/// The local time the schedule's setpoint for now is read at: the day (0
/// Monday) and seconds after midnight.
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

/// The schedule's setpoint at `at`: the day's last entry at or before it,
/// else the last entry of the days before, however far back.
fn setpoint_now(days: &Value, at: Local) -> Option<f64> {
    let entries = |day: usize| -> Vec<(f64, f64)> {
        let mut out: Vec<(f64, f64)> = days.get(day.to_string()).and_then(Value::as_object)
            .map(|e| e.values().filter_map(|e| Some((number(e.get("time"))?, number(e.get("temp"))?))).collect())
            .unwrap_or_default();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    };
    if let Some(t) = entries(at.day).iter().rev().find(|(time, _)| *time <= at.seconds) {
        return Some(t.1);
    }
    (1..=7).find_map(|back| entries((at.day + 7 - back) % 7).last().map(|t| t.1))
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

    pub fn phase(&self) -> Phase {
        let i = self.inner.lock().unwrap();
        if !i.kept.enabled {
            Phase::Nest
        } else if i.kept.aside.is_none() {
            Phase::OnItsOwn
        } else if i.heat {
            Phase::Heating
        } else {
            Phase::Idle
        }
    }

    /// Turns heating control on or off; off puts the schedule back.
    pub fn set_enabled(&self, on: bool, at: Local) -> Vec<Change> {
        let mut i = self.inner.lock().unwrap();
        let mut changes = Vec::new();
        if !on {
            changes = release(&mut i, at);
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
            let mode = Some(sprint(shared.get("target_temperature_type"))).filter(|m| matches!(m.as_str(), "heat" | "cool" | "range"))
                .unwrap_or_else(heat_mode);
            i.kept.aside = Some(Aside { days, learning: truthy(device.get("learning_mode")), mode });
            changes.push(("schedule".into(), fields!{"days" => empty_week()}));
            changes.push(("device".into(), fields!{"learning_mode" => false}));
            let kept = i.kept.clone();
            drop(i);
            self.save(&kept);
            i = self.inner.lock().unwrap();
        }
        changes.extend(steer(&i, shared));
        Ok(changes)
    }

    /// Runs every few seconds: lets a lease run out, and otherwise puts the
    /// target and mode back where Home Assistant asked, whatever moved them.
    pub fn tick(&self, now: i64, at: Local, shared: &Fields) -> Vec<Change> {
        let mut i = self.inner.lock().unwrap();
        if i.kept.aside.is_none() {
            return vec![];
        }
        if i.lease_until.is_none_or(|u| now >= u) {
            crate::logf!("heat control: no ask from Home Assistant in time; the Nest's own schedule is back");
            let changes = release(&mut i, at);
            let kept = i.kept.clone();
            drop(i);
            self.save(&kept);
            return changes;
        }
        steer(&i, shared)
    }

    /// Whether Home Assistant decides right now: an ask holds.
    pub fn deciding(&self) -> bool {
        matches!(self.phase(), Phase::Heating | Phase::Idle)
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
    pub fn shown(&self) -> Fields {
        let phase = self.phase();
        let i = self.inner.lock().unwrap();
        let rfc = |t: i64| chrono::DateTime::from_timestamp(t, 0).map_or(Value::Null, |d| d.to_rfc3339().into());
        fields!{
            "heat_control" => i.kept.enabled,
            "heat_control_phase" => phase.key(),
            "heat_control_rooms" => i.rooms.clone(),
            "heat_control_lease_until" => i.lease_until.filter(|_| i.kept.aside.is_some()).map_or(Value::Null, rfc),
        }
    }
}

/// What Home Assistant's ask means: heat at HEAT, or off. Written if the
/// unit has anything else, whoever put it there.
fn steer(i: &Inner, shared: &Fields) -> Vec<Change> {
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

/// The schedule and learning back as they were, the target at the
/// schedule's setpoint for now; with no schedule, FALLBACK.
fn release(i: &mut Inner, at: Local) -> Vec<Change> {
    i.lease_until = None;
    i.heat = false;
    i.rooms.clear();
    let Some(aside) = i.kept.aside.take() else { return vec![] };
    // The target as an ask left it (HEAT, say) would hold until the
    // schedule's next change: the setpoint for now instead.
    let target = if has_setpoints(&aside.days) { setpoint_now(&aside.days, at).unwrap_or(FALLBACK) } else { FALLBACK };
    let shared = fields!{"target_temperature_type" => aside.mode.clone(), "target_temperature" => target,
                         "target_change_pending" => true};
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

    /// Monday, midnight.
    const MON: Local = Local { day: 0, seconds: 0.0 };

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
        assert_eq!(c.phase(), Phase::Nest);
        assert!(c.ask(Ask::parse("ON").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).is_err());
        assert!(!c.holding());
    }

    #[test]
    fn an_ask_moves_the_schedule_aside_and_a_lapse_puts_it_back() {
        let file = temp_file();
        let c = HeatControl::at(file.clone());
        c.set_enabled(true, MON);
        assert_eq!(c.phase(), Phase::OnItsOwn);
        let dev = obj(json!({"learning_mode": true}));
        let sh = obj(json!({"target_temperature": 20.0, "target_temperature_type": "heat"}));
        let changes = c.ask(Ask::parse("ON").unwrap(), 1000, Some(&week()), &dev, &sh).unwrap();
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], empty_week());
        assert_eq!(changed(&changes, "device").unwrap()["learning_mode"], json!(false));
        assert_eq!(changed(&changes, "shared").unwrap()["target_temperature"], json!(HEAT));
        assert_eq!(c.phase(), Phase::Heating);
        assert!(HeatControl::at(file.clone()).holding(), "kept across a restart");

        // Renewed: nothing more to change while it heats at HEAT.
        let sh = obj(json!({"target_temperature": HEAT, "target_temperature_type": "heat"}));
        assert!(c.tick(1500, MON, &sh).is_empty());
        // No heat wanted: off.
        let changes = c.ask(Ask::parse("OFF").unwrap(), 1500, None, &dev, &sh).unwrap();
        assert_eq!(changes, vec![("shared".into(), fields!{"target_temperature_type" => "off", "target_change_pending" => true})]);
        let off = obj(json!({"target_temperature": HEAT, "target_temperature_type": "off"}));
        assert!(c.tick(1510, MON, &off).is_empty(), "off as asked");
        assert_eq!(c.phase(), Phase::Idle);

        // No ask for the lease: the schedule, learning and heat come back.
        let changes = c.tick(1500 + LEASE, MON, &off);
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], week()["days"]);
        assert_eq!(changed(&changes, "device").unwrap()["learning_mode"], json!(true));
        let shared = changed(&changes, "shared").unwrap();
        assert_eq!(shared["target_temperature_type"], json!("heat"));
        assert_eq!(shared["target_temperature"], json!(17.0), "the schedule's for now, not HEAT");
        assert_eq!(c.phase(), Phase::OnItsOwn);
        assert!(!HeatControl::at(file).holding());
    }

    #[test]
    fn with_no_schedule_to_go_back_to_it_holds_the_fallback() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true, MON);
        c.ask(Ask::parse("ON").unwrap(), 0, None, &Fields::new(), &Fields::new()).unwrap();
        let changes = c.tick(LEASE, MON, &Fields::new());
        assert_eq!(changed(&changes, "shared").unwrap()["target_temperature"], json!(FALLBACK));
    }

    #[test]
    fn a_restart_while_holding_gives_the_schedule_back() {
        let file = temp_file();
        let c = HeatControl::at(file.clone());
        c.set_enabled(true, MON);
        c.ask(Ask::parse("ON").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).unwrap();
        let after = HeatControl::at(file);
        let changes = after.tick(1, MON, &Fields::new());
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], week()["days"]);
    }

    #[test]
    fn turning_it_off_gives_the_schedule_back() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true, MON);
        c.ask(Ask::parse("ON").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).unwrap();
        let changes = c.set_enabled(false, MON);
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], week()["days"]);
        assert_eq!(c.phase(), Phase::Nest);
    }

    #[test]
    fn whatever_moves_the_target_while_an_ask_holds_is_put_back() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true, MON);
        c.ask(Ask::parse("OFF").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).unwrap();
        // Nest's own dial (or anything else) to 21, heating: off again.
        let dialled = obj(json!({"target_temperature": 21.0, "target_temperature_type": "heat"}));
        let changes = c.tick(10, MON, &dialled);
        assert_eq!(changed(&changes, "shared").unwrap()["target_temperature_type"], json!("off"));
        assert_eq!(c.phase(), Phase::Idle);
        // Asked to heat, turned down to 18: HEAT again.
        c.ask(Ask::parse("ON").unwrap(), 20, None, &Fields::new(), &dialled).unwrap();
        let changes = c.tick(30, MON, &obj(json!({"target_temperature": 18.0, "target_temperature_type": "heat"})));
        assert_eq!(changed(&changes, "shared").unwrap()["target_temperature"], json!(HEAT));
        // Put in eco's range or cool: heat at HEAT.
        let changes = c.tick(40, MON, &obj(json!({"target_temperature": HEAT, "target_temperature_type": "cool"})));
        assert_eq!(changed(&changes, "shared").unwrap()["target_temperature_type"], json!("heat"));
        assert_eq!(c.phase(), Phase::Heating);
    }

    #[test]
    fn a_schedule_written_while_aside_changes_the_one_aside() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true, MON);
        assert!(!c.edit_aside(&fields!{"days" => empty_week()}), "nothing aside");
        c.ask(Ask::parse("ON").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).unwrap();
        let new = obj(json!({"days": {"0": {"1": {"entry_type": "setpoint", "time": 3600, "temp": 19}}}}));
        assert!(c.edit_aside(&new));
        assert_eq!(c.aside_schedule(None).unwrap()["days"], new["days"]);
        let changes = c.set_enabled(false, MON);
        assert_eq!(changed(&changes, "schedule").unwrap()["days"], new["days"]);
    }

    #[test]
    fn the_setpoint_now_carries_on_from_before() {
        let w = week();
        assert_eq!(setpoint_now(&w["days"], Local { day: 0, seconds: 100.0 }), Some(17.0));
        assert_eq!(setpoint_now(&w["days"], Local { day: 0, seconds: 23400.0 }), Some(20.0));
        assert_eq!(setpoint_now(&w["days"], Local { day: 4, seconds: 0.0 }), Some(20.0), "Monday's last");
        assert_eq!(setpoint_now(&empty_week(), MON), None);
    }

    #[test]
    fn a_lapse_mid_heat_does_not_leave_the_nest_at_heat() {
        let c = HeatControl::at(temp_file());
        c.set_enabled(true, MON);
        c.ask(Ask::parse("ON").unwrap(), 0, Some(&week()), &Fields::new(), &Fields::new()).unwrap();
        let at = Local { day: 0, seconds: 30000.0 };
        let changes = c.tick(LEASE, at, &obj(json!({"target_temperature": HEAT, "target_temperature_type": "heat"})));
        let shared = changed(&changes, "shared").unwrap();
        assert_eq!(shared["target_temperature"], json!(20.0));
        assert_eq!(shared["target_temperature_type"], json!("heat"));
    }
}
