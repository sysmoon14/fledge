//! The Focus card's timer: Pomodoro blocks of focus and breaks, and busy,
//! open-ended. It runs on the Nest, by the wall clock, and is kept on disk
//! (FOCUS), so a block carries on through a restart of nestlocal or Home
//! Assistant. Home Assistant follows it on <base>/focus and drives it with
//! <base>/set/focus; the card shows it and its ring and button work it.
//!
//! A block that runs out waits for a hold before the next starts, unless
//! its card says to go straight on, so a break never starts while whoever
//! it's for is away. Busy pauses a block running, which carries on as busy
//! ends.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Where the timer is kept between starts.
pub const FOCUS: &str = "/media/data/fledge-focus";

/// The lengths a focus block can be given, in minutes, and how the ring
/// steps them while it's ready.
pub const LENGTH_MIN: u32 = 5;
pub const LENGTH_MAX: u32 = 90;
pub const LENGTH_STEP: u32 = 5;
/// A running block is never turned down below this many seconds left.
pub const LEAST_LEFT: f64 = 60.0;
/// A block that ran out longer ago than this (the Nest off meanwhile)
/// ends quietly, without its tones.
pub const LATE: f64 = 90.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Nothing running: the next focus block's length on show.
    #[default]
    Ready,
    Focus,
    ShortBreak,
    LongBreak,
    /// Busy, counting up since `since`.
    Busy,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::Ready => "ready",
            Phase::Focus => "focus",
            Phase::ShortBreak => "short_break",
            Phase::LongBreak => "long_break",
            Phase::Busy => "busy",
        }
    }

    pub fn is_break(self) -> bool {
        matches!(self, Phase::ShortBreak | Phase::LongBreak)
    }
}

/// The card's settings: lengths in minutes, how many focus blocks before
/// the long break, and whether the next block starts by itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub focus: u32,
    pub short_break: u32,
    pub long_break: u32,
    pub every: u32,
    pub straight_on: bool,
}

impl Default for Plan {
    fn default() -> Plan {
        Plan { focus: 25, short_break: 5, long_break: 15, every: 4, straight_on: false }
    }
}

impl Plan {
    /// A phase's whole length, in seconds (Ready's and Busy's: none).
    fn seconds(&self, phase: Phase, focus: u32) -> f64 {
        60.0 * match phase {
            Phase::Focus => focus,
            Phase::ShortBreak => self.short_break,
            Phase::LongBreak => self.long_break,
            Phase::Ready | Phase::Busy => 0,
        }.max(1) as f64
    }
}

/// What a block running out, or a change, says to Home Assistant
/// (<base>/focus/event) and how the Nest sounds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Happened {
    FocusStarted,
    FocusDone,
    BreakStarted,
    BreakDone,
    Paused,
    Resumed,
    BusyStarted,
    BusyEnded,
    Stopped,
}

impl Happened {
    pub fn name(self) -> &'static str {
        match self {
            Happened::FocusStarted => "focus_started",
            Happened::FocusDone => "focus_done",
            Happened::BreakStarted => "break_started",
            Happened::BreakDone => "break_done",
            Happened::Paused => "paused",
            Happened::Resumed => "resumed",
            Happened::BusyStarted => "busy_started",
            Happened::BusyEnded => "busy_ended",
            Happened::Stopped => "stopped",
        }
    }
}

/// A block busy interrupted: what it was, and how much of it was left.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Held {
    pub phase: Phase,
    pub left: f64,
    pub length: f64,
    pub running: bool,
    #[serde(default)]
    pub waiting: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Focus {
    pub phase: Phase,
    /// Running: when it runs out (Unix seconds). None paused, ready,
    /// waiting and busy.
    #[serde(default)]
    pub ends: Option<f64>,
    /// Seconds left while paused or waiting, and the block's whole length.
    #[serde(default)]
    pub left: f64,
    #[serde(default)]
    pub length: f64,
    /// The block ran out and `phase` is the next, waiting for a hold.
    #[serde(default)]
    pub waiting: bool,
    /// Focus blocks finished towards the long break.
    #[serde(default)]
    pub done: u32,
    /// Focus blocks finished on `day` (days since 1970, local).
    #[serde(default)]
    pub today: u32,
    #[serde(default)]
    pub day: i64,
    /// The focus length the ring set (minutes); 0 for the card's.
    #[serde(default)]
    pub chosen: u32,
    /// Busy: since when, whether Home Assistant's entities started it, what
    /// it says it is ("In a call"), and the block it paused.
    #[serde(default)]
    pub since: Option<f64>,
    #[serde(default)]
    pub auto: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub held: Option<Held>,
}

impl Default for Focus {
    fn default() -> Focus {
        Focus { phase: Phase::Ready, ends: None, left: 0.0, length: 0.0, waiting: false, done: 0, today: 0, day: 0,
                chosen: 0, since: None, auto: false, status: String::new(), held: None }
    }
}

impl Focus {
    /// The timer as kept, or a fresh one.
    pub fn load() -> Focus {
        std::fs::read(FOCUS).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let Ok(data) = serde_json::to_vec(self) else { return };
        let temp = format!("{FOCUS}.new");
        if let Err(e) = std::fs::write(&temp, data).and_then(|()| std::fs::rename(&temp, FOCUS)) {
            crate::logf!("ui: {FOCUS}: {e}");
        }
    }

    /// The focus length a start gives, in minutes: the ring's, else the card's.
    pub fn focus_minutes(&self, plan: &Plan) -> u32 {
        if self.chosen != 0 { self.chosen } else { plan.focus.clamp(LENGTH_MIN, LENGTH_MAX) }
    }

    /// Running down now.
    pub fn running(&self) -> bool {
        self.ends.is_some()
    }

    /// A focus block or break started and not yet run out, paused or not.
    pub fn timing(&self) -> bool {
        (self.phase == Phase::Focus || self.phase.is_break()) && !self.waiting
    }

    /// Anything under way: a block (running, paused, or done and waiting
    /// for the next), or busy.
    pub fn active(&self) -> bool {
        self.phase != Phase::Ready
    }

    /// Paused: a block stopped part way, not by busy.
    pub fn paused(&self) -> bool {
        self.timing() && !self.running()
    }

    /// Seconds left of the block at `now`.
    pub fn left_at(&self, now: f64) -> f64 {
        match self.ends {
            Some(ends) => (ends - now).max(0.0),
            None => self.left,
        }
    }

    /// How much of the block is left (1 full, 0 run out).
    pub fn fraction(&self, now: f64) -> f32 {
        if self.length <= 0.0 { 1.0 } else { (self.left_at(now) / self.length).clamp(0.0, 1.0) as f32 }
    }

    /// Whether whoever it's for shouldn't be disturbed: focusing (running)
    /// or busy.
    pub fn busy(&self) -> bool {
        self.phase == Phase::Busy || (self.phase == Phase::Focus && self.running())
    }

    /// The block number shown (1 for the first towards the long break).
    pub fn block(&self) -> u32 {
        match self.phase {
            Phase::Focus => self.done + 1,
            _ => self.done,
        }
    }

    /// A new day: the count of today's blocks starts again.
    pub fn new_day(&mut self, day: i64) -> bool {
        if self.day == day {
            return false;
        }
        self.day = day;
        let had = self.today != 0;
        self.today = 0;
        had
    }

    fn run(&mut self, phase: Phase, seconds: f64, now: f64) {
        self.phase = phase;
        self.length = seconds;
        self.left = seconds;
        self.ends = Some(now + seconds);
        self.waiting = false;
    }

    /// Starts a focus block now, of the length on show.
    pub fn start(&mut self, plan: &Plan, now: f64) -> Happened {
        let seconds = plan.seconds(Phase::Focus, self.focus_minutes(plan));
        self.run(Phase::Focus, seconds, now);
        Happened::FocusStarted
    }

    /// Starts the block waiting, now.
    fn start_waiting(&mut self, now: f64) -> Happened {
        let phase = self.phase;
        let seconds = self.length;
        self.run(phase, seconds, now);
        if phase.is_break() { Happened::BreakStarted } else { Happened::FocusStarted }
    }

    pub fn pause(&mut self, now: f64) -> Option<Happened> {
        let ends = self.ends.take()?;
        self.left = (ends - now).max(0.0);
        Some(Happened::Paused)
    }

    pub fn resume(&mut self, now: f64) -> Option<Happened> {
        if !self.paused() {
            return None;
        }
        self.ends = Some(now + self.left);
        Some(Happened::Resumed)
    }

    /// The hold on the open card: starts what's ready or waiting, pauses
    /// what's running, carries on what's paused, and ends busy.
    pub fn hold(&mut self, plan: &Plan, now: f64) -> Option<Happened> {
        match self.phase {
            Phase::Ready => Some(self.start(plan, now)),
            Phase::Busy => self.end_busy(false, now),
            _ if self.waiting => Some(self.start_waiting(now)),
            _ if self.running() => self.pause(now),
            _ => self.resume(now),
        }
    }

    /// The ring turned `steps`: while ready, the length to start by
    /// LENGTH_STEP minutes; while a block runs or is paused, a minute a step
    /// on or off it. Whether it changed.
    pub fn turn(&mut self, plan: &Plan, steps: i32, now: f64) -> bool {
        match self.phase {
            Phase::Ready => {
                let was = self.focus_minutes(plan);
                let to = (was as i32 + steps * LENGTH_STEP as i32).clamp(LENGTH_MIN as i32, LENGTH_MAX as i32) as u32;
                // Back on the card's own: that again (it follows the card).
                self.chosen = if to == plan.focus { 0 } else { to };
                to != was
            }
            _ if self.timing() => {
                let left = self.left_at(now);
                let to = (left + 60.0 * steps as f64).max(LEAST_LEFT.min(left));
                if (to - left).abs() < 0.5 {
                    return false;
                }
                // The whole grows with it, so the ring never shows more than full.
                self.length = (self.length + to - left).max(to);
                match self.ends {
                    Some(_) => self.ends = Some(now + to),
                    None => self.left = to,
                }
                true
            }
            _ => false,
        }
    }

    /// Skips the block on show: a focus block to its break, a break (or a
    /// block waiting) on to the next, started now.
    pub fn skip(&mut self, plan: &Plan, now: f64) -> Option<Happened> {
        match self.phase {
            Phase::Ready | Phase::Busy => None,
            Phase::Focus if !self.waiting => {
                let next = self.break_after(plan);
                self.run(next, plan.seconds(next, 0), now);
                Some(Happened::BreakStarted)
            }
            _ => {
                if self.phase == Phase::LongBreak {
                    self.done = 0;
                }
                Some(self.start(plan, now))
            }
        }
    }

    /// The break due after the focus block on show: the long one at the
    /// end of a round.
    fn break_after(&self, plan: &Plan) -> Phase {
        if (self.done + 1).is_multiple_of(plan.every.max(1)) { Phase::LongBreak } else { Phase::ShortBreak }
    }

    /// Back to ready, the round started again.
    pub fn stop(&mut self) -> Option<Happened> {
        if self.phase == Phase::Ready {
            return None;
        }
        let was_busy = self.phase == Phase::Busy;
        self.phase = Phase::Ready;
        self.ends = None;
        self.left = 0.0;
        self.length = 0.0;
        self.waiting = false;
        self.done = 0;
        self.since = None;
        self.auto = false;
        self.held = None;
        Some(if was_busy { Happened::BusyEnded } else { Happened::Stopped })
    }

    /// Busy from now: a block running pauses, kept to carry on after.
    /// `auto`: Home Assistant's entities started it (they end it too); one
    /// started by hand stays until ended by hand. `status`: what it says
    /// it is ("In a call"), if anything.
    pub fn start_busy(&mut self, auto: bool, status: &str, now: f64) -> Option<Happened> {
        if self.phase == Phase::Busy {
            // Asked by hand while busy by itself: it stays until ended by hand.
            self.auto &= auto;
            if !status.is_empty() {
                self.status = status.to_owned();
            }
            return None;
        }
        self.held = (self.phase != Phase::Ready).then(|| Held {
            phase: self.phase,
            left: self.left_at(now),
            length: self.length,
            running: self.running(),
            waiting: self.waiting,
        });
        self.phase = Phase::Busy;
        self.ends = None;
        self.left = 0.0;
        self.length = 0.0;
        self.waiting = false;
        self.since = Some(now);
        self.auto = auto;
        self.status = status.to_owned();
        Some(Happened::BusyStarted)
    }

    /// Busy ends: what it paused carries on as it was. `auto`: Home
    /// Assistant's entities have gone quiet, which ends only busy they
    /// started.
    pub fn end_busy(&mut self, auto: bool, now: f64) -> Option<Happened> {
        if self.phase != Phase::Busy || (auto && !self.auto) {
            return None;
        }
        self.since = None;
        self.auto = false;
        self.status.clear();
        match self.held.take() {
            Some(held) => {
                self.phase = held.phase;
                self.length = held.length;
                self.left = held.left;
                self.waiting = held.waiting;
                self.ends = held.running.then_some(now + held.left);
            }
            None => {
                self.phase = Phase::Ready;
                self.ends = None;
                self.left = 0.0;
                self.length = 0.0;
                self.waiting = false;
            }
        }
        Some(Happened::BusyEnded)
    }

    /// The clock moved on to `now`: a block that has run out ends, and the
    /// next one waits for a hold (or starts, straight on). What happened,
    /// and whether it ran out late (LATE).
    pub fn tick(&mut self, plan: &Plan, now: f64) -> Option<(Happened, bool)> {
        let ends = self.ends.filter(|ends| now >= *ends)?;
        let late = now - ends > LATE;
        let (happened, next, seconds) = match self.phase {
            Phase::Focus => {
                let next = self.break_after(plan);
                self.done += 1;
                self.today += 1;
                (Happened::FocusDone, next, plan.seconds(next, 0))
            }
            Phase::ShortBreak | Phase::LongBreak => {
                if self.phase == Phase::LongBreak {
                    self.done = 0;
                }
                (Happened::BreakDone, Phase::Focus, plan.seconds(Phase::Focus, self.focus_minutes(plan)))
            }
            Phase::Ready | Phase::Busy => {
                self.ends = None;
                return None;
            }
        };
        if plan.straight_on {
            // Started as the last ran out, so nothing is lost to a late tick.
            self.run(next, seconds, ends.max(now - seconds + 1.0));
        } else {
            self.phase = next;
            self.length = seconds;
            self.left = seconds;
            self.ends = None;
            self.waiting = true;
        }
        Some((happened, late))
    }

    /// What Home Assistant reads (<base>/focus, retained).
    pub fn state(&self, plan: &Plan, now: f64) -> Value {
        let state = match self.phase {
            Phase::Ready => "ready",
            Phase::Busy => "busy",
            _ if self.waiting => "waiting",
            _ if self.running() => self.phase.name(),
            _ => "paused",
        };
        json!({
            "state": state,
            "phase": self.phase.name(),
            "running": self.running(),
            "waiting": self.waiting,
            "ends": self.ends,
            "left": self.left_at(now).round(),
            "length": self.length.round(),
            "block": self.block(),
            "blocks": plan.every,
            "today": self.today,
            "busy": self.busy(),
            "since": self.since,
            "auto": self.auto,
            "status": self.status,
            "focus_minutes": self.focus_minutes(plan),
        })
    }
}

/// Home Assistant's command (<base>/set/focus): {"command": "start", ...}.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Order {
    pub command: String,
    /// start: a focus block of this many minutes; length: the next one's.
    #[serde(default)]
    pub minutes: Option<u32>,
    /// busy_on, busy_off: from Home Assistant's busy entities (busy they
    /// start, they end).
    #[serde(default)]
    pub auto: bool,
    /// busy_on: what it says it is.
    #[serde(default)]
    pub status: String,
}

impl Order {
    pub fn parse(payload: &[u8]) -> Result<Order, String> {
        let text = String::from_utf8_lossy(payload);
        let text = text.trim();
        if !text.starts_with('{') {
            // A bare word: "start", "pause".
            return Ok(Order { command: text.to_ascii_lowercase(), ..Default::default() });
        }
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// Does it. What happened, if anything; `changed` says whether the
    /// state changed without anything to tell (a length set).
    pub fn apply(&self, focus: &mut Focus, plan: &Plan, now: f64) -> Result<(Option<Happened>, bool), String> {
        let before = focus.clone();
        let happened = match self.command.as_str() {
            "start" => {
                if let Some(minutes) = self.minutes {
                    focus.chosen = minutes.clamp(1, 600);
                }
                match focus.phase {
                    Phase::Busy => {
                        focus.end_busy(false, now);
                        focus.stop();
                        Some(focus.start(plan, now))
                    }
                    _ if focus.waiting => focus.hold(plan, now),
                    Phase::Ready => Some(focus.start(plan, now)),
                    _ if self.minutes.is_some() => Some(focus.start(plan, now)),
                    _ => focus.resume(now),
                }
            }
            "pause" => focus.pause(now),
            "resume" => focus.resume(now),
            "toggle" => focus.hold(plan, now),
            "skip" => focus.skip(plan, now),
            "stop" => focus.stop(),
            "busy_on" => focus.start_busy(self.auto, &self.status, now),
            "busy_off" => focus.end_busy(self.auto, now),
            "length" => {
                let minutes = self.minutes.ok_or("length needs minutes")?;
                focus.chosen = if minutes == plan.focus { 0 } else { minutes.clamp(1, 600) };
                None
            }
            other => return Err(format!("{other:?}: expected start, pause, resume, toggle, skip, stop, busy_on, busy_off or length")),
        };
        Ok((happened, *focus != before))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: f64 = 1_800_000_000.0;

    fn plan() -> Plan {
        Plan::default()
    }

    #[test]
    fn a_hold_starts_pauses_and_carries_on_a_focus_block() {
        let (mut f, p) = (Focus::default(), plan());
        assert_eq!(f.hold(&p, T), Some(Happened::FocusStarted));
        assert_eq!(f.left_at(T + 60.0), 24.0 * 60.0);
        assert!(f.busy(), "focusing is busy");
        assert_eq!(f.hold(&p, T + 60.0), Some(Happened::Paused));
        assert!(f.paused() && !f.busy(), "paused, not busy");
        assert_eq!(f.left_at(T + 600.0), 24.0 * 60.0, "a paused block keeps its time");
        assert_eq!(f.hold(&p, T + 600.0), Some(Happened::Resumed));
        assert_eq!(f.ends, Some(T + 600.0 + 24.0 * 60.0));
    }

    #[test]
    fn a_block_that_runs_out_waits_for_a_hold_and_every_fourth_break_is_long() {
        let (mut f, p) = (Focus::default(), plan());
        let mut now = T;
        for round in 1..=4 {
            f.hold(&p, now);
            now += 25.0 * 60.0;
            assert_eq!(f.tick(&p, now - 1.0), None, "not yet");
            assert_eq!(f.tick(&p, now), Some((Happened::FocusDone, false)));
            let long = round == 4;
            assert_eq!(f.phase, if long { Phase::LongBreak } else { Phase::ShortBreak });
            assert!(f.waiting && !f.running(), "the break waits for a hold");
            assert_eq!(f.left, if long { 15.0 } else { 5.0 } * 60.0);
            assert_eq!(f.hold(&p, now + 30.0), Some(Happened::BreakStarted));
            now += 30.0 + f.length;
            assert_eq!(f.tick(&p, now), Some((Happened::BreakDone, false)));
            assert_eq!((f.phase, f.waiting), (Phase::Focus, true), "the next focus block waits too");
        }
        assert_eq!((f.done, f.today), (0, 4), "the long break starts the round again; today counts on");
    }

    #[test]
    fn straight_on_starts_the_next_block_as_the_last_runs_out() {
        let mut f = Focus::default();
        let p = Plan { straight_on: true, ..plan() };
        f.hold(&p, T);
        assert_eq!(f.tick(&p, T + 1500.0), Some((Happened::FocusDone, false)));
        assert_eq!(f.phase, Phase::ShortBreak);
        assert_eq!(f.ends, Some(T + 1500.0 + 300.0), "from when the last ran out");
    }

    #[test]
    fn a_block_long_run_out_ends_quietly() {
        let (mut f, p) = (Focus::default(), plan());
        f.hold(&p, T);
        assert_eq!(f.tick(&p, T + 1500.0 + LATE + 1.0), Some((Happened::FocusDone, true)));
    }

    #[test]
    fn the_ring_sets_the_length_while_ready_and_a_minute_a_step_while_it_runs() {
        let (mut f, p) = (Focus::default(), plan());
        assert!(f.turn(&p, 1, T));
        assert_eq!(f.focus_minutes(&p), 30);
        f.turn(&p, -100, T);
        assert_eq!(f.focus_minutes(&p), LENGTH_MIN);
        f.turn(&p, 4, T);
        assert_eq!((f.focus_minutes(&p), f.chosen), (25, 0), "back on the card's: it follows the card again");
        f.hold(&p, T);
        assert!(f.turn(&p, 2, T));
        assert_eq!(f.left_at(T), 27.0 * 60.0);
        assert_eq!(f.fraction(T), 1.0, "never more than full");
        f.turn(&p, -100, T);
        assert_eq!(f.left_at(T), LEAST_LEFT, "never down to nothing");
        f.pause(T);
        assert!(f.turn(&p, 1, T));
        assert_eq!(f.left, LEAST_LEFT + 60.0, "paused too");
    }

    #[test]
    fn skip_goes_to_the_break_and_on_and_stop_starts_the_round_again() {
        let (mut f, p) = (Focus::default(), plan());
        f.hold(&p, T);
        assert_eq!(f.skip(&p, T + 10.0), Some(Happened::BreakStarted));
        assert_eq!((f.phase, f.done), (Phase::ShortBreak, 0), "a skipped block isn't counted");
        assert_eq!(f.skip(&p, T + 20.0), Some(Happened::FocusStarted));
        assert_eq!(f.stop(), Some(Happened::Stopped));
        assert_eq!(f.phase, Phase::Ready);
        assert_eq!(f.stop(), None);
    }

    #[test]
    fn busy_pauses_a_running_block_which_carries_on_after() {
        let (mut f, p) = (Focus::default(), plan());
        f.hold(&p, T);
        assert_eq!(f.start_busy(true, "In a call", T + 300.0), Some(Happened::BusyStarted));
        assert!(f.busy() && f.phase == Phase::Busy);
        assert_eq!(f.end_busy(true, T + 900.0), Some(Happened::BusyEnded));
        assert_eq!(f.phase, Phase::Focus);
        assert_eq!(f.ends, Some(T + 900.0 + 20.0 * 60.0), "the 20 minutes it had left");
        // Busy by hand isn't ended by Home Assistant's entities going quiet.
        f.start_busy(false, "", T + 1000.0);
        assert_eq!(f.end_busy(true, T + 1100.0), None);
        f.start_busy(true, "", T + 1100.0);
        assert!(!f.auto, "asked by itself while busy by hand: still by hand");
        assert_eq!(f.hold(&p, T + 1200.0), Some(Happened::BusyEnded), "a hold ends it");
        assert!(f.running());
    }

    #[test]
    fn busy_from_ready_goes_back_to_ready() {
        let (mut f, p) = (Focus::default(), plan());
        f.start_busy(false, "", T);
        assert_eq!(f.state(&p, T + 5.0)["state"], "busy");
        f.end_busy(false, T + 5.0);
        assert_eq!(f.phase, Phase::Ready);
    }

    #[test]
    fn a_new_day_counts_from_nothing() {
        let mut f = Focus { today: 3, day: 10, ..Default::default() };
        assert!(!f.new_day(10));
        assert!(f.new_day(11));
        assert_eq!(f.today, 0);
    }

    #[test]
    fn home_assistant_s_orders() {
        let (mut f, p) = (Focus::default(), plan());
        let order = Order::parse(br#"{"command":"start","minutes":50}"#).unwrap();
        assert_eq!(order.apply(&mut f, &p, T).unwrap(), (Some(Happened::FocusStarted), true));
        assert_eq!(f.length, 50.0 * 60.0);
        assert_eq!(Order::parse(b"pause").unwrap().apply(&mut f, &p, T).unwrap().0, Some(Happened::Paused));
        assert!(Order::parse(b"explode").unwrap().apply(&mut f, &p, T).is_err());
        let state = f.state(&p, T);
        assert_eq!((state["state"].as_str(), state["block"].as_u64(), state["busy"].as_bool()), (Some("paused"), Some(1), Some(false)));
    }

    #[test]
    fn the_timer_is_kept_as_it_was() {
        let (mut f, p) = (Focus::default(), plan());
        f.hold(&p, T);
        let kept: Focus = serde_json::from_slice(&serde_json::to_vec(&f).unwrap()).unwrap();
        assert_eq!(kept, f);
        let old: Focus = serde_json::from_str(r#"{"phase":"focus"}"#).unwrap();
        assert_eq!(old.phase, Phase::Focus, "missing fields take their defaults");
    }
}
