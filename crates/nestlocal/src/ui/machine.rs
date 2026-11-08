//! What the ring and button do, as a state machine apart from the unit: it
//! takes turns, presses and time, and says what should happen (Effect).
//!
//! One rule throughout: zoomed out on the cards (the chooser), the ring
//! moves between devices; opened, it works the device. Waking shows the
//! chooser on the thermostat, or what Home Assistant chose (Wake). A press
//! opens the card; inside, a press acts, or opens a ring of actions whose
//! first is Back. A long press goes back a level. Left alone, each level
//! goes back to the one above.

use std::time::{Duration, Instant};

use super::model::{Alert, Card, Cards, Command, Wake};
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
/// Between cards, a turn that stops this long settles: on the next card if
/// it went past halfway, else back where it was.
/// How often a camera full screen asks for a picture, and how long it
/// stays with nothing pressed.
pub const CAMERA_EVERY: Duration = Duration::from_secs(3);
pub const CAMERA_IDLE: Duration = Duration::from_secs(60);
/// How long a scene's card says it ran.
pub const RAN_SHOWN: Duration = Duration::from_secs(2);
/// A step of a light's warmth dial, in kelvin, and of its colour's, in degrees.
pub const KELVIN_STEP: i32 = 50;
pub const HUE_STEP: i32 = 4;
/// How far the cards follow a turn past either end, in counts.
pub const END_GIVE: i32 = CARD_STEP / 6;
pub const SNAP: Duration = Duration::from_millis(250);
/// How long each level waits, untouched, before going back to the one above.
pub const CONTROL_IDLE: Duration = Duration::from_secs(8);
pub const LIST_IDLE: Duration = Duration::from_secs(12);
pub const CARD_IDLE: Duration = Duration::from_secs(20);
/// Back to the thermostat this long after the last input on nlclient's
/// screens, once nlclient shows its home dial again.
pub const NATIVE_IDLE: Duration = Duration::from_secs(5);
/// How often the unit is held awake again while a card shows.
pub const KEEP: Duration = Duration::from_secs(4);
/// The least time between commands while turning.
pub const SEND_EVERY: Duration = Duration::from_millis(300);
/// How long a woken display has to light before dark means it went out.
pub const LIGHTING: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    Thermostat,
    Card(usize),
    Settings,
}

/// What an item of a device's ring of actions does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Back to the cards.
    Back,
    /// Switches the light on (true) or off.
    Switch(bool),
    /// Plays (true) or pauses the media player.
    Play(bool),
    /// A light's white: its own dial.
    Warmth,
    /// A light's colour: its own dial.
    Colour,
    /// A media player's next and previous tracks.
    Next,
    Previous,
    /// A thermostat's preset, by its place in its card's presets.
    Preset(u8),
    /// A light group's own lights, each to switch.
    Members,
    /// A cover opened or closed whole.
    Open,
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    /// No cards, or no nlclient: everything as stock.
    Idle,
    /// nlclient's screen with the ring held, the UI not showing: asleep, or
    /// lit by something other than the ring.
    Home,
    /// nlclient's screen and ring: the thermostat opened (its temperature,
    /// menus and settings).
    Native,
    /// The cards zoomed out, on the one at this place in the deck.
    Chooser(usize),
    LightList { slot: usize, selected: usize },
    LightDim { slot: usize, light: usize, on: bool, value: u8 },
    /// A light's white, in kelvin, and its colour, by hue: dials of their own.
    LightWarmth { slot: usize, light: usize, value: u16 },
    LightHue { slot: usize, light: usize, value: u16 },
    ClimateSet { slot: usize, target: f64 },
    Volume { slot: usize, value: u8 },
    /// A camera, full screen, its picture refreshed while it shows.
    Camera { slot: usize },
    /// A select's options, `selected` highlighted (its last row Back).
    OptionList { slot: usize, selected: usize },
    /// A light group's own lights, `selected` highlighted (its last row Back).
    Members { slot: usize, selected: usize },
    /// A cover, the ring setting how open (percent).
    CoverSet { slot: usize, value: u8 },
    /// A notification over `back`, which shows again once it goes.
    Alert { id: String, back: Box<Mode> },
    /// A device's ring of actions over `view`, `selected` chosen.
    Menu { view: Box<Mode>, actions: Vec<Action>, selected: usize },
}

impl Mode {
    /// Whether the UI owns the display in this mode.
    pub fn owning(&self) -> bool {
        !matches!(self, Mode::Idle | Mode::Home | Mode::Native)
    }

    fn holding(&self) -> bool {
        !matches!(self, Mode::Idle | Mode::Native)
    }

    /// The card a device's own screen belongs to, by its place in the deck.
    pub fn card_index(&self) -> Option<usize> {
        match self {
            Mode::LightList { slot, .. } | Mode::LightDim { slot, .. } | Mode::LightWarmth { slot, .. }
            | Mode::LightHue { slot, .. } | Mode::ClimateSet { slot, .. } | Mode::Camera { slot }
            | Mode::Volume { slot, .. } | Mode::OptionList { slot, .. } | Mode::Members { slot, .. }
            | Mode::CoverSet { slot, .. } => Some(slot + 1),
            Mode::Menu { view, .. } => view.card_index(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// Ring counts, clockwise positive.
    Turn(i32),
    Press,
    /// The button held (LONG_PRESS): back a level.
    LongPress,
    /// Back chosen in nlclient's own menu, which is now on its dial.
    NativeBack,
    /// nlclient's Settings, opened from the cards, left for its dial.
    SettingsDone,
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
    /// nlclient's Settings, from its menu: opened, Back checked as chosen,
    /// one step on to Settings, checked again, and pressed.
    OpenNestSettings,
    /// A notification's chime.
    Chime,
    Send(Command),
}

pub struct Machine {
    pub mode: Mode,
    pub cards: Cards,
    /// Set when a turn or press woke the display: what shows first appears
    /// at once, as the display lights.
    pub woke: bool,
    /// Set until the first input after waking to the cards: the hint.
    pub hint: bool,
    dirty: bool,
    last_input: Instant,
    /// When the unit was last held awake under a card; None: now.
    last_keep: Option<Instant>,
    turn: i32,
    movement: i32,
    pending: Option<Command>,
    last_sent: Instant,
    /// What was last said of the display and the ring (Effect::Own, Hold).
    owned: bool,
    held: bool,
    /// Until when the display, just woken, may still be dark.
    lighting: Option<Instant>,
    /// The notification showing, and until when.
    pub alert: Option<Alert>,
    alert_until: Option<Instant>,
    /// A scene or script just run from its card, and when: its card says
    /// so for a moment.
    pub ran: Option<(usize, Instant)>,
    /// When a camera showing full screen last asked for a picture.
    pictured: Option<Instant>,
    /// nlclient's Settings opened from the cards: on its dial again, the
    /// cards (SettingsDone), not the thermostat.
    pub settings_open: bool,
    /// Whether the display was lit at the last tick: lighting by itself
    /// (someone coming near) wakes to the cards as a turn would.
    was_lit: Option<bool>,
}

impl Machine {
    pub fn new(now: Instant) -> Machine {
        Machine {
            mode: Mode::Idle, cards: Cards::default(), woke: false, hint: false, dirty: false, last_input: now,
            last_keep: None, turn: 0, movement: 0, pending: None, last_sent: now, owned: false, held: false,
            lighting: None, was_lit: None, alert: None, alert_until: None, ran: None, pictured: None, settings_open: false,
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
        let fine = |mode: &Mode, cards: &Cards| match mode {
            Mode::Chooser(index) => *index < count + 2,
            Mode::LightList { slot, .. } => matches!(cards.cards.get(*slot), Some(Card::Lights(_))),
            Mode::ClimateSet { slot, .. } => matches!(cards.cards.get(*slot), Some(Card::Climate(_))),
            Mode::Volume { slot, .. } => matches!(cards.cards.get(*slot), Some(Card::Media(_))),
            Mode::Camera { slot } => matches!(cards.cards.get(*slot), Some(Card::Camera(_))),
            Mode::OptionList { slot, .. } => matches!(cards.cards.get(*slot), Some(Card::Options(_))),
            Mode::CoverSet { slot, .. } => matches!(cards.cards.get(*slot), Some(Card::Cover(_))),
            Mode::Members { slot, .. } => matches!(cards.cards.get(*slot), Some(Card::Lights(c)) if !c.members.is_empty()),
            Mode::LightDim { slot, light, .. } | Mode::LightWarmth { slot, light, .. }
            | Mode::LightHue { slot, light, .. } => match cards.cards.get(*slot) {
                Some(Card::Lights(card)) => *light < card.lights.len(),
                _ => false,
            },
            _ => true,
        };
        let shown = match &self.mode {
            Mode::Menu { view, .. } => view.as_ref(),
            mode => mode,
        };
        if !fine(shown, &self.cards) {
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

    /// Whether the UI shows on the display: its own screens, and the cards
    /// while a turn from the lit thermostat is under way.
    pub fn owns(&self) -> bool {
        self.mode.owning() || (self.mode == Mode::Home && self.turn != 0)
    }

    /// Where the cards are as the ring turns: the card's place in the deck,
    /// plus how far the turn has gone towards the next (-1..1). None off
    /// the cards.
    pub fn position(&self) -> Option<f32> {
        let partial = self.turn as f32 / CARD_STEP as f32;
        match self.mode {
            Mode::Home => Some(partial),
            Mode::Chooser(index) => Some(index as f32 + partial),
            _ => None,
        }
    }

    /// Moves to `mode`.
    fn go(&mut self, mode: Mode, out: &mut Vec<Effect>) {
        if let Some(command) = self.pending.take() {
            out.push(Effect::Send(command));
        }
        if !matches!(mode, Mode::Home | Mode::Chooser(_)) {
            self.turn = 0;
        }
        self.mode = mode;
        self.dirty = true;
        self.sync(out);
    }

    /// Hands the display and the ring over as what the UI shows now needs.
    fn sync(&mut self, out: &mut Vec<Effect>) {
        let (own, hold) = (self.owns(), self.mode.holding());
        if own != self.owned || hold != self.held {
            logf!("ui: {} the ring, {} the display", if hold { "holds" } else { "leaves nlclient" },
                  if own { "owns" } else { "nlclient has" });
        }
        if own != self.owned {
            out.push(Effect::Own(own));
            if own {
                self.last_keep = None;
            }
        }
        if hold != self.held {
            out.push(Effect::Hold(hold));
        }
        (self.owned, self.held) = (own, hold);
    }

    pub fn input(&mut self, input: Input, now: Instant, lit: bool, out: &mut Vec<Effect>) {
        let pause = now.duration_since(self.last_input);
        self.last_input = now;
        match (&self.mode, input) {
            (Mode::Idle, _) => return,
            // A notification: a press answers it, a long press puts it away.
            (Mode::Alert { id, .. }, Input::Press) => {
                let id = id.clone();
                out.push(Effect::Click);
                self.end_alert(Some(Command::AlertPressed(id)), out);
                return;
            }
            (Mode::Alert { id, .. }, Input::LongPress) => {
                let id = id.clone();
                out.push(Effect::Click);
                self.end_alert(Some(Command::AlertDismissed(id)), out);
                return;
            }
            (Mode::Alert { .. }, _) => return,
            // The thermostat opened: nlclient has the ring; its menu's Back,
            // or a long press, brings the cards back.
            (Mode::Native, Input::LongPress) => {
                out.push(Effect::Click);
                self.go(Mode::Chooser(0), out);
                return;
            }
            (Mode::Native, Input::NativeBack) => {
                self.go(Mode::Chooser(0), out);
                return;
            }
            (Mode::Native, Input::SettingsDone) => {
                self.settings_open = false;
                let last = self.deck().len() - 1;
                self.go(Mode::Chooser(last), out);
                return;
            }
            (_, Input::SettingsDone) => return,
            (_, Input::NativeBack) => return,
            (Mode::Native, _) => return,
            (Mode::Home, _) if !lit => {
                // Dark: the turn or press wakes the display, onto what
                // Home Assistant chose.
                out.push(Effect::WakeDisplay);
                self.lighting = Some(now + LIGHTING);
                self.wake(out);
                return;
            }
            _ => {}
        }
        self.hint = false;
        match input {
            Input::Press => self.press(out),
            Input::LongPress => self.back(out),
            Input::NativeBack | Input::SettingsDone => {}
            Input::Turn(0) => {}
            Input::Turn(value) => {
                if pause > GESTURE && self.movement != 0 {
                    logf!("ui: turned {} counts", self.movement);
                    self.movement = 0;
                }
                // A slow turn keeps what it has, until it stops or reverses;
                // between cards, which follow the hand, it runs on both ways.
                let cards = matches!(self.mode, Mode::Home | Mode::Chooser(_));
                let reversed = self.turn != 0 && value.signum() != self.turn.signum();
                if pause > TURN_KEPT || (reversed && !cards) {
                    self.turn = 0;
                }
                self.turn += value;
                if cards {
                    // At an end the cards give a little, then spring back.
                    let at = if let Mode::Chooser(index) = self.mode { index } else { 0 };
                    if at == 0 {
                        self.turn = self.turn.max(-END_GIVE);
                    }
                    if at + 1 == self.deck().len() {
                        self.turn = self.turn.min(END_GIVE);
                    }
                }
                self.movement += value;
                let step = match self.mode {
                    Mode::Home | Mode::Chooser(_) => CARD_STEP,
                    Mode::LightList { .. } | Mode::Menu { .. } | Mode::OptionList { .. } | Mode::Members { .. } => ROW_STEP,
                    Mode::ClimateSet { .. } => CLIMATE_STEP,
                    _ => VALUE_STEP,
                };
                let steps = self.turn / step;
                if steps != 0 {
                    self.turn -= steps * step;
                    self.turned(steps, out);
                }
                self.dirty = true;
                self.sync(out);
            }
        }
    }

    /// Home Assistant asks for the display: lit, if it isn't, and on what
    /// it wakes to, unless something already shows.
    pub fn wake_up(&mut self, now: Instant, lit: bool, out: &mut Vec<Effect>) {
        if self.mode == Mode::Idle {
            return;
        }
        if !lit {
            out.push(Effect::WakeDisplay);
            self.lighting = Some(now + LIGHTING);
        }
        if self.mode == Mode::Home {
            self.last_input = now;
            self.wake(out);
        }
    }

    /// The display lit: what Home Assistant chose shows, as it lights.
    fn wake(&mut self, out: &mut Vec<Effect>) {
        self.woke = true;
        match self.cards.wake() {
            Wake::Chooser => {
                self.hint = true;
                self.go(Mode::Chooser(0), out);
            }
            Wake::Thermostat => self.go(Mode::Native, out),
            // A scene or a camera isn't opened: that would run it.
            Wake::Card(slot) if matches!(self.card(slot), Some(Card::Action(_) | Card::Camera(_))) => {
                self.go(Mode::Chooser(slot + 1), out);
            }
            Wake::Card(slot) => self.open(slot + 1, out),
        }
    }

    /// A notification from Home Assistant: over whatever shows, the display
    /// woken for it, with a chime; one already showing gives way to it.
    pub fn show_alert(&mut self, alert: Alert, now: Instant, lit: bool, out: &mut Vec<Effect>) {
        if self.mode == Mode::Idle {
            return;
        }
        if alert.dismiss {
            if matches!(&self.mode, Mode::Alert { id, .. } if *id == alert.id) {
                self.end_alert(None, out);
            }
            return;
        }
        if !lit {
            out.push(Effect::WakeDisplay);
            self.lighting = Some(now + LIGHTING);
        }
        out.push(Effect::Chime);
        let back = match self.mode.clone() {
            Mode::Alert { back, .. } => back,
            Mode::Native | Mode::Home => Box::new(Mode::Home),
            mode => Box::new(mode),
        };
        self.alert_until = Some(now + Duration::from_secs_f64(alert.timeout.clamp(3.0, 3600.0)));
        self.last_input = now;
        let id = alert.id.clone();
        self.alert = Some(alert);
        self.go(Mode::Alert { id, back }, out);
    }

    /// The notification goes, saying why (`said`: pressed or dismissed; None
    /// when Home Assistant took it back), and what it covered shows again.
    fn end_alert(&mut self, said: Option<Command>, out: &mut Vec<Effect>) {
        let Mode::Alert { back, .. } = self.mode.clone() else { return };
        if let Some(command) = said {
            out.push(Effect::Send(command));
        }
        self.alert = None;
        self.alert_until = None;
        self.go(*back, out);
    }

    fn turned(&mut self, steps: i32, out: &mut Vec<Effect>) {
        let deck = self.deck().len() as i32;
        if let Mode::Home | Mode::Chooser(_) = self.mode {
            // The cards stop at either end, as Nest settings do.
            let from = if let Mode::Chooser(index) = self.mode { index as i32 } else { 0 };
            let index = (from + steps).clamp(0, deck - 1);
            if index != from {
                out.push(Effect::Click);
                self.go(Mode::Chooser(index as usize), out);
            }
            return;
        }
        out.push(Effect::Click);
        match self.mode.clone() {
            Mode::Home | Mode::Chooser(_) => {}
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
            Mode::LightWarmth { slot, light, value } => {
                if let Some(Card::Lights(card)) = self.card(slot) {
                    let one = &card.lights[light];
                    let [low, high] = one.kelvins.unwrap_or([2000, 6500]);
                    let value = (value as i32 + steps * KELVIN_STEP).clamp(low as i32, high as i32) as u16;
                    self.pending = Some(Command::Kelvin(one.id.clone(), value));
                    self.mode = Mode::LightWarmth { slot, light, value };
                }
            }
            Mode::LightHue { slot, light, value } => {
                if let Some(Card::Lights(card)) = self.card(slot) {
                    let value = (value as i32 + steps * HUE_STEP).rem_euclid(360) as u16;
                    self.pending = Some(Command::Hue(card.lights[light].id.clone(), value));
                    self.mode = Mode::LightHue { slot, light, value };
                }
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
            Mode::OptionList { slot, selected } => {
                if let Some(Card::Options(card)) = self.card(slot) {
                    let selected = (selected as i32 + steps).clamp(0, card.options.len() as i32) as usize;
                    self.mode = Mode::OptionList { slot, selected };
                }
            }
            Mode::Members { slot, selected } => {
                if let Some(Card::Lights(card)) = self.card(slot) {
                    let selected = (selected as i32 + steps).clamp(0, card.members.len() as i32) as usize;
                    self.mode = Mode::Members { slot, selected };
                }
            }
            Mode::CoverSet { slot, value } => {
                if let Some(Card::Cover(card)) = self.card(slot) {
                    if card.positionable {
                        let value = (value as i32 + steps).clamp(0, 100) as u8;
                        self.pending = Some(Command::Position(card.id.clone(), value));
                        self.mode = Mode::CoverSet { slot, value };
                    }
                }
            }
            Mode::Menu { view, actions, selected } => {
                let selected = (selected as i32 + steps).clamp(0, actions.len() as i32 - 1) as usize;
                self.mode = Mode::Menu { view, actions, selected };
            }
            Mode::Idle | Mode::Native | Mode::Alert { .. } | Mode::Camera { .. } => {}
        }
        self.dirty = true;
    }

    /// Opens the card at `index` in the deck: the thermostat hands nlclient
    /// the ring, Nest settings opens nlclient's menu, a device its controls.
    fn open(&mut self, index: usize, out: &mut Vec<Effect>) {
        match self.deck().get(index) {
            Some(Slot::Thermostat) => self.go(Mode::Native, out),
            Some(Slot::Settings) => {
                self.go(Mode::Native, out);
                // nlclient is on its home dial: straight to its Settings.
                self.settings_open = true;
                out.push(Effect::OpenNestSettings);
            }
            Some(&Slot::Card(slot)) => {
                let mode = match self.card(slot) {
                    // One light: its dimmer, or switched where it is.
                    Some(Card::Lights(card)) if card.lights.len() == 1 => {
                        let light = &card.lights[0];
                        if !light.dimmable {
                            let on = !light.on;
                            self.switch_light(slot, 0, on, out);
                            self.dirty = true;
                            return;
                        }
                        Mode::LightDim { slot, light: 0, on: light.on, value: light.brightness.unwrap_or(100) }
                    }
                    Some(Card::Lights(_)) => Mode::LightList { slot, selected: 0 },
                    Some(Card::Climate(card)) => {
                        Mode::ClimateSet { slot, target: card.target.unwrap_or((card.min + card.max) / 2.0) }
                    }
                    Some(Card::Media(card)) => Mode::Volume { slot, value: card.volume.unwrap_or(30) },
                    // A scene, script or button runs, staying on the cards.
                    Some(Card::Action(card)) => {
                        out.push(Effect::Send(Command::Run(card.id.clone())));
                        self.ran = Some((slot, self.last_input));
                        self.dirty = true;
                        return;
                    }
                    Some(Card::Camera(card)) => {
                        // Full screen, with a fresh picture on its way.
                        out.push(Effect::Send(Command::Snapshot(card.id.clone())));
                        self.pictured = Some(self.last_input);
                        Mode::Camera { slot }
                    }
                    Some(Card::Options(card)) => {
                        let at = card.current.as_ref().and_then(|c| card.options.iter().position(|o| o == c));
                        Mode::OptionList { slot, selected: at.unwrap_or(0) }
                    }
                    Some(Card::Cover(card)) => {
                        let open = if card.state == "open" { 100 } else { 0 };
                        Mode::CoverSet { slot, value: card.position.unwrap_or(open) }
                    }
                    Some(Card::Unknown) | None => return,
                };
                self.go(mode, out);
            }
            None => {}
        }
    }

    fn press(&mut self, out: &mut Vec<Effect>) {
        let deck = self.deck();
        // Nest settings passes the press on to nlclient, which clicks itself.
        if !matches!(self.mode, Mode::Chooser(index) if deck.get(index) == Some(&Slot::Settings)) {
            out.push(Effect::Click);
        }
        match self.mode.clone() {
            // Lit by something else, the thermostat showing: open it.
            Mode::Home => self.go(Mode::Native, out),
            Mode::Chooser(index) => self.open(index, out),
            Mode::LightList { slot, selected } => {
                let Some(Card::Lights(card)) = self.card(slot) else { return };
                match card.lights.get(selected).cloned() {
                    None => self.go(Mode::Chooser(slot + 1), out),
                    Some(light) if light.dimmable => {
                        self.mode = Mode::LightDim { slot, light: selected, on: light.on,
                                                     value: light.brightness.unwrap_or(100) };
                    }
                    Some(light) => self.switch_light(slot, selected, !light.on, out),
                }
            }
            view @ Mode::LightDim { slot, light, on, .. } => {
                let mut actions = vec![Action::Back, Action::Switch(!on)];
                if let Some(Card::Lights(card)) = self.card(slot) {
                    if card.lights[light].kelvins.is_some() {
                        actions.push(Action::Warmth);
                    }
                    if card.lights[light].hues {
                        actions.push(Action::Colour);
                    }
                    if !card.members.is_empty() {
                        actions.push(Action::Members);
                    }
                }
                self.mode = Mode::Menu { view: Box::new(view), actions, selected: 0 };
            }
            Mode::LightWarmth { slot, light, .. } | Mode::LightHue { slot, light, .. } => {
                // Done: back to its dimmer.
                self.mode = self.dimmer(slot, light);
            }
            view @ Mode::Volume { slot, .. } => {
                let playing = matches!(self.card(slot), Some(Card::Media(m)) if m.state == "playing");
                self.mode = Mode::Menu { view: Box::new(view),
                                         actions: vec![Action::Back, Action::Play(!playing), Action::Previous, Action::Next],
                                         selected: 0 };
            }
            view @ Mode::ClimateSet { slot, .. } => match self.card(slot) {
                // Its presets, where it has them; else done.
                Some(Card::Climate(card)) if !card.presets.is_empty() => {
                    let mut actions = vec![Action::Back];
                    actions.extend((0..card.presets.len().min(4)).map(|i| Action::Preset(i as u8)));
                    self.mode = Mode::Menu { view: Box::new(view), actions, selected: 0 };
                }
                _ => self.go(Mode::Chooser(slot + 1), out),
            },
            Mode::Camera { slot } => self.go(Mode::Chooser(slot + 1), out),
            Mode::OptionList { slot, selected } => {
                if let Some(Card::Options(card)) = self.cards.cards.get_mut(slot) {
                    if let Some(option) = card.options.get(selected).cloned() {
                        out.push(Effect::Send(Command::Select(card.id.clone(), option.clone())));
                        card.current = Some(option);
                    }
                }
                self.go(Mode::Chooser(slot + 1), out);
            }
            Mode::Members { slot, selected } => {
                let member = match self.cards.cards.get_mut(slot) {
                    Some(Card::Lights(card)) => card.members.get_mut(selected),
                    _ => None,
                };
                match member {
                    Some(light) => {
                        light.on = !light.on;
                        let id = light.id.clone();
                        out.push(Effect::Send(if light.on { Command::On(id) } else { Command::Off(id) }));
                    }
                    None => self.mode = self.dimmer(slot, 0),
                }
            }
            view @ Mode::CoverSet { .. } => {
                self.mode = Mode::Menu { view: Box::new(view), actions: vec![Action::Back, Action::Open, Action::Close], selected: 0 };
            }
            Mode::Menu { view, actions, selected } => self.act(*view, actions[selected], out),
            Mode::Idle | Mode::Native | Mode::Alert { .. } => {}
        }
        self.dirty = true;
    }

    /// Does what a ring of actions' chosen item says.
    fn act(&mut self, view: Mode, action: Action, out: &mut Vec<Effect>) {
        match (action, view) {
            (Action::Back, view) => {
                if let Some(index) = view.card_index() {
                    self.go(Mode::Chooser(index), out);
                }
            }
            (Action::Switch(on), Mode::LightDim { slot, light, value, .. }) => {
                self.switch_light(slot, light, on, out);
                self.mode = Mode::LightDim { slot, light, on, value };
            }
            (Action::Warmth, Mode::LightDim { slot, light, .. }) => {
                if let Some(Card::Lights(card)) = self.card(slot) {
                    let one = &card.lights[light];
                    let [low, high] = one.kelvins.unwrap_or([2000, 6500]);
                    let value = one.kelvin.unwrap_or((low + high) / 2).clamp(low, high);
                    self.mode = Mode::LightWarmth { slot, light, value };
                }
            }
            (Action::Colour, Mode::LightDim { slot, light, .. }) => {
                if let Some(Card::Lights(card)) = self.card(slot) {
                    self.mode = Mode::LightHue { slot, light, value: card.lights[light].hue.unwrap_or(30) % 360 };
                }
            }
            (Action::Play(_), Mode::Volume { slot, value }) => {
                self.toggle(slot, None, out);
                self.mode = Mode::Volume { slot, value };
            }
            (Action::Next | Action::Previous, view @ Mode::Volume { slot, .. }) => {
                if let Some(Card::Media(card)) = self.card(slot) {
                    let id = card.id.clone();
                    out.push(Effect::Send(if action == Action::Next { Command::Next(id) } else { Command::Previous(id) }));
                }
                self.mode = view;
            }
            (Action::Preset(i), Mode::ClimateSet { slot, .. }) => {
                if let Some(Card::Climate(card)) = self.cards.cards.get_mut(slot) {
                    if let Some(preset) = card.presets.get(i as usize).cloned() {
                        out.push(Effect::Send(Command::Preset(card.id.clone(), preset.clone())));
                        card.preset = Some(preset);
                    }
                }
                self.go(Mode::Chooser(slot + 1), out);
            }
            (Action::Members, Mode::LightDim { slot, .. }) => self.mode = Mode::Members { slot, selected: 0 },
            (Action::Open | Action::Close, view @ Mode::CoverSet { slot, .. }) => {
                if let Some(Card::Cover(card)) = self.cards.cards.get_mut(slot) {
                    let (id, open) = (card.id.clone(), action == Action::Open);
                    out.push(Effect::Send(if open { Command::Open(id) } else { Command::Close(id) }));
                    card.state = if open { "opening".into() } else { "closing".into() };
                }
                self.mode = view;
            }
            (_, view) => self.mode = view,
        }
    }

    /// Switches a card's device the way it has: a light (`light`, or all of
    /// a card's lights, off if any is on) on or off, a media player played
    /// or paused. False if it has no such way (a thermostat, Back).
    fn toggle(&mut self, slot: usize, light: Option<usize>, out: &mut Vec<Effect>) -> bool {
        match self.cards.cards.get_mut(slot) {
            Some(Card::Lights(card)) => {
                let (lights, on) = match light {
                    Some(index) if index < card.lights.len() => (index..index + 1, !card.lights[index].on),
                    Some(_) => return false,
                    None => (0..card.lights.len(), !card.lights.iter().any(|l| l.on)),
                };
                let changing: Vec<usize> = lights.filter(|&i| card.lights[i].on != on).collect();
                for index in changing {
                    self.switch_light(slot, index, on, out);
                }
                true
            }
            Some(Card::Cover(card)) => {
                let id = card.id.clone();
                let open = card.state == "open" || card.state == "opening";
                out.push(Effect::Send(if open { Command::Close(id) } else { Command::Open(id) }));
                card.state = if open { "closing".into() } else { "opening".into() };
                true
            }
            Some(Card::Media(card)) => {
                out.push(Effect::Send(Command::PlayPause(card.id.clone())));
                card.state = if card.state == "playing" { "paused".into() } else { "playing".into() };
                true
            }
            _ => false,
        }
    }

    /// A long press: a device's on/off or play/pause, from its card or its
    /// own screen. Otherwise back: out of a ring of actions to its device,
    /// out of a device to the cards, and from the cards to the thermostat.
    fn back(&mut self, out: &mut Vec<Effect>) {
        let toggled = match self.mode.clone() {
            Mode::Chooser(index) => match self.deck().get(index) {
                Some(&Slot::Card(slot)) => self.toggle(slot, None, out),
                _ => false,
            },
            Mode::LightList { slot, selected } => self.toggle(slot, Some(selected), out),
            Mode::LightDim { slot, light, value, .. } => {
                let toggled = self.toggle(slot, Some(light), out);
                if let Some(Card::Lights(card)) = self.card(slot) {
                    self.mode = Mode::LightDim { slot, light, on: card.lights[light].on, value };
                }
                toggled
            }
            Mode::LightWarmth { slot, light, .. } | Mode::LightHue { slot, light, .. } => self.toggle(slot, Some(light), out),
            Mode::Volume { slot, .. } => self.toggle(slot, None, out),
            _ => false,
        };
        if toggled {
            out.push(Effect::Click);
            self.dirty = true;
            return;
        }
        let back = match self.mode.clone() {
            Mode::Menu { view, .. } => {
                out.push(Effect::Click);
                self.mode = *view;
                self.dirty = true;
                return;
            }
            Mode::Chooser(_) => Mode::Home,
            mode => match mode.card_index() {
                Some(index) => Mode::Chooser(index),
                None => return,
            },
        };
        out.push(Effect::Click);
        self.go(back, out);
    }

    /// A light's dimmer, as it is now.
    fn dimmer(&self, slot: usize, light: usize) -> Mode {
        let (on, value) = match self.card(slot) {
            Some(Card::Lights(card)) => (card.lights[light].on, card.lights[light].brightness.unwrap_or(100)),
            _ => (false, 100),
        };
        Mode::LightDim { slot, light, on, value }
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
        if let Mode::Alert { id, .. } = &self.mode {
            let dark = !lit && !self.lighting.is_some_and(|until| now < until);
            if dark || self.alert_until.is_some_and(|until| now >= until) {
                let id = id.clone();
                self.end_alert(Some(Command::AlertDismissed(id)), out);
            }
        }
        if self.ran.is_some_and(|(_, at)| now.duration_since(at) >= RAN_SHOWN) {
            self.ran = None;
            self.dirty = true;
        }
        let idle = now.duration_since(self.last_input);
        if self.was_lit.replace(lit) == Some(false) && lit && self.mode == Mode::Home {
            // Lit by itself, as someone came near: wake as a turn would.
            self.last_input = now;
            self.wake(out);
            return;
        }
        // Dark, unless the display was just woken and is still lighting.
        let dark = !lit && !self.lighting.is_some_and(|until| now < until);
        if matches!(self.mode, Mode::Home | Mode::Chooser(_)) && self.turn != 0 && idle >= SNAP {
            let (past_half, sign) = (self.turn.abs() * 2 >= CARD_STEP, self.turn.signum());
            self.turn = 0;
            if past_half {
                self.turned(sign, out);
            }
            self.dirty = true;
            self.sync(out);
        }
        match self.mode.clone() {
            Mode::Native => {
                // Back to the thermostat's dial once nlclient is home; from
                // Settings opened from the cards, the runtime brings the cards.
                if dark || (!self.settings_open && idle >= NATIVE_IDLE && home_dial()) {
                    self.settings_open = false;
                    self.go(Mode::Home, out);
                }
                return;
            }
            Mode::Idle | Mode::Home => return,
            _ => {}
        }
        if dark {
            // Something else put the display out: give it back.
            self.go(Mode::Home, out);
            return;
        }
        if self.last_keep.is_none_or(|last| now.duration_since(last) >= KEEP) {
            self.last_keep = Some(now);
            out.push(Effect::KeepAwake);
        }
        let back = match self.mode.clone() {
            Mode::Chooser(_) if idle >= CARD_IDLE => Some(Mode::Home),
            Mode::LightList { slot, .. } if idle >= LIST_IDLE => Some(Mode::Chooser(slot + 1)),
            Mode::LightDim { slot, light, .. } if idle >= CONTROL_IDLE => Some(match self.card(slot) {
                Some(Card::Lights(card)) if card.lights.len() > 1 => Mode::LightList { slot, selected: light },
                _ => Mode::Chooser(slot + 1),
            }),
            Mode::LightWarmth { slot, light, .. } | Mode::LightHue { slot, light, .. } if idle >= CONTROL_IDLE => {
                Some(self.dimmer(slot, light))
            }
            Mode::ClimateSet { slot, .. } | Mode::Volume { slot, .. } if idle >= CONTROL_IDLE => Some(Mode::Chooser(slot + 1)),
            Mode::Camera { slot } if idle >= CAMERA_IDLE => Some(Mode::Chooser(slot + 1)),
            Mode::OptionList { slot, .. } if idle >= LIST_IDLE => Some(Mode::Chooser(slot + 1)),
            Mode::Members { slot, .. } if idle >= LIST_IDLE => Some(self.dimmer(slot, 0)),
            Mode::CoverSet { slot, .. } if idle >= CONTROL_IDLE => Some(Mode::Chooser(slot + 1)),
            Mode::Menu { view, .. } if idle >= CONTROL_IDLE => Some(*view),
            _ => None,
        };
        if let Some(mode) = back {
            self.last_input = now;
            self.go(mode, out);
        }
        if let Mode::Camera { slot } = self.mode {
            // While it shows, a new picture every few seconds.
            if self.pictured.is_none_or(|at| now.duration_since(at) >= CAMERA_EVERY) {
                if let Some(Card::Camera(card)) = self.card(slot) {
                    out.push(Effect::Send(Command::Snapshot(card.id.clone())));
                }
                self.pictured = Some(now);
            }
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

    fn cards_waking(wake: &str) -> Cards {
        Cards::parse(format!(r#"{{"cards":[
            {{"kind":"lights","name":"Bedroom","icon":"","lights":[
                {{"id":"light.lamp","name":"Lamp","on":true,"dimmable":true,"brightness":60}},
                {{"id":"switch.fan","name":"Fan","on":false,"dimmable":false}}]}},
            {{"kind":"climate","id":"climate.kitchen","name":"Kitchen","target":19,"min":7,"max":30,"step":0.5}},
            {{"kind":"media","id":"media_player.speaker","name":"Speaker","state":"playing","volume":30}}],
            "wake":{wake}}}"#).as_bytes()).unwrap()
    }

    fn cards() -> Cards {
        cards_waking(r#""chooser""#)
    }

    /// A machine with cards, nlclient's dial lit and the ring held.
    struct Rig {
        machine: Machine,
        now: Instant,
        lit: bool,
        dial: bool,
    }

    impl Rig {
        fn with(cards: Cards) -> Rig {
            let now = Instant::now();
            let mut machine = Machine::new(now);
            let mut out = vec![];
            machine.set_cards(cards, &mut out);
            machine.set_active(true, now, &mut out);
            let mut rig = Rig { machine, now, lit: true, dial: true };
            rig.tick();
            assert_eq!(rig.machine.mode, Mode::Home);
            rig
        }

        fn new() -> Rig {
            Rig::with(cards())
        }

        /// On the cards, as a turn on the dark display leaves them.
        fn woken() -> Rig {
            let mut rig = Rig::new();
            rig.lit = false;
            rig.press();
            rig.lit = true;
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

        fn long(&mut self) -> Vec<Effect> {
            self.input(Input::LongPress)
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
    fn waking_shows_the_cards_on_the_thermostat() {
        let mut rig = Rig::new();
        rig.lit = false;
        assert_eq!(rig.turn(CARD_STEP * 3), vec![Effect::WakeDisplay, Effect::Own(true)],
                   "the turn only wakes it, onto the cards");
        assert_eq!(rig.machine.mode, Mode::Chooser(0));
        assert!(rig.machine.woke && rig.machine.hint);
        rig.lit = true;
        rig.turn(CARD_STEP);
        assert_eq!(rig.machine.mode, Mode::Chooser(1));
        assert!(!rig.machine.hint, "the hint goes with the first turn");
    }

    #[test]
    fn a_card_of_one_light_dims_it_or_switches_it() {
        let mut rig = Rig::with(Cards::parse(br#"{"cards":[
            {"kind":"lights","name":"Lamp","icon":"","lights":[
                {"id":"light.lamp","name":"Lamp","on":true,"dimmable":true,"brightness":60}]},
            {"kind":"lights","name":"Fan","icon":"","lights":[
                {"id":"switch.fan","name":"Fan","on":false,"dimmable":false}]}]}"#).unwrap());
        rig.lit = false;
        rig.press();
        rig.lit = true;
        rig.turn(CARD_STEP);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 0, light: 0, on: true, value: 60 }, "straight to its dimmer");
        rig.wait(CONTROL_IDLE);
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "left: back to the cards, no list");
        rig.turn(CARD_STEP);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::On("switch.fan".into())], "switched where it is");
        assert_eq!(rig.machine.mode, Mode::Chooser(2));
    }

    #[test]
    fn a_light_s_warmth_and_colour_have_dials_of_their_own() {
        let mut rig = Rig::with(Cards::parse(br#"{"cards":[
            {"kind":"lights","name":"Lamp","icon":"","lights":[
                {"id":"light.lamp","name":"Lamp","on":true,"dimmable":true,"brightness":60,
                 "kelvin":2700,"kelvins":[2200,6500],"hues":true,"hue":350}]},
            {"kind":"lights","name":"Strip","icon":"","lights":[
                {"id":"light.strip","name":"Strip","on":true,"dimmable":true,"brightness":60}]}]}"#).unwrap());
        rig.lit = false;
        rig.press();
        rig.lit = true;
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        let Mode::Menu { actions, .. } = rig.machine.mode.clone() else { panic!() };
        assert_eq!(actions, vec![Action::Back, Action::Switch(false), Action::Warmth, Action::Colour]);
        rig.turn(ROW_STEP * 2);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::LightWarmth { slot: 0, light: 0, value: 2700 });
        for _ in 0..3 {
            rig.turn(VALUE_STEP);
        }
        assert_eq!(Rig::sends(&rig.wait(SEND_EVERY)), vec![Command::Kelvin("light.lamp".into(), 2850)]);
        rig.turn(-VALUE_STEP * 40);
        assert_eq!(rig.machine.mode, Mode::LightWarmth { slot: 0, light: 0, value: 2200 }, "no warmer than it goes");
        rig.press();
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 0, light: 0, on: true, value: 60 }, "done: its dimmer");
        rig.press();
        rig.turn(ROW_STEP * 3);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::LightHue { slot: 0, light: 0, value: 350 });
        rig.turn(VALUE_STEP * 4);
        assert_eq!(rig.machine.mode, Mode::LightHue { slot: 0, light: 0, value: 6 }, "round the wheel");
        rig.wait(CONTROL_IDLE);
        assert!(matches!(rig.machine.mode, Mode::LightDim { .. }), "left: its dimmer");
        rig.press();
        rig.press();
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        let Mode::Menu { actions, .. } = rig.machine.mode.clone() else { panic!() };
        assert_eq!(actions, vec![Action::Back, Action::Switch(false)], "a light with neither: neither");
    }

    #[test]
    fn home_assistant_can_wake_it() {
        let mut rig = Rig::new();
        rig.lit = false;
        rig.wait(Duration::from_secs(5));
        let mut out = vec![];
        rig.machine.wake_up(rig.now, false, &mut out);
        assert_eq!(out, vec![Effect::WakeDisplay, Effect::Own(true)]);
        assert_eq!(rig.machine.mode, Mode::Chooser(0));
        rig.lit = true;
        rig.turn(CARD_STEP);
        let mut out = vec![];
        rig.machine.wake_up(rig.now, true, &mut out);
        assert!(out.is_empty() && rig.machine.mode == Mode::Chooser(1), "showing already: left as it is");
    }

    fn alert(id: &str, timeout: f64) -> Alert {
        Alert { id: id.into(), title: "Front door".into(), timeout, ..Default::default() }
    }

    #[test]
    fn a_notification_shows_over_anything_and_goes() {
        let mut rig = Rig::new();
        rig.lit = false;
        let mut out = vec![];
        rig.machine.show_alert(alert("door", 30.0), rig.now, false, &mut out);
        assert_eq!(out, vec![Effect::WakeDisplay, Effect::Chime, Effect::Own(true)], "dark: woken, with a chime");
        rig.lit = true;
        rig.turn(CARD_STEP * 3);
        assert!(matches!(rig.machine.mode, Mode::Alert { .. }), "turning doesn't move it");
        assert_eq!(Rig::sends(&rig.press()), vec![Command::AlertPressed("door".into())], "a press answers it");
        assert_eq!(rig.machine.mode, Mode::Home, "and what was there shows again");

        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        rig.press();
        let dimmer = rig.machine.mode.clone();
        let mut out = vec![];
        rig.machine.show_alert(alert("wash", 10.0), rig.now, true, &mut out);
        assert_eq!(Rig::sends(&rig.long()), vec![Command::AlertDismissed("wash".into())], "held: put away");
        assert_eq!(rig.machine.mode, dimmer, "back to the dimmer it covered");

        rig.machine.show_alert(alert("wash", 10.0), rig.now, true, &mut vec![]);
        assert_eq!(Rig::sends(&rig.wait(Duration::from_secs(11))), vec![Command::AlertDismissed("wash".into())],
                   "its time up");
        rig.machine.show_alert(alert("door", 30.0), rig.now, true, &mut vec![]);
        let mut gone = alert("door", 30.0);
        gone.dismiss = true;
        let mut out = vec![];
        rig.machine.show_alert(gone, rig.now, true, &mut out);
        assert!(Rig::sends(&out).is_empty() && rig.machine.mode == dimmer, "Home Assistant took it back");
    }

    #[test]
    fn a_scene_runs_and_a_camera_takes_a_picture_from_their_cards() {
        let mut rig = Rig::with(Cards::parse(br#"{"cards":[
            {"kind":"action","id":"scene.goodnight","name":"Goodnight"},
            {"kind":"camera","id":"camera.front_door","name":"Front door"},
            {"kind":"doorbell_from_the_future","id":"x"}],"wake":0}"#).unwrap());
        assert_eq!(rig.machine.cards.cards.len(), 2, "a kind it doesn't know is left out");
        rig.lit = false;
        rig.press();
        rig.lit = true;
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "woken to a scene: shown, not run");
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Run("scene.goodnight".into())]);
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "it stays on the cards");
        assert!(rig.machine.ran.is_some_and(|(slot, _)| slot == 0), "its card says so");
        rig.wait(RAN_SHOWN);
        assert_eq!(rig.machine.ran, None, "for a moment");
        rig.turn(CARD_STEP);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Snapshot("camera.front_door".into())]);
        assert_eq!(rig.machine.mode, Mode::Camera { slot: 1 }, "a camera opens full screen");
        assert_eq!(Rig::sends(&rig.wait(CAMERA_EVERY)), vec![Command::Snapshot("camera.front_door".into())],
                   "a new picture every few seconds");
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Chooser(2), "a press: back to the cards");
        rig.press();
        rig.wait(CAMERA_IDLE);
        assert_eq!(rig.machine.mode, Mode::Chooser(2), "left: back too");
    }

    fn deck_of(json: &[u8]) -> Rig {
        let mut rig = Rig::with(Cards::parse(json).unwrap());
        rig.lit = false;
        rig.press();
        rig.lit = true;
        rig
    }

    #[test]
    fn an_option_is_chosen_from_its_list() {
        let mut rig = deck_of(br#"{"cards":[{"kind":"options","id":"select.boost","name":"Hot water boost",
            "options":["Off","30 min","1 hour"],"current":"Off"}]}"#);
        rig.turn(CARD_STEP);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::OptionList { slot: 0, selected: 0 }, "on the current one");
        rig.turn(ROW_STEP * 2);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Select("select.boost".into(), "1 hour".into())]);
        assert_eq!(rig.machine.mode, Mode::Chooser(1));
        assert!(matches!(&rig.machine.cards.cards[0], Card::Options(c) if c.current.as_deref() == Some("1 hour")));
        rig.press();
        rig.turn(ROW_STEP * 9);
        assert!(Rig::sends(&rig.press()).is_empty() && rig.machine.mode == Mode::Chooser(1), "Back: nothing chosen");
    }

    #[test]
    fn a_cover_opens_as_far_as_turned_and_its_menu_opens_or_closes_it() {
        let mut rig = deck_of(br#"{"cards":[{"kind":"cover","id":"cover.blind","name":"Blind","state":"open",
            "position":40,"positionable":true}]}"#);
        rig.turn(CARD_STEP);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::CoverSet { slot: 0, value: 40 });
        for _ in 0..5 {
            rig.turn(VALUE_STEP);
        }
        assert_eq!(Rig::sends(&rig.wait(SEND_EVERY)), vec![Command::Position("cover.blind".into(), 45)]);
        rig.press();
        rig.turn(ROW_STEP * 2);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Close("cover.blind".into())], "its menu: Close");
        rig.long();
        assert_eq!(rig.machine.mode, Mode::Chooser(1));
        assert_eq!(Rig::sends(&rig.long()), vec![Command::Open("cover.blind".into())],
                   "long press on its card: open again, as it's closing");
    }

    #[test]
    fn a_thermostat_s_presets_and_a_speaker_s_tracks_are_in_their_menus() {
        let mut rig = deck_of(br#"{"cards":[
            {"kind":"climate","id":"climate.kitchen","name":"Kitchen","target":19,"presets":["eco","away"]},
            {"kind":"media","id":"media_player.speaker","name":"Speaker","state":"playing","volume":30}]}"#);
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        assert!(matches!(&rig.machine.mode, Mode::Menu { actions, .. } if actions[1..] == [Action::Preset(0), Action::Preset(1)]));
        rig.turn(ROW_STEP);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Preset("climate.kitchen".into(), "eco".into())]);
        assert_eq!(rig.machine.mode, Mode::Chooser(1));
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        rig.turn(ROW_STEP * 3);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Next("media_player.speaker".into())]);
        assert!(matches!(rig.machine.mode, Mode::Volume { .. }), "back to its volume");
    }

    #[test]
    fn a_light_group_s_menu_lists_its_lights_to_switch() {
        let mut rig = deck_of(br#"{"cards":[{"kind":"lights","name":"Bedroom","lights":[
            {"id":"light.bedroom","name":"Bedroom","on":true,"dimmable":true,"brightness":50}],
            "members":[{"id":"light.lamp","name":"Lamp","on":true},{"id":"light.ceiling","name":"Ceiling","on":false}]}]}"#);
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        let Mode::Menu { actions, .. } = rig.machine.mode.clone() else { panic!() };
        assert_eq!(actions.last(), Some(&Action::Members));
        rig.turn(ROW_STEP * (actions.len() as i32 - 1));
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Members { slot: 0, selected: 0 });
        rig.turn(ROW_STEP);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::On("light.ceiling".into())], "each switched");
        rig.turn(ROW_STEP);
        rig.press();
        assert!(matches!(rig.machine.mode, Mode::LightDim { .. }), "Back: the group's dimmer");
    }

    #[test]
    fn lighting_by_itself_wakes_to_the_cards_too() {
        let mut rig = Rig::new();
        rig.lit = false;
        rig.wait(Duration::from_secs(5));
        assert_eq!(rig.machine.mode, Mode::Home);
        // Someone comes near: the Nest lights its display itself.
        rig.lit = true;
        let out = rig.tick();
        assert_eq!(out, vec![Effect::Own(true)], "no wake asked for: it is lit");
        assert_eq!(rig.machine.mode, Mode::Chooser(0));
        assert!(rig.machine.woke && rig.machine.hint);
        rig.wait(CARD_IDLE);
        assert_eq!(rig.machine.mode, Mode::Home, "left, back to the dial");
        rig.wait(Duration::from_secs(5));
        assert_eq!(rig.machine.mode, Mode::Home, "and it stays there while lit");
    }

    #[test]
    fn a_woken_display_has_time_to_light() {
        let mut rig = Rig::new();
        rig.lit = false;
        rig.press();
        assert!(!rig.wait(LIGHTING - Duration::from_millis(100)).contains(&Effect::Own(false)),
                "still lighting: the cards stay");
        assert_eq!(rig.machine.mode, Mode::Chooser(0));
        assert!(rig.wait(Duration::from_millis(200)).contains(&Effect::Own(false)), "dark after that: it went out");
        let mut rig = Rig::with(cards_waking(r#""thermostat""#));
        rig.lit = false;
        rig.press();
        rig.wait(LIGHTING - Duration::from_millis(100));
        assert_eq!(rig.machine.mode, Mode::Native, "nor does the thermostat, woken onto");
    }

    #[test]
    fn waking_to_what_home_assistant_chose() {
        let mut rig = Rig::with(cards_waking(r#""thermostat""#));
        rig.lit = false;
        assert_eq!(rig.press(), vec![Effect::WakeDisplay, Effect::Hold(false)], "the Nest as it was");
        assert_eq!(rig.machine.mode, Mode::Native);
        let mut rig = Rig::with(cards_waking("1"));
        rig.lit = false;
        assert_eq!(rig.press(), vec![Effect::WakeDisplay, Effect::Own(true)]);
        assert_eq!(rig.machine.mode, Mode::ClimateSet { slot: 1, target: 19.0 }, "the card opened");
    }

    #[test]
    fn turning_browses_and_nothing_opens_by_itself() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        rig.wait(SNAP * 8);
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "stopped on a card, it stays a card");
        for index in [2, 3, 4] {
            rig.turn(CARD_STEP);
            assert_eq!(rig.machine.mode, Mode::Chooser(index));
        }
        assert!(Rig::sends(&rig.turn(CARD_STEP)).is_empty());
        assert_eq!(rig.machine.mode, Mode::Chooser(4), "settings is the last: they stop there");
        rig.turn(-CARD_STEP * 9);
        assert_eq!(rig.machine.mode, Mode::Chooser(0), "and at the thermostat, the first");
        assert!(rig.turn(-CARD_STEP).is_empty(), "nothing before it");
        assert!(rig.machine.position().unwrap() >= -(END_GIVE as f32) / CARD_STEP as f32, "past the end: only a little give");
        rig.wait(SNAP * 2);
        assert_eq!(rig.machine.position(), Some(0.0), "and back");
    }

    #[test]
    fn cards_follow_a_turn_and_settle_on_the_nearest() {
        let mut rig = Rig::new();
        assert_eq!(rig.turn(CARD_STEP / 5), vec![Effect::Own(true)], "a turn on the lit dial shows the cards");
        assert_eq!(rig.machine.position(), Some(0.2));
        assert_eq!(rig.wait(SNAP * 2), vec![Effect::Own(false)], "short of halfway: back to the thermostat");
        assert_eq!((rig.machine.mode.clone(), rig.machine.position()), (Mode::Home, Some(0.0)));
        rig.turn(CARD_STEP * 3 / 5);
        rig.wait(SNAP * 2);
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "past halfway: on to the next card");
        assert_eq!(rig.machine.position(), Some(1.0));
        rig.turn(-CARD_STEP * 2 / 5);
        assert!((rig.machine.position().unwrap() - 0.6).abs() < 1e-6, "backwards, towards the thermostat");
        rig.turn(CARD_STEP / 5);
        assert!((rig.machine.position().unwrap() - 0.8).abs() < 1e-6, "and back again, smoothly");
    }

    #[test]
    fn a_press_opens_the_card() {
        let mut rig = Rig::woken();
        assert_eq!(rig.press(), vec![Effect::Own(false), Effect::Hold(false)], "the thermostat: nlclient's ring");
        assert_eq!(rig.machine.mode, Mode::Native);
        assert!(rig.turn(CARD_STEP * 4).is_empty(), "nlclient's turn: the temperature");
        assert_eq!(rig.long(), vec![Effect::Own(true), Effect::Hold(true)], "a long press: the cards again");
        assert_eq!(rig.machine.mode, Mode::Chooser(0));
        rig.press();
        assert_eq!(rig.input(Input::NativeBack), vec![Effect::Own(true), Effect::Hold(true)],
                   "Back in nlclient's menu: the cards");
        assert_eq!(rig.machine.mode, Mode::Chooser(0));
        assert!(rig.input(Input::NativeBack).is_empty(), "only from nlclient's screens");
        rig.turn(CARD_STEP);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::LightList { slot: 0, selected: 0 });
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP * 4);
        assert_eq!(rig.press(), vec![Effect::Own(false), Effect::Hold(false), Effect::OpenNestSettings],
                   "Nest settings: nlclient's menu");
        assert!(rig.machine.settings_open, "straight to nlclient's Settings");
        rig.wait(NATIVE_IDLE * 3);
        assert_eq!(rig.machine.mode, Mode::Native, "staying while in Settings, though nlclient's dial shows");
        rig.input(Input::SettingsDone);
        assert_eq!(rig.machine.mode, Mode::Chooser(4), "Done: the cards, on Nest settings");
        assert!(!rig.machine.settings_open);
    }

    #[test]
    fn the_lit_dial_opens_on_a_press_and_comes_back_when_home() {
        let mut rig = Rig::new();
        assert_eq!(rig.press(), vec![Effect::Hold(false)]);
        assert_eq!(rig.machine.mode, Mode::Native);
        rig.dial = false;
        assert!(rig.wait(NATIVE_IDLE * 2).is_empty(), "not while nlclient shows something else");
        rig.dial = true;
        assert_eq!(rig.wait(Duration::from_millis(100)), vec![Effect::Hold(true)]);
        rig.press();
        rig.lit = false;
        assert_eq!(rig.tick(), vec![Effect::Hold(true)], "dark is home too");
    }

    #[test]
    fn the_cards_keep_the_unit_awake_and_go_back_when_left() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        let effects = rig.wait(CARD_IDLE - Duration::from_millis(100));
        let keeps = effects.iter().filter(|e| **e == Effect::KeepAwake).count();
        assert!(keeps >= 5, "every 4 s: {keeps}");
        assert!(!effects.iter().any(|e| matches!(e, Effect::WakeDisplay)), "nlclient keeps its display lit itself");
        assert_eq!(rig.wait(Duration::from_millis(200)), vec![Effect::Own(false)]);
        assert_eq!(rig.machine.mode, Mode::Home);
    }

    #[test]
    fn a_dark_display_under_the_cards_goes_back_to_nlclient() {
        let mut rig = Rig::woken();
        rig.wait(LIGHTING);
        rig.lit = false;
        assert_eq!(rig.tick(), vec![Effect::Own(false)]);
        assert_eq!(rig.machine.mode, Mode::Home);
    }

    #[test]
    fn the_light_list_switches_and_dims_and_a_menu_goes_back() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        rig.press();
        rig.turn(ROW_STEP);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::On("switch.fan".into())], "on/off: switched in place");
        assert!(matches!(&rig.machine.cards.cards[0], Card::Lights(c) if c.lights[1].on), "shown on at once");
        rig.turn(ROW_STEP * 5);
        assert_eq!(rig.machine.mode, Mode::LightList { slot: 0, selected: 2 }, "the list stops at Back");
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "Back: the cards");
        rig.press();
        rig.press();
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 0, light: 0, on: true, value: 60 });
        for _ in 0..5 {
            rig.turn(VALUE_STEP);
        }
        let sent = Rig::sends(&rig.wait(SEND_EVERY));
        assert_eq!(sent, vec![Command::Brightness("light.lamp".into(), 65)], "a turn's commands are thinned");
        rig.press();
        let dim = Mode::LightDim { slot: 0, light: 0, on: true, value: 65 };
        assert_eq!(rig.machine.mode, Mode::Menu { view: Box::new(dim.clone()),
                                                  actions: vec![Action::Back, Action::Switch(false)], selected: 0 },
                   "a press: the ring of actions, Back first");
        rig.turn(ROW_STEP);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Off("light.lamp".into())]);
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 0, light: 0, on: false, value: 65 });
        rig.press();
        rig.long();
        assert!(matches!(rig.machine.mode, Mode::LightDim { .. }), "a long press closes the menu");
        rig.press();
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "Back: the cards");
    }

    #[test]
    fn each_level_goes_back_when_left() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        rig.press();
        assert!(matches!(rig.machine.mode, Mode::Menu { .. }));
        rig.wait(CONTROL_IDLE);
        assert!(matches!(rig.machine.mode, Mode::LightDim { .. }));
        rig.wait(CONTROL_IDLE);
        assert_eq!(rig.machine.mode, Mode::LightList { slot: 0, selected: 0 });
        rig.wait(LIST_IDLE);
        assert_eq!(rig.machine.mode, Mode::Chooser(1));
    }

    #[test]
    fn a_turned_off_light_comes_on_from_low() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        rig.press();
        rig.turn(ROW_STEP);
        rig.press();
        rig.turn(VALUE_STEP);
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 0, light: 0, on: true, value: 1 });
    }

    #[test]
    fn other_thermostats_and_media_are_set_with_the_ring() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP * 2);
        rig.press();
        rig.turn(CLIMATE_STEP * 3);
        assert_eq!(rig.machine.mode, Mode::ClimateSet { slot: 1, target: 20.5 });
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Temperature("climate.kitchen".into(), 20.5)],
                   "done sends what is still waiting");
        assert_eq!(rig.machine.mode, Mode::Chooser(2));
        rig.turn(CARD_STEP);
        rig.press();
        rig.turn(-VALUE_STEP * 5);
        assert_eq!(rig.machine.mode, Mode::Volume { slot: 2, value: 25 });
        rig.press();
        assert!(matches!(&rig.machine.mode, Mode::Menu { actions, .. } if actions[1] == Action::Play(false)),
                "playing: the menu offers pause");
        rig.turn(ROW_STEP);
        let effects = rig.press();
        assert!(Rig::sends(&effects).contains(&Command::PlayPause("media_player.speaker".into())));
        assert!(matches!(&rig.machine.cards.cards[2], Card::Media(m) if m.state == "paused"));
        assert_eq!(rig.machine.mode, Mode::Volume { slot: 2, value: 25 });
    }

    #[test]
    fn a_long_press_switches_a_device_or_goes_back_a_level() {
        let mut rig = Rig::new();
        assert!(rig.long().is_empty(), "nothing above the thermostat");
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        assert_eq!(Rig::sends(&rig.long()), vec![Command::Off("light.lamp".into())], "a card: off, as one is on");
        assert_eq!(Rig::sends(&rig.long()), vec![Command::On("light.lamp".into()), Command::On("switch.fan".into())],
                   "then all on");
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "still on the cards");
        rig.press();
        rig.press();
        rig.turn(VALUE_STEP * 3);
        assert_eq!(Rig::sends(&rig.long()), vec![Command::Off("light.lamp".into())],
                   "on its dimmer: off, what was turned dropped");
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 0, light: 0, on: false, value: 63 });
        rig.press();
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "the menu's Back");
        rig.press();
        rig.turn(ROW_STEP * 5);
        rig.long();
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "the list's Back row, long: back");
        rig.turn(CARD_STEP);
        rig.long();
        assert_eq!(rig.machine.mode, Mode::Home, "a thermostat's card has no switch: back to the dial");
        rig.lit = false;
        rig.wait(Duration::from_secs(1));
        rig.press();
        rig.lit = true;
        rig.turn(CARD_STEP * 3);
        assert_eq!(Rig::sends(&rig.long()), vec![Command::PlayPause("media_player.speaker".into())]);
        rig.press();
        assert_eq!(Rig::sends(&rig.long()), vec![Command::PlayPause("media_player.speaker".into())],
                   "and on its own screen");
        assert!(matches!(&rig.machine.cards.cards[2], Card::Media(m) if m.state == "playing"));
    }

    #[test]
    fn clicks_on_each_step_and_press() {
        let mut rig = Rig::woken();
        let clicks = |effects: Vec<Effect>| effects.iter().filter(|e| **e == Effect::Click).count();
        assert_eq!(clicks(rig.input_with_clicks(Input::Turn(CARD_STEP / 2))), 0, "not between steps");
        assert_eq!(clicks(rig.input_with_clicks(Input::Turn(CARD_STEP / 2))), 1);
        assert_eq!(clicks(rig.input_with_clicks(Input::Turn(-CARD_STEP))), 1, "back to the first");
        assert_eq!(clicks(rig.input_with_clicks(Input::Turn(-CARD_STEP))), 0, "none past the end");
        rig.wait(SNAP * 2);
        rig.turn(CARD_STEP);
        assert_eq!(clicks(rig.input_with_clicks(Input::Press)), 1);
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP * 4);
        assert_eq!(clicks(rig.input_with_clicks(Input::Press)), 0, "nlclient clicks for Nest settings");
        let mut rig = Rig::new();
        rig.lit = false;
        assert_eq!(clicks(rig.input_with_clicks(Input::Turn(CARD_STEP))), 0, "waking is silent, as nlclient's");
    }

    #[test]
    fn cards_changing_underneath_go_back_to_the_thermostat_if_gone() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        let mut out = vec![];
        let mut fewer = cards();
        fewer.cards.remove(0);
        rig.machine.set_cards(fewer, &mut out);
        assert_eq!((rig.machine.mode.clone(), out), (Mode::Home, vec![Effect::Own(false)]));
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP * 2);
        let mut out = vec![];
        rig.machine.set_cards(cards(), &mut out);
        assert_eq!(rig.machine.mode, Mode::Chooser(2), "a card still there stays");
    }
}
