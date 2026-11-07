//! The unit's hot water schedule as Home Assistant reads and writes it: per
//! day, the periods hot water is on.
//!
//! nlclient keeps it as a custom schedule (cloud.rs gives it its bucket,
//! custom_schedule.<id>, with "mode": "hotwater") whose "intervals" list
//! the on periods: start and end as a day (0 Monday to 6 Sunday) and
//! seconds after midnight. A write replaces "intervals" whole, so a command
//! changing some days rebuilds the list: days it names get new periods,
//! the others keep theirs.

use serde_json::Value;

use crate::fields;
use crate::json::{num, number, sprint, Fields};

const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

/// Marks the periods a command writes as the owner's, as the schedule's
/// setpoints are (schedule.rs).
const TOUCHED_BY_LOCAL: i64 = 4;

const DAY: i64 = 24 * 3600;

/// Whether a custom schedule bucket is the hot water one.
pub fn is_hot_water(bucket: &Fields) -> bool {
    sprint(bucket.get("mode")).eq_ignore_ascii_case("hotwater")
}

fn hhmm(seconds: i64) -> String {
    let m = seconds.rem_euclid(DAY) / 60;
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// (start day, start s, end day, end s) of each interval, in week order.
fn periods(bucket: &Fields) -> Vec<(i64, i64, i64, i64)> {
    let mut v: Vec<_> = bucket.get("intervals").and_then(Value::as_array).map_or(Vec::new(), |l| {
        l.iter().filter_map(|i| {
            let n = |k| number(i.get(k)).map(|x| x as i64);
            Some((n("start_day")?, n("start_time")?, n("end_day")?, n("end_time")?))
        }).collect()
    });
    v.sort();
    v
}

/// The schedule as Home Assistant reads it: by day, the on periods in time
/// order as {"on": "HH:MM", "off": "HH:MM"}; one that runs past midnight
/// is listed under the day it starts, with an "off" before its "on".
pub fn simple(bucket: &Fields) -> Value {
    let mut days = Fields::new();
    for (i, name) in DAYS.iter().enumerate() {
        let list: Vec<Value> = periods(bucket).into_iter()
            .filter(|(sd, ..)| *sd == i as i64)
            .map(|(_, st, _, et)| Value::Object(fields!{"on" => hhmm(st), "off" => hhmm(et)}))
            .collect();
        days.insert((*name).into(), Value::Array(list));
    }
    Value::Object(fields!{"days" => Value::Object(days)})
}

/// "HH:MM" as seconds after midnight.
fn seconds(s: &str) -> Option<i64> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m) && s.len() == 5).then_some(h * 3600 + m * 60)
}

/// One day's new on periods: its index from Monday, and (on s, off s) in
/// time order, off after on (an off at or before its on is the next day's).
type DayChange = (usize, Vec<(i64, i64)>);

/// The days a command names, as new on periods.
fn parse(arg: &str) -> Result<Vec<DayChange>, String> {
    let bad = |e: &str| Err(format!("expected {{\"mon\": [{{\"on\": \"06:00\", \"off\": \"08:00\"}}], ...}}: {e}"));
    let Ok(Value::Object(o)) = serde_json::from_str::<Value>(arg) else {
        return bad("not a JSON object");
    };
    let mut out = Vec::new();
    for (name, list) in &o {
        let Some(day) = DAYS.iter().position(|d| d == name) else {
            return bad(&format!("unknown day {name}"));
        };
        let Some(list) = list.as_array() else {
            return bad(&format!("{name} is not a list"));
        };
        let mut v = Vec::new();
        for p in list {
            let t = |k| p.get(k).and_then(Value::as_str).and_then(seconds);
            let (Some(on), Some(off)) = (t("on"), t("off")) else {
                return bad(&format!("{name}: each period needs an on and an off, HH:MM"));
            };
            v.push((on, if off > on { off } else { off + DAY }));
        }
        v.sort();
        if v.windows(2).any(|w| w[1].0 < w[0].1) {
            return bad(&format!("{name}: periods overlap"));
        }
        if v.last().is_some_and(|l| l.1 > DAY + v[0].0) {
            return bad(&format!("{name}: a period runs past the next day's first"));
        }
        out.push((day, v));
    }
    Ok(out)
}

/// The bucket fields a command writes: every interval, with the days it
/// names replaced.
pub fn write(bucket: &Fields, arg: &str, now: i64) -> Result<Fields, String> {
    let changes = parse(arg)?;
    let kept = bucket.get("intervals").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut intervals: Vec<Value> = kept.into_iter().filter(|i| {
        let day = number(i.get("start_day")).map(|d| d as usize);
        !changes.iter().any(|(d, _)| Some(*d) == day)
    }).collect();
    for (day, list) in &changes {
        for (on, off) in list {
            let end_day = (*day as i64 + off / DAY) % 7;
            intervals.push(Value::Object(fields!{
                "start_day" => num(*day as f64),
                "start_time" => num(*on as f64),
                "end_day" => num(end_day as f64),
                "end_time" => num((off % DAY) as f64),
                "repeat" => vec![num(*day as f64)],
                "touched_at" => num(now as f64),
                "touched_by" => num(TOUCHED_BY_LOCAL as f64),
                "touched_tzo" => 0,
            }));
        }
    }
    let at = |i: &Value| (number(i.get("start_day")).unwrap_or(0.0) as i64, number(i.get("start_time")).unwrap_or(0.0) as i64);
    intervals.sort_by_key(at);
    Ok(fields!{"intervals" => Value::Array(intervals)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bucket() -> Fields {
        // As nlclient uploaded it: Monday 06:00-08:00 and 17:00-20:00,
        // Friday 15:30-16:30.
        let i = |sd, st, et| json!({"start_day": sd, "start_time": st, "end_day": sd, "end_time": et,
            "repeat": [sd], "touched_at": 1694618279, "touched_by": 1, "touched_tzo": -25200});
        let v = json!({"mode": "hotwater", "name": "HOTWATER", "is_built_in": true, "version": "1",
            "intervals": [i(0, 61200, 72000), i(0, 21600, 28800), i(4, 55800, 59400)]});
        v.as_object().unwrap().clone()
    }

    #[test]
    fn reads_the_on_periods_by_day_in_time_order() {
        let s = simple(&bucket());
        assert_eq!(s["days"]["mon"], json!([{"on": "06:00", "off": "08:00"}, {"on": "17:00", "off": "20:00"}]));
        assert_eq!(s["days"]["fri"], json!([{"on": "15:30", "off": "16:30"}]));
        assert_eq!(s["days"]["sun"], json!([]));
        assert!(is_hot_water(&bucket()));
    }

    #[test]
    fn a_write_replaces_the_days_it_names_and_keeps_the_rest() {
        let f = write(&bucket(), r#"{"fri": [{"on": "18:00", "off": "19:00"}], "sat": [{"on": "23:00", "off": "01:00"}]}"#, 1_800_000_000).unwrap();
        let mut b = bucket();
        b.extend(f);
        let s = simple(&b);
        assert_eq!(s["days"]["mon"].as_array().unwrap().len(), 2, "untouched");
        assert_eq!(s["days"]["fri"], json!([{"on": "18:00", "off": "19:00"}]));
        assert_eq!(s["days"]["sat"], json!([{"on": "23:00", "off": "01:00"}]));
        let sat = b["intervals"].as_array().unwrap().iter().find(|i| i["start_day"] == json!(5)).unwrap();
        assert_eq!((&sat["end_day"], &sat["end_time"], &sat["touched_by"]), (&json!(6), &json!(3600), &json!(4)));
        // An empty list turns a day's hot water off.
        let f = write(&bucket(), r#"{"mon": []}"#, 0).unwrap();
        assert_eq!(f["intervals"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn a_bad_command_is_refused() {
        for bad in ["[]", r#"{"funday": []}"#, r#"{"mon": [{"on": "6:00", "off": "08:00"}]}"#,
            r#"{"mon": [{"on": "06:00", "off": "09:00"}, {"on": "08:00", "off": "10:00"}]}"#] {
            assert!(write(&bucket(), bad, 0).is_err(), "{bad}");
        }
    }
}
