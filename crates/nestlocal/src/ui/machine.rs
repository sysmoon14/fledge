//! What the ring and button do, as a state machine apart from the unit: it
//! takes turns, presses and time, and says what should happen (Effect).

use std::time::{Duration, Instant};

use super::model::{Card, Cards, Command};
use crate::logf;

/// Ring counts per step, by what a step moves: a card, a list row, a value.
/// REL_DIAL reports the ADC's raw counts; a step is a deliberate movement.
pub const CARD_STEP: i32 = 500;
pub const ROW_STEP: i32 = 250;
/// A percent of brightness or volume; a thermostat's step (half a degree).
pub const VALUE_STEP: i32 = 30;
pub const CLIMATE_STEP: i32 = 90;
/// A pause this long ends a movement (for the log); this long, its counts
/// towards the next step.
const GESTURE: Duration = Duration::from_millis(400);
const TURN_KEPT: Duration = Duration::from_millis(1500);
/// How long each view waits, untouched, before going back a level.
pub const CONTROL_IDLE: Duration = Duration::from_secs(8);
pub const LIST_IDLE: Duration = Duration::from_secs(12);
pub const CARD_IDLE: Duration = Duration::from_secs(20);
/// Back to the cards this long after the last input on nlclient's screens,
/// once nlclient shows its home dial again.
pub const NATIVE_IDLE: Duration = Duration::from_secs(5);
/// How often the unit is held awake again while a card shows.
pub const KEEP: Duration = Duration::from_secs(4);
/// The least time between commands while turning.
pub const SEND_EVERY: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    Thermostat,
    Card(usize),
    Settings,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// No cards, or no nlclient: everything as stock.
    Idle,
    /// nlclient's screen with the ring held: the thermostat card.
    Home,
    /// nlclient's screen and ring: its temperature, menus and settings.
    Native,
    /// One of the deck's own cards, at its index.
    Card(usize),
    LightList { slot: usize, selected: usize },
    LightDim { slot: usize, light: usize, on: bool, value: u8 },
    ClimateSet { slot: usize, target: f64 },
    Volume { slot: usize, value: u8 },
}

impl Mode {
    /// Whether the UI owns the display in this mode.
    pub fn owning(&self) -> bool {
        !matches!(self, Mode::Idle | Mode::Home | Mode::Native)
    }

    fn holding(&self) -> bool {
        !matches!(self, Mode::Idle | Mode::Native)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// Ring counts, clockwise positive.
    Turn(i32),
    Press,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// Take the display, keeping nlclient's frame, or give it back with it.
    Own(bool),
    /// Hold the ring and button, or leave them to nlclient.
    Hold(bool),
    /// Asks nlclient to light its display (a turn while it was dark). While
    /// the UI owns the display, nlclient keeps it lit by itself.
    WakeDisplay,
    /// The Nest's click, for a step of the ring or a press.
    Click,
    /// Holds the unit awake while a card shows.
    KeepAwake,
    /// A press for nlclient, once it has its frame back.
    PressNlclient,
    Send(Command),
}

pub struct Machine {
    pub mode: Mode,
    pub cards: Cards,
    dirty: bool,
    last_input: Instant,
    /// When the unit was last held awake under a card; None: now.
    last_keep: Option<Instant>,
    turn: i32,
    movement: i32,
    pending: Option<Command>,
    last_sent: Instant,
}

impl Machine {
    pub fn new(now: Instant) -> Machine {
        Machine {
            mode: Mode::Idle, cards: Cards::default(), dirty: false, last_input: now, last_keep: None,
            turn: 0, movement: 0, pending: None, last_sent: now,
        }
    }

    /// The cards in turning order: the thermostat, Home Assistant's, settings.
    pub fn deck(&self) -> Vec<Slot> {
        let mut deck = vec![Slot::Thermostat];
        deck.extend((0..self.cards.cards.len()).map(Slot::Card));
        deck.push(Slot::Settings);
        deck
    }

    /// Whether the display needs drawing; clears it.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// New cards from Home Assistant: keeps the mode if what it shows exists.
    pub fn set_cards(&mut self, cards: Cards, out: &mut Vec<Effect>) {
        self.cards = cards;
        let count = self.cards.cards.len();
        let fine = match &self.mode {
            Mode::Card(index) => *index < count + 2,
            Mode::LightList { slot, .. } => matches!(self.card(*slot), Some(Card::Lights(_))),
            Mode::ClimateSet { slot, .. } => matches!(self.card(*slot), Some(Card::Climate(_))),
            Mode::Volume { slot, .. } => matches!(self.card(*slot), Some(Card::Media(_))),
            Mode::LightDim { slot, light, .. } => match self.card(*slot) {
                Some(Card::Lights(card)) => *light < card.lights.len(),
                _ => false,
            },
            _ => true,
        };
        if !fine {
            self.go(Mode::Home, out);
        }
        self.dirty = true;
    }

    /// Whether the cards should run: there are some, and nlclient is up.
    pub fn set_active(&mut self, nlclient: bool, now: Instant, out: &mut Vec<Effect>) {
        let active = nlclient && !self.cards.cards.is_empty();
        if !active && self.mode != Mode::Idle {
            logf!("ui: {}", if nlclient { "no cards: the thermostat as it was" } else { "nlclient stopped: standing aside" });
            self.go(Mode::Idle, out);
        } else if active && self.mode == Mode::Idle {
            logf!("ui: {} home screen cards", self.cards.cards.len());
            // nlclient may be anywhere; the cards start from its home dial.
            self.go(Mode::Native, out);
            self.last_input = now - NATIVE_IDLE;
        }
    }

    fn card(&self, slot: usize) -> Option<&Card> {
        self.cards.cards.get(slot)
    }

    /// Moves to `mode`, handing the display and the ring over as it needs.
    fn go(&mut self, mode: Mode, out: &mut Vec<Effect>) {
        let (was, will) = (self.mode.owning(), mode.owning());
        let (held, hold) = (self.mode.holding(), mode.holding());
        if will != was || hold != held {
            logf!("ui: {} the ring, {} the display", if hold { "holds" } else { "leaves nlclient" },
                  if will { "owns" } else { "nlclient has" });
        }
        if will != was {
            out.push(Effect::Own(will));
        }
        if hold != held {
            out.push(Effect::Hold(hold));
        }
        if will && !was {
            self.last_keep = None;
        }
        if let Some(command) = self.pending.take() {
            out.push(Effect::Send(command));
        }
        self.mode = mode;
        self.dirty = true;
    }

    pub fn input(&mut self, input: Input, now: Instant, lit: bool, out: &mut Vec<Effect>) {
        let pause = now.duration_since(self.last_input);
        self.last_input = now;
        match self.mode {
            Mode::Idle | Mode::Native => return,
            Mode::Home if !lit => {
                // Dark: the turn or press only wakes nlclient's display.
                out.push(Effect::WakeDisplay);
                return;
            }
            _ => {}
        }
        match input {
            Input::Press => self.press(out),
            Input::Turn(0) => {}
            Input::Turn(value) => {
                if pause > GESTURE && self.movement != 0 {
                    logf!("ui: turned {} counts", self.movement);
                    self.movement = 0;
                }
                // A slow turn keeps what it has, until it stops or reverses.
                if pause > TURN_KEPT || (self.turn != 0 && value.signum() != self.turn.signum()) {
                    self.turn = 0;
                }
                self.turn += value;
                self.movement += value;
                let step = match self.mode {
                    Mode::Home | Mode::Card(_) => CARD_STEP,
                    Mode::LightList { .. } => ROW_STEP,
                    Mode::ClimateSet { .. } => CLIMATE_STEP,
                    _ => VALUE_STEP,
                };
                let steps = self.turn / step;
                if steps != 0 {
                    self.turn -= steps * step;
                    self.turned(steps, out);
                }
            }
        }
    }

    fn turned(&mut self, steps: i32, out: &mut Vec<Effect>) {
        out.push(Effect::Click);
        let deck = self.deck().len() as i32;
        match self.mode.clone() {
            Mode::Home | Mode::Card(_) => {
                let from = if let Mode::Card(index) = self.mode { index as i32 } else { 0 };
                let index = (from + steps).rem_euclid(deck) as usize;
                self.go(if index == 0 { Mode::Home } else { Mode::Card(index) }, out);
            }
            Mode::LightList { slot, selected } => {
                if let Some(Card::Lights(card)) = self.card(slot) {
                    let rows = card.lights.len() as i32 + 1;
                    let selected = (selected as i32 + steps).clamp(0, rows - 1) as usize;
                    self.mode = Mode::LightList { slot, selected };
                }
            }
            Mode::LightDim { slot, light, on, value } => {
                let start = if on { value as i32 } else { 0 };
                let value = (start + steps).clamp(1, 100) as u8;
                if let Some(Card::Lights(card)) = self.card(slot) {
                    self.pending = Some(Command::Brightness(card.lights[light].id.clone(), value));
                }
                self.mode = Mode::LightDim { slot, light, on: true, value };
            }
            Mode::ClimateSet { slot, target } => {
                if let Some(Card::Climate(card)) = self.card(slot) {
                    let target = (target + steps as f64 * card.step).clamp(card.min, card.max);
                    self.pending = Some(Command::Temperature(card.id.clone(), target));
                    self.mode = Mode::ClimateSet { slot, target };
                }
            }
            Mode::Volume { slot, value } => {
                if let Some(Card::Media(card)) = self.card(slot) {
                    let value = (value as i32 + steps).clamp(0, 100) as u8;
                    self.pending = Some(Command::Volume(card.id.clone(), value));
                    self.mode = Mode::Volume { slot, value };
                }
            }
            Mode::Idle | Mode::Native => {}
        }
        self.dirty = true;
    }

    fn press(&mut self, out: &mut Vec<Effect>) {
        let deck = self.deck();
        // Nest settings passes the press on to nlclient, which clicks itself.
        if !matches!(self.mode, Mode::Card(index) if deck.get(index) == Some(&Slot::Settings)) {
            out.push(Effect::Click);
        }
        match self.mode.clone() {
            Mode::Home => self.go(Mode::Native, out),
            Mode::Card(index) => match deck.get(index) {
                Some(Slot::Settings) => {
                    self.go(Mode::Native, out);
                    // nlclient is on its home dial: a press opens its menu.
                    out.push(Effect::PressNlclient);
                }
                Some(&Slot::Card(slot)) => match self.card(slot) {
                    Some(Card::Lights(_)) => self.mode = Mode::LightList { slot, selected: 0 },
                    Some(Card::Climate(card)) => {
                        let target = card.target.unwrap_or((card.min + card.max) / 2.0);
                        self.mode = Mode::ClimateSet { slot, target };
                    }
                    Some(Card::Media(card)) => self.mode = Mode::Volume { slot, value: card.volume.unwrap_or(30) },
                    None => {}
                },
                _ => {}
            },
            Mode::LightList { slot, selected } => {
                let Some(Card::Lights(card)) = self.card(slot) else { return };
                match card.lights.get(selected).cloned() {
                    None => self.mode = Mode::Card(slot + 1),
                    Some(light) if light.dimmable => {
                        self.mode = Mode::LightDim { slot, light: selected, on: light.on,
                                                     value: light.brightness.unwrap_or(100) };
                    }
                    Some(light) => self.switch_light(slot, selected, !light.on, out),
                }
            }
            Mode::LightDim { slot, light, on, value } => {
                self.switch_light(slot, light, !on, out);
                self.mode = Mode::LightDim { slot, light, on: !on, value };
            }
            Mode::ClimateSet { slot, .. } => {
                if let Some(command) = self.pending.take() {
                    out.push(Effect::Send(command));
                }
                self.mode = Mode::Card(slot + 1);
            }
            Mode::Volume { slot, .. } => {
                if let Some(Card::Media(card)) = self.cards.cards.get_mut(slot) {
                    out.push(Effect::Send(Command::PlayPause(card.id.clone())));
                    card.state = if card.state == "playing" { "paused".into() } else { "playing".into() };
                }
            }
            Mode::Idle | Mode::Native => {}
        }
        self.dirty = true;
    }

    /// Switches a light, showing it switched until Home Assistant says.
    fn switch_light(&mut self, slot: usize, index: usize, on: bool, out: &mut Vec<Effect>) {
        self.pending = None;
        if let Some(Card::Lights(card)) = self.cards.cards.get_mut(slot) {
            let light = &mut card.lights[index];
            light.on = on;
            out.push(Effect::Send(if on { Command::On(light.id.clone()) } else { Command::Off(light.id.clone()) }));
        }
    }

    /// Time passing: going back when left alone, keeping nlclient's display
    /// awake under a card, sending what turning asked for. `home_dial` says
    /// whether nlclient shows its home dial (only asked for when it matters).
    pub fn tick(&mut self, now: Instant, lit: bool, home_dial: &mut dyn FnMut() -> bool, out: &mut Vec<Effect>) {
        let idle = now.duration_since(self.last_input);
        match self.mode.clone() {
            Mode::Native => {
                // Back to the cards once nlclient is home: dark, or its dial.
                if !lit || (idle >= NATIVE_IDLE && home_dial()) {
                    self.go(Mode::Home, out);
                }
                return;
            }
            Mode::Idle | Mode::Home => return,
            _ => {}
        }
        if !lit {
            // Something else put the display out: give it back.
            self.go(Mode::Home, out);
            return;
        }
        if self.last_keep.is_none_or(|last| now.duration_since(last) >= KEEP) {
            self.last_keep = Some(now);
            out.push(Effect::KeepAwake);
        }
        let back = match self.mode {
            Mode::Card(_) if idle >= CARD_IDLE => Some(Mode::Home),
            Mode::LightList { slot, .. } if idle >= LIST_IDLE => Some(Mode::Card(slot + 1)),
            Mode::LightDim { slot, light, .. } if idle >= CONTROL_IDLE => Some(Mode::LightList { slot, selected: light }),
            Mode::ClimateSet { slot, .. } | Mode::Volume { slot, .. } if idle >= CONTROL_IDLE => Some(Mode::Card(slot + 1)),
            _ => None,
        };
        if let Some(mode) = back {
            self.last_input = now;
            self.go(mode, out);
        }
        if self.pending.is_some() && now.duration_since(self.last_sent) >= SEND_EVERY {
            self.last_sent = now;
            out.push(Effect::Send(self.pending.take().unwrap()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn cards() -> Cards {
        Cards::parse(br#"{"cards":[
            {"kind":"lights","name":"Bedroom","icon":"","lights":[
                {"id":"light.lamp","name":"Lamp","on":true,"dimmable":true,"brightness":60},
                {"id":"switch.fan","name":"Fan","on":false,"dimmable":false}]},
            {"kind":"climate","id":"climate.kitchen","name":"Kitchen","target":19,"min":7,"max":30,"step":0.5},
            {"kind":"media","id":"media_player.speaker","name":"Speaker","state":"playing","volume":30}]}"#).unwrap()
    }

    /// A machine with cards, on the thermostat card with the ring held.
    struct Rig {
        machine: Machine,
        now: Instant,
        lit: bool,
        dial: bool,
    }

    impl Rig {
        fn new() -> Rig {
            let now = Instant::now();
            let mut machine = Machine::new(now);
            let mut out = vec![];
            machine.set_cards(cards(), &mut out);
            machine.set_active(true, now, &mut out);
            let mut rig = Rig { machine, now, lit: true, dial: true };
            rig.tick();
            assert_eq!(rig.machine.mode, Mode::Home);
            rig
        }

        fn wait(&mut self, time: Duration) -> Vec<Effect> {
            let mut out = vec![];
            let end = self.now + time;
            while self.now < end {
                self.now += Duration::from_millis(50).min(end - self.now);
                out.extend(self.tick());
            }
            out
        }

        fn tick(&mut self) -> Vec<Effect> {
            let mut out = vec![];
            let dial = self.dial;
            self.machine.tick(self.now, self.lit, &mut || dial, &mut out);
            out
        }

        /// What an input does, its clicks aside (clicks_on_each_step_and_press).
        fn input(&mut self, input: Input) -> Vec<Effect> {
            self.input_with_clicks(input).into_iter().filter(|e| *e != Effect::Click).collect()
        }

        fn input_with_clicks(&mut self, input: Input) -> Vec<Effect> {
            self.now += 20 * MS;
            let mut out = vec![];
            self.machine.input(input, self.now, self.lit, &mut out);
            out
        }

        fn turn(&mut self, counts: i32) -> Vec<Effect> {
            self.input(Input::Turn(counts))
        }

        fn press(&mut self) -> Vec<Effect> {
            self.input(Input::Press)
        }

        fn sends(effects: &[Effect]) -> Vec<Command> {
            effects.iter().filter_map(|e| if let Effect::Send(c) = e { Some(c.clone()) } else { None }).collect()
        }
    }

    #[test]
    fn without_cards_or_nlclient_nothing_is_held() {
        let now = Instant::now();
        let mut machine = Machine::new(now);
        let mut out = vec![];
        machine.set_active(true, now, &mut out);
        assert_eq!((machine.mode.clone(), out.len()), (Mode::Idle, 0), "no cards");
        machine.set_cards(cards(), &mut out);
        machine.set_active(true, now, &mut out);
        assert_eq!(machine.mode, Mode::Native, "nlclient may be anywhere at first");
        out.clear();
        machine.set_active(false, now, &mut out);
        assert_eq!(machine.mode, Mode::Idle);
        assert!(!out.contains(&Effect::Hold(true)));
    }

    #[test]
    fn starts_on_the_dial_once_nlclient_shows_it() {
        let now = Instant::now();
        let mut machine = Machine::new(now);
        let mut out = vec![];
        machine.set_cards(cards(), &mut out);
        machine.set_active(true, now, &mut out);
        out.clear();
        machine.tick(now, true, &mut || false, &mut out);
        assert_eq!(machine.mode, Mode::Native, "not while a menu shows");
        machine.tick(now, true, &mut || true, &mut out);
        assert_eq!((machine.mode, out), (Mode::Home, vec![Effect::Hold(true)]));
    }

    #[test]
    fn turning_moves_through_the_cards_and_back_to_the_thermostat() {
        let mut rig = Rig::new();
        assert_eq!(rig.turn(CARD_STEP), vec![Effect::Own(true)]);
        assert_eq!(rig.machine.mode, Mode::Card(1));
        for index in 2..=4 {
            assert!(rig.turn(CARD_STEP).is_empty());
            assert_eq!(rig.machine.mode, Mode::Card(index));
        }
        assert_eq!(rig.turn(CARD_STEP), vec![Effect::Own(false)], "past settings: the thermostat");
        assert_eq!(rig.machine.mode, Mode::Home);
        rig.turn(-CARD_STEP);
        assert_eq!(rig.machine.mode, Mode::Card(4), "backwards from the thermostat: settings");
    }

    #[test]
    fn a_slow_turn_adds_up_and_a_reversal_starts_again() {
        let mut rig = Rig::new();
        for _ in 0..4 {
            rig.turn(CARD_STEP / 5);
            rig.wait(Duration::from_millis(600));
        }
        assert_eq!(rig.machine.mode, Mode::Home);
        rig.turn(CARD_STEP / 5);
        assert_eq!(rig.machine.mode, Mode::Card(1), "pauses under 1.5 s keep the turn");
        rig.turn(CARD_STEP * 4 / 5);
        rig.turn(-CARD_STEP * 4 / 5);
        assert_eq!(rig.machine.mode, Mode::Card(1), "a reversal drops what was turned");
    }

    #[test]
    fn clicks_on_each_step_and_press() {
        let mut rig = Rig::new();
        let clicks = |effects: Vec<Effect>| effects.iter().filter(|e| **e == Effect::Click).count();
        assert_eq!(clicks(rig.input_with_clicks(Input::Turn(CARD_STEP / 2))), 0, "not between steps");
        assert_eq!(clicks(rig.input_with_clicks(Input::Turn(CARD_STEP / 2))), 1);
        assert_eq!(clicks(rig.input_with_clicks(Input::Press)), 1);
        let mut rig = Rig::new();
        rig.turn(-CARD_STEP);
        assert_eq!(rig.machine.mode, Mode::Card(4));
        assert_eq!(clicks(rig.input_with_clicks(Input::Press)), 0, "nlclient clicks for Nest settings");
        rig.lit = false;
        rig.tick();
        assert_eq!(clicks(rig.input_with_clicks(Input::Turn(CARD_STEP))), 0, "waking is silent, as nlclient's");
    }

    #[test]
    fn a_dark_display_only_wakes() {
        let mut rig = Rig::new();
        rig.lit = false;
        assert_eq!(rig.turn(CARD_STEP * 3), vec![Effect::WakeDisplay]);
        assert_eq!(rig.press(), vec![Effect::WakeDisplay]);
        assert_eq!(rig.machine.mode, Mode::Home);
    }

    #[test]
    fn a_press_on_the_thermostat_hands_nlclient_the_ring_until_it_is_home() {
        let mut rig = Rig::new();
        assert_eq!(rig.press(), vec![Effect::Hold(false)]);
        assert_eq!(rig.machine.mode, Mode::Native);
        assert!(rig.turn(CARD_STEP * 4).is_empty(), "nlclient's turn: the temperature");
        rig.dial = false;
        assert!(rig.wait(NATIVE_IDLE * 2).is_empty(), "not while nlclient shows something else");
        rig.dial = true;
        assert_eq!(rig.wait(Duration::from_millis(100)), vec![Effect::Hold(true)]);
        rig.press();
        rig.lit = false;
        assert_eq!(rig.tick(), vec![Effect::Hold(true)], "dark is home too");
    }

    #[test]
    fn nest_settings_opens_nlclients_menu() {
        let mut rig = Rig::new();
        rig.turn(-CARD_STEP);
        assert_eq!(rig.press(), vec![Effect::Own(false), Effect::Hold(false), Effect::PressNlclient]);
        assert_eq!(rig.machine.mode, Mode::Native);
    }

    #[test]
    fn a_card_keeps_nlclient_awake_and_goes_back_when_left() {
        let mut rig = Rig::new();
        rig.turn(CARD_STEP);
        let effects = rig.wait(CARD_IDLE - Duration::from_millis(100));
        let keeps = effects.iter().filter(|e| **e == Effect::KeepAwake).count();
        assert_eq!(keeps, 5, "at once, then every 4 s");
        assert!(!effects.iter().any(|e| matches!(e, Effect::WakeDisplay)), "nlclient keeps its display lit itself");
        assert_eq!(rig.wait(Duration::from_millis(200)), vec![Effect::Own(false)]);
        assert_eq!(rig.machine.mode, Mode::Home);
    }

    #[test]
    fn a_dark_display_under_a_card_goes_back_to_nlclient() {
        let mut rig = Rig::new();
        rig.turn(CARD_STEP);
        rig.lit = false;
        assert_eq!(rig.tick(), vec![Effect::Own(false)]);
        assert_eq!(rig.machine.mode, Mode::Home);
    }

    #[test]
    fn the_light_list_switches_dims_and_goes_back() {
        let mut rig = Rig::new();
        rig.turn(CARD_STEP);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::LightList { slot: 0, selected: 0 });
        rig.turn(ROW_STEP);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::On("switch.fan".into())], "on/off: switched in place");
        assert!(matches!(&rig.machine.cards.cards[0], Card::Lights(c) if c.lights[1].on), "shown on at once");
        rig.turn(ROW_STEP * 5);
        assert_eq!(rig.machine.mode, Mode::LightList { slot: 0, selected: 2 }, "the list stops at Back");
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Card(1));
        rig.press();
        rig.press();
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 0, light: 0, on: true, value: 60 });
        for _ in 0..5 {
            rig.turn(VALUE_STEP);
        }
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 0, light: 0, on: true, value: 65 });
        let sent = Rig::sends(&rig.wait(SEND_EVERY));
        assert_eq!(sent, vec![Command::Brightness("light.lamp".into(), 65)], "a turn's commands are thinned");
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Off("light.lamp".into())]);
        rig.wait(CONTROL_IDLE);
        assert_eq!(rig.machine.mode, Mode::LightList { slot: 0, selected: 0 });
        rig.wait(LIST_IDLE);
        assert_eq!(rig.machine.mode, Mode::Card(1));
    }

    #[test]
    fn a_turned_off_light_comes_on_from_low() {
        let mut rig = Rig::new();
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        rig.press();
        rig.turn(VALUE_STEP);
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 0, light: 0, on: true, value: 1 });
    }

    #[test]
    fn other_thermostats_and_media_are_set_with_the_ring() {
        let mut rig = Rig::new();
        rig.turn(CARD_STEP * 2);
        rig.press();
        rig.turn(CLIMATE_STEP * 3);
        assert_eq!(rig.machine.mode, Mode::ClimateSet { slot: 1, target: 20.5 });
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Temperature("climate.kitchen".into(), 20.5)],
                   "done sends what is still waiting");
        assert_eq!(rig.machine.mode, Mode::Card(2));
        rig.turn(CARD_STEP);
        rig.press();
        rig.turn(-VALUE_STEP * 5);
        assert_eq!(rig.machine.mode, Mode::Volume { slot: 2, value: 25 });
        let effects = rig.press();
        assert!(Rig::sends(&effects).contains(&Command::PlayPause("media_player.speaker".into())));
        assert!(matches!(&rig.machine.cards.cards[2], Card::Media(m) if m.state == "paused"));
    }

    #[test]
    fn cards_changing_underneath_go_back_to_the_thermostat_if_gone() {
        let mut rig = Rig::new();
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        let mut out = vec![];
        let mut fewer = cards();
        fewer.cards.remove(0);
        rig.machine.set_cards(fewer, &mut out);
        assert_eq!((rig.machine.mode.clone(), out), (Mode::Home, vec![Effect::Own(false)]));
        rig.turn(CARD_STEP * 2);
        let mut out = vec![];
        rig.machine.set_cards(cards(), &mut out);
        assert_eq!(rig.machine.mode, Mode::Card(2), "a card still there stays");
    }
}
