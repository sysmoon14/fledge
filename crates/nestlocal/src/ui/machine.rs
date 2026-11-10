//! What the ring and button do, as a state machine apart from the unit: it
//! takes turns, presses and time, and says what should happen (Effect).
//!
//! One rule throughout: zoomed out on the cards (the chooser), the ring
//! moves between devices; opened, it works the device. Waking shows the
//! chooser on the thermostat, or what Home Assistant chose (Wake). A press
//! opens the card; inside, a press acts, or opens a ring of actions whose
//! first is Back. A long press goes back a level. Going back is the hand's
//! alone: left untouched, the screen times out wherever it is, and the next
//! wake starts again from the top.

use std::time::{Duration, Instant};

use super::model::{Alert, Card, Cards, Command, Far, Near, Nest, Wake};
use crate::motion::Sensor;
use crate::logf;

/// Ring counts per step, by what a step moves: a card, a list row, a value.
/// REL_DIAL reports the ADC's raw counts; a step is a deliberate movement.
pub const CARD_STEP: i32 = 500;
pub const ROW_STEP: i32 = 250;
/// A device's ring of actions: an icon a step, never two at once (a quick
/// hand's reading can hold several rows' worth), and what's left over after
/// a step doesn't count towards the next.
pub const MENU_STEP: i32 = 400;
/// And at most an icon this often: the menu takes a while to draw, and an
/// icon passed quicker than it shows is skipped as far as the eye goes.
pub const MENU_EVERY: Duration = Duration::from_millis(220);
/// The off thermostat asks for a mode only for a turn meant as one: this
/// much, and none of it from before a press (a hand coming off the ring).
pub const PROMPT_TURN: i32 = 250;
/// A percent of brightness or volume; a thermostat's step (half a degree).
pub const VALUE_STEP: i32 = 30;
pub const CLIMATE_STEP: i32 = 90;
/// The thermostat's ring clicks this often: twice a step, its target
/// following the hand between, as the Nest's ring does.
pub const CLIMATE_CLICK: i32 = CLIMATE_STEP / 2;

/// `t` to the nearest `step` (a thermostat's half degree).
pub fn on_step(t: f64, step: f64) -> f64 {
    (t / step).round() * step
}
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
/// How long the glance shows with nothing touched.
pub const GLANCE_IDLE: Duration = Duration::from_secs(20);
/// The display lit this soon after an approach reading lit it for someone
/// close, not across the room.
pub const NEAR_LIT: Duration = Duration::from_secs(2);
/// How long a scene's card says it ran.
pub const RAN_SHOWN: Duration = Duration::from_secs(2);
/// A step of a light's warmth dial, in kelvin, and of its colour's, in degrees.
pub const KELVIN_STEP: i32 = 50;
pub const HUE_STEP: i32 = 4;
/// The colour dial's far end, a step short of where it began (red).
pub const HUE_MAX: u16 = 356;
/// How far the cards follow a turn past either end, in counts.
pub const END_GIVE: i32 = CARD_STEP / 6;
pub const SNAP: Duration = Duration::from_millis(250);
/// How long the screen stays, untouched, on the cards or anything opened
/// from them, before it times out (Home Assistant's card_idle, if set).
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
/// The thermostat's target goes to nlclient once the ring has rested this
/// long, not at every step: each change is a bucket write it acts on.
pub const SETTLE: Duration = Duration::from_millis(700);
/// How long a media player's volume shows large after the ring turns.
pub const VOLUME_SHOWN: Duration = Duration::from_millis(1500);
/// How long the thermostat's dial says "Heat set to" after the ring turns.
pub const SET_SHOWN: Duration = Duration::from_secs(3);
/// The minutes a hot water boost runs from the thermostat's menu.
pub const BOOST_MINUTES: u32 = 60;

/// A row of the thermostat's own Settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setting {
    /// Eco's lowest temperature, and highest (with cooling).
    EcoLow,
    EcoHigh,
    HomeAway,
    /// Auto-Schedule: the thermostat learns a schedule.
    Learning,
    EarlyOn,
    Sunblock,
    HotWaterSchedule,
    /// How the screens look: the Nest's own, or Fledge's.
    Theme,
    /// °C or °F.
    Scale,
    /// Nest's own dial in place of the thermostat card.
    NestDial,
    /// Fledge's version; pressed, it looks for an update, or installs one.
    Software,
    /// nlclient's own Settings, for the rest (Wi-Fi, equipment, reset).
    More,
    Done,
}

impl Setting {
    /// The ones set on their own dial (temperatures), with their range.
    pub fn range(self, fahrenheit: bool) -> Option<(f64, f64, f64)> {
        let c = |low: f64, high: f64| if fahrenheit {
            ((low * 1.8 + 32.0).round(), (high * 1.8 + 32.0).round(), 1.0)
        } else {
            (low, high, 0.5)
        };
        match self {
            Setting::EcoLow => Some(c(crate::commands::ECO_LOW.0, crate::commands::ECO_LOW.1)),
            Setting::EcoHigh => Some(c(crate::commands::ECO_HIGH.0, crate::commands::ECO_HIGH.1)),
            _ => None,
        }
    }
}

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
    /// The thermostat: Eco on or off, heating on or off, a hot water boost
    /// on or off, and Nest's own dial (its schedule, its menus).
    Eco(bool),
    Power(bool),
    Boost(bool),
    NestDial,
}

/// The off thermostat's prompt, as the Nest's words, and its choices: on
/// (heat), left off, or not now. Listed under it as Settings are, from row 1
/// (row 0 is the question).
pub const MODE_PROMPT: &str = "To adjust the temperature,\nchange the thermostat mode";
pub const PROMPT_ON: usize = 1;
pub const PROMPT_OFF: usize = 2;
pub const PROMPT_CANCEL: usize = 3;

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
    /// The time, the weather and what's on, as someone comes near; a touch
    /// shows what waking does.
    Glance,
    /// This thermostat, the ring setting its target (as it shows it).
    Thermostat { target: f64 },
    /// Its Settings, `selected` highlighted, and one set on its own dial.
    Settings { selected: usize },
    SettingSet { setting: Setting, value: f64 },
    /// Settings' Theme: the two side by side, `fledge` chosen.
    ThemePick { fledge: bool },
    /// The thermostat off and its ring turned: change the mode? Over its dial
    /// at `target`, `selected` (PROMPT_ON..=PROMPT_CANCEL) highlighted.
    ModePrompt { target: f64, selected: usize },
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

    /// The card a device's own screen belongs to.
    pub fn card_slot(&self) -> Option<Slot> {
        match self {
            Mode::LightList { slot, .. } | Mode::LightDim { slot, .. } | Mode::LightWarmth { slot, .. }
            | Mode::LightHue { slot, .. } | Mode::ClimateSet { slot, .. } | Mode::Camera { slot }
            | Mode::Volume { slot, .. } | Mode::OptionList { slot, .. } | Mode::Members { slot, .. }
            | Mode::CoverSet { slot, .. } => Some(Slot::Card(*slot)),
            Mode::Thermostat { .. } | Mode::ModePrompt { .. } => Some(Slot::Thermostat),
            Mode::Menu { view, .. } => view.card_slot(),
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
    /// Nest's own dial for the thermostat (true), or the thermostat card:
    /// kept for the next start.
    NestDial(bool),
    /// The Fledge theme (true), or the Nest's: kept for the next start.
    Theme(bool),
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
    /// This thermostat's own state, once nestlocal has it, and whether its
    /// card gives way to Nest's own dial (the thermostat as it was).
    pub nest: Option<Nest>,
    pub nest_dial: bool,
    /// When the thermostat's ring last turned: its dial says what it's set
    /// to for SET_SHOWN after.
    pub set_at: Option<Instant>,
    /// When the ring last stepped a ring of actions on (MENU_EVERY).
    menu_stepped: Option<Instant>,
    /// How far the thermostat's ring has gone towards its next step (-1..1),
    /// its mark following, and staying there at rest, as the Nest's does.
    pub dial_offset: f64,
    /// When a media player's ring last turned: its volume shows large for
    /// VOLUME_SHOWN after, its track the rest of the time.
    pub volume_at: Option<Instant>,
    /// The Fledge theme rather than the Nest's.
    pub fledge: bool,
    /// nlclient's screen patch lets the UI hold the display at rest, its own
    /// timing putting it out (keeper.rests): the UI never hands it back but
    /// to nlclient's own screens (holds_screen).
    pub rests: bool,
    /// When the thermostat last saw someone close.
    near_at: Option<Instant>,
}

impl Machine {
    pub fn new(now: Instant) -> Machine {
        Machine {
            mode: Mode::Idle, cards: Cards::default(), woke: false, hint: false, dirty: false, last_input: now,
            last_keep: None, turn: 0, movement: 0, pending: None, last_sent: now, owned: false, held: false,
            lighting: None, was_lit: None, alert: None, alert_until: None, ran: None, pictured: None, settings_open: false,
            nest: None, nest_dial: false, set_at: None, menu_stepped: None, dial_offset: 0.0, volume_at: None, fledge: false,
            rests: false, near_at: None,
        }
    }

    /// How long since the last turn or press.
    pub fn idle(&self, now: Instant) -> Duration {
        now.duration_since(self.last_input)
    }

    /// Whether the thermostat is our own card, rather than Nest's dial.
    pub fn own_thermostat(&self) -> bool {
        !self.nest_dial && self.nest.is_some()
    }

    /// The thermostat's state, as nestlocal has it now.
    pub fn set_nest(&mut self, nest: Nest) {
        if self.nest.as_ref() != Some(&nest) {
            self.nest = Some(nest);
            self.dirty = true;
        }
    }

    /// The rows of Settings, for what this thermostat has.
    pub fn settings(&self) -> Vec<Setting> {
        let mut rows = vec![Setting::EcoLow];
        let nest = self.nest.as_ref();
        if nest.is_some_and(|n| n.can_cool) {
            rows.push(Setting::EcoHigh);
        }
        rows.extend([Setting::HomeAway, Setting::Learning, Setting::EarlyOn]);
        if nest.is_none_or(|n| n.can_heat) {
            rows.push(Setting::Sunblock);
        }
        if nest.is_some_and(|n| n.hot_water) {
            rows.push(Setting::HotWaterSchedule);
        }
        rows.extend([Setting::Theme, Setting::Scale, Setting::NestDial, Setting::Software, Setting::More, Setting::Done]);
        rows
    }

    /// The thermostat opened: its own dial, or Nest's.
    fn thermostat(&mut self) -> Mode {
        self.dial_offset = 0.0;
        match &self.nest {
            Some(nest) if !self.nest_dial => {
                let card = &nest.card;
                Mode::Thermostat { target: card.target.unwrap_or((card.min + card.max) / 2.0) }
            }
            _ => Mode::Native,
        }
    }

    /// Settings, on `setting`'s row.
    fn settings_at(&self, setting: Setting) -> Mode {
        Mode::Settings { selected: self.settings().iter().position(|s| *s == setting).unwrap_or(0) }
    }

    /// Sends one of the thermostat's own commands.
    fn nest_command(&mut self, name: &str, arg: impl Into<String>, out: &mut Vec<Effect>) {
        out.push(Effect::Send(Command::Nest(name.into(), arg.into())));
    }

    /// The cards in turning order: Home Assistant's, with the thermostat
    /// where it put it (first, unless it says), then settings.
    pub fn deck(&self) -> Vec<Slot> {
        let count = self.cards.cards.len();
        let mut deck: Vec<Slot> = (0..count).map(Slot::Card).collect();
        deck.insert(self.cards.thermostat().min(count), Slot::Thermostat);
        deck.push(Slot::Settings);
        deck
    }

    /// Where `slot` is in the deck.
    pub fn place(&self, slot: Slot) -> usize {
        self.deck().iter().position(|s| *s == slot).unwrap_or(0)
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
    /// while a turn from the lit thermostat is under way; at rest too, dark,
    /// when it holds the screen.
    pub fn owns(&self) -> bool {
        self.mode.owning() || (self.mode == Mode::Home && (self.turn != 0 || self.holds_screen()))
    }

    /// Whether the UI keeps the display at rest (Home, black) rather than
    /// handing it back to nlclient's dial: unless Nest's own dial is the
    /// thermostat, and only with a screen patch that lets it go dark.
    pub fn holds_screen(&self) -> bool {
        self.rests && !self.nest_dial
    }

    /// Whether nlclient's own timing puts the display out (the UI not
    /// keeping it lit): at rest, and under the glance, which lasts as long
    /// as nlclient keeps it lit for whoever's in the room.
    pub fn resting(&self) -> bool {
        self.holds_screen() && self.turn == 0 && matches!(self.mode, Mode::Home | Mode::Glance)
    }

    /// The thermostat saw someone: across the room (its PIR), or close (its
    /// approach reading, about each second while they're there). Across the
    /// room, a lit display at rest (black) shows the glance again. Close, it
    /// shows what Home Assistant asked for, lit if dark; the cards stay
    /// while someone is close.
    pub fn sensed(&mut self, sensor: Sensor, now: Instant, lit: bool, out: &mut Vec<Effect>) {
        match sensor {
            Sensor::Motion => {
                if lit && self.mode == Mode::Home && self.turn == 0 && self.holds_screen()
                    && self.cards.far() == Far::Glance {
                    self.last_input = now;
                    self.go(Mode::Glance, out);
                }
                return;
            }
            Sensor::Near => self.near_at = Some(now),
        }
        if !matches!(self.mode, Mode::Home | Mode::Glance) || self.turn != 0 {
            return;
        }
        let near = self.cards.near();
        if near == Near::None || (near == Near::Glance && self.mode == Mode::Glance) {
            return;
        }
        if !lit {
            out.push(Effect::WakeDisplay);
            self.lighting = Some(now + LIGHTING);
        }
        self.last_input = now;
        if near == Near::Glance {
            self.go(Mode::Glance, out);
        } else {
            logf!("ui: someone close: waking");
            self.wake(out);
        }
    }

    /// Where the cards go, left alone: the glance, if Home Assistant shows
    /// one, until nlclient puts the display out (as it would its Farsight),
    /// rather than black on a lit display; else rest.
    fn left_alone(&self) -> Mode {
        if self.cards.far() == Far::Glance && self.holds_screen() {
            Mode::Glance
        } else {
            Mode::Home
        }
    }

    /// Whether nlclient's screen patch lets the display rest held.
    pub fn set_rests(&mut self, rests: bool, out: &mut Vec<Effect>) {
        self.rests = rests;
        self.dirty = true;
        self.sync(out);
    }

    /// The Nest shows one of its alerts (its log said so): its screen and
    /// ring until it's done, as Nest's own dial is.
    pub fn nest_alert(&mut self, now: Instant, out: &mut Vec<Effect>) {
        if matches!(self.mode, Mode::Idle | Mode::Native) {
            return;
        }
        logf!("ui: the Nest shows an alert: its screen");
        self.alert = None;
        self.alert_until = None;
        self.settings_open = false;
        self.last_input = now;
        self.go(Mode::Native, out);
        out.push(Effect::WakeDisplay);
    }

    /// Where the cards are as the ring turns: the card's place in the deck,
    /// plus how far the turn has gone towards the next (-1..1). None off
    /// the cards.
    pub fn position(&self) -> Option<f32> {
        let partial = self.turn as f32 / CARD_STEP as f32;
        match self.mode {
            Mode::Home if self.turn == 0 && self.holds_screen() => None,
            Mode::Home => Some(self.place(Slot::Thermostat) as f32 + partial),
            Mode::Chooser(index) => Some(index as f32 + partial),
            _ => None,
        }
    }

    /// Moves to `mode`.
    fn go(&mut self, mode: Mode, out: &mut Vec<Effect>) {
        if let Some(command) = self.pending.take() {
            self.sent_target(&command);
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
            // A touch on the glance: what waking shows.
            (Mode::Glance, Input::Turn(_) | Input::Press | Input::LongPress) => {
                self.wake(out);
                return;
            }
            // The thermostat opened: nlclient has the ring; its menu's Back,
            // or a long press, brings the cards back.
            (Mode::Native, Input::LongPress) => {
                out.push(Effect::Click);
                let back = if std::mem::take(&mut self.settings_open) { self.settings_at(Setting::More) } else { Mode::Chooser(self.place(Slot::Thermostat)) };
                self.go(back, out);
                return;
            }
            (Mode::Native, Input::NativeBack) => {
                self.go(Mode::Chooser(self.place(Slot::Thermostat)), out);
                return;
            }
            (Mode::Native, Input::SettingsDone) => {
                self.settings_open = false;
                let back = self.settings_at(Setting::More);
                self.go(back, out);
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
                    let at = if let Mode::Chooser(index) = self.mode { index } else { self.place(Slot::Thermostat) };
                    if at == 0 {
                        self.turn = self.turn.max(-END_GIVE);
                    }
                    if at + 1 == self.deck().len() {
                        self.turn = self.turn.min(END_GIVE);
                    }
                }
                self.movement += value;
                if matches!(self.mode, Mode::Volume { .. }) {
                    self.volume_at = Some(now);
                }
                if let Mode::Thermostat { target } = self.mode {
                    if self.nest.as_ref().is_some_and(|n| n.card.mode == "off") {
                        // Nothing to set while it's off: a turn meant as one
                        // asks for a mode, as the Nest does.
                        if self.turn.abs() < PROMPT_TURN {
                            return;
                        }
                        self.turn = 0;
                        out.push(Effect::Click);
                        self.mode = Mode::ModePrompt { target, selected: PROMPT_ON };
                        self.dirty = true;
                        self.sync(out);
                        return;
                    }
                }
                if let Mode::Thermostat { target } = self.mode {
                    self.set_at = Some(now);
                    self.turn_thermostat(target, out);
                    self.dirty = true;
                    self.sync(out);
                    return;
                }
                let step = match self.mode {
                    Mode::Home | Mode::Chooser(_) => CARD_STEP,
                    Mode::Menu { .. } => MENU_STEP,
                    Mode::LightList { .. } | Mode::OptionList { .. } | Mode::Members { .. }
                    | Mode::Settings { .. } | Mode::ThemePick { .. } | Mode::ModePrompt { .. } => ROW_STEP,
                    Mode::ClimateSet { .. } | Mode::Thermostat { .. } | Mode::SettingSet { .. } => CLIMATE_STEP,
                    _ => VALUE_STEP,
                };
                let steps = self.turn / step;
                if steps != 0 && step == MENU_STEP {
                    self.turn = 0;
                    if self.menu_stepped.is_some_and(|at| now.duration_since(at) < MENU_EVERY) {
                        return;
                    }
                    self.menu_stepped = Some(now);
                    self.turned(steps.signum(), out);
                } else if steps != 0 {
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
                self.go(Mode::Chooser(self.place(Slot::Thermostat)), out);
            }
            Wake::Thermostat => {
                let mode = self.thermostat();
                self.go(mode, out);
            }
            // A scene or a camera isn't opened: that would run it.
            Wake::Card(slot) if matches!(self.card(slot), Some(Card::Action(_) | Card::Camera(_))) => {
                self.go(Mode::Chooser(self.place(Slot::Card(slot))), out);
            }
            Wake::Card(slot) => self.open(self.place(Slot::Card(slot)), out),
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
            let from = if let Mode::Chooser(index) = self.mode { index } else { self.place(Slot::Thermostat) } as i32;
            let index = (from + steps).clamp(0, deck - 1);
            if index != from {
                out.push(Effect::Click);
                self.go(Mode::Chooser(index as usize), out);
            }
            return;
        }
        // Nothing to turn: a switch has no dimmer, and the colour dial stops
        // at either end, as the white one does.
        match self.mode {
            Mode::LightDim { slot, light, .. } => {
                if let Some(Card::Lights(card)) = self.card(slot) {
                    if !card.lights[light].dimmable {
                        return;
                    }
                }
            }
            Mode::LightHue { value, .. } if (steps < 0 && value == 0) || (steps > 0 && value >= HUE_MAX) => return,
            _ => {}
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
                    let value = (value as i32 + steps * HUE_STEP).clamp(0, HUE_MAX as i32) as u16;
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
            Mode::Thermostat { .. } => {}
            Mode::ModePrompt { target, selected } => {
                let selected = (selected as i32 + steps).clamp(PROMPT_ON as i32, PROMPT_CANCEL as i32) as usize;
                self.mode = Mode::ModePrompt { target, selected };
            }
            Mode::Settings { selected } => {
                let rows = self.settings().len() as i32;
                self.mode = Mode::Settings { selected: (selected as i32 + steps).clamp(0, rows - 1) as usize };
            }
            Mode::ThemePick { .. } => self.mode = Mode::ThemePick { fledge: steps > 0 },
            Mode::SettingSet { setting, value } => {
                let fahrenheit = self.nest.as_ref().is_some_and(|n| n.fahrenheit);
                if let Some((low, high, step)) = setting.range(fahrenheit) {
                    self.mode = Mode::SettingSet { setting, value: (value + steps as f64 * step).clamp(low, high) };
                }
            }
            Mode::Idle | Mode::Native | Mode::Alert { .. } | Mode::Camera { .. } | Mode::Glance => {}
        }
        self.dirty = true;
    }

    /// Opens the card at `index` in the deck: the thermostat hands nlclient
    /// the ring, Nest settings opens nlclient's menu, a device its controls.
    fn open(&mut self, index: usize, out: &mut Vec<Effect>) {
        match self.deck().get(index) {
            Some(Slot::Thermostat) => {
                let mode = self.thermostat();
                self.go(mode, out);
            }
            Some(Slot::Settings) => self.go(Mode::Settings { selected: 0 }, out),
            Some(&Slot::Card(slot)) => {
                let mode = match self.card(slot) {
                    // One light (or switch): its own screen. A switch's has no
                    // dimmer: held, it switches; pressed, its menu.
                    Some(Card::Lights(card)) if card.lights.len() == 1 => {
                        let light = &card.lights[0];
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
        out.push(Effect::Click);
        // What the ring had turned is spent: it doesn't carry on past a press.
        self.turn = 0;
        match self.mode.clone() {
            // Lit by something else, the thermostat showing: open it.
            Mode::Home => {
                let mode = self.thermostat();
                self.go(mode, out);
            }
            Mode::Chooser(index) => self.open(index, out),
            Mode::LightList { slot, selected } => {
                let Some(Card::Lights(card)) = self.card(slot) else { return };
                match card.lights.get(selected).cloned() {
                    None => self.go(Mode::Chooser(self.place(Slot::Card(slot))), out),
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
                _ => self.go(Mode::Chooser(self.place(Slot::Card(slot))), out),
            },
            Mode::Camera { slot } => self.go(Mode::Chooser(self.place(Slot::Card(slot))), out),
            Mode::OptionList { slot, selected } => {
                if let Some(Card::Options(card)) = self.cards.cards.get_mut(slot) {
                    if let Some(option) = card.options.get(selected).cloned() {
                        out.push(Effect::Send(Command::Select(card.id.clone(), option.clone())));
                        card.current = Some(option);
                    }
                }
                self.go(Mode::Chooser(self.place(Slot::Card(slot))), out);
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
            view @ Mode::Thermostat { .. } => {
                // Its target goes now; then what else it does.
                if let Some(command) = self.pending.take() {
                    self.sent_target(&command);
                    out.push(Effect::Send(command));
                }
                let Some(nest) = &self.nest else { return };
                let on = nest.card.mode != "off";
                let mut actions = vec![Action::Back];
                if on {
                    actions.push(Action::Eco(!nest.eco));
                }
                actions.push(Action::Power(!on));
                if nest.hot_water {
                    actions.push(Action::Boost(nest.hot_water_boost.is_none()));
                }
                actions.push(Action::NestDial);
                self.mode = Mode::Menu { view: Box::new(view), actions, selected: 0 };
            }
            Mode::ModePrompt { target, selected } => {
                if selected == PROMPT_ON {
                    if let Some(nest) = &mut self.nest {
                        let mode = nest.on_mode();
                        nest.card.mode = mode.to_owned();
                        self.nest_command("mode", mode, out);
                    }
                }
                // On its dial: to set, once on.
                let target = self.nest.as_ref().and_then(|n| n.card.target).unwrap_or(target);
                self.mode = Mode::Thermostat { target };
            }
            Mode::Settings { selected } => {
                let Some(&setting) = self.settings().get(selected) else { return };
                self.choose_setting(setting, out);
            }
            Mode::ThemePick { fledge } => {
                // Kept: the screens in it from now on.
                if fledge != self.fledge {
                    self.fledge = fledge;
                    out.push(Effect::Theme(fledge));
                }
                self.mode = self.settings_at(Setting::Theme);
            }
            Mode::SettingSet { setting, value } => {
                if let Some(nest) = &mut self.nest {
                    let celsius = nest.celsius(value);
                    let name = if setting == Setting::EcoHigh { "away_temperature_high" } else { "away_temperature_low" };
                    match setting {
                        Setting::EcoHigh => nest.eco_high = Some(value),
                        _ => nest.eco_low = Some(value),
                    }
                    self.nest_command(name, celsius.to_string(), out);
                }
                self.mode = self.settings_at(setting);
            }
            Mode::Idle | Mode::Native | Mode::Alert { .. } | Mode::Glance => {}
        }
        self.dirty = true;
    }

    /// A row of Settings pressed: a switch switched, a temperature's dial
    /// opened, the rest done.
    fn choose_setting(&mut self, setting: Setting, out: &mut Vec<Effect>) {
        let Some(nest) = self.nest.clone() else {
            // Not heard from nlclient yet: only its own Settings, and Done.
            match setting {
                Setting::Done => self.go(Mode::Chooser(self.deck().len() - 1), out),
                Setting::More => self.open_nest_settings(out),
                Setting::Theme => self.mode = Mode::ThemePick { fledge: self.fledge },
                _ => {}
            }
            return;
        };
        let switch = |on: bool| if on { "OFF" } else { "ON" };
        match setting {
            Setting::EcoLow | Setting::EcoHigh => {
                let fahrenheit = nest.fahrenheit;
                let (low, high, _) = setting.range(fahrenheit).expect("a temperature");
                let now = if setting == Setting::EcoLow { nest.eco_low } else { nest.eco_high };
                let value = now.unwrap_or(if setting == Setting::EcoLow { low } else { high }).clamp(low, high);
                self.mode = Mode::SettingSet { setting, value };
            }
            Setting::HomeAway => {
                self.nest_command("auto_away_enable", switch(nest.home_away), out);
                if let Some(n) = &mut self.nest { n.home_away = !nest.home_away; }
            }
            Setting::Learning => {
                self.nest_command("learning_mode", switch(nest.learning), out);
                if let Some(n) = &mut self.nest { n.learning = !nest.learning; }
            }
            Setting::EarlyOn => {
                self.nest_command("preconditioning_enabled", switch(nest.early_on), out);
                if let Some(n) = &mut self.nest { n.early_on = !nest.early_on; }
            }
            Setting::Sunblock => {
                self.nest_command("sunlight_correction_enabled", switch(nest.sunblock), out);
                if let Some(n) = &mut self.nest { n.sunblock = !nest.sunblock; }
            }
            Setting::HotWaterSchedule => {
                self.nest_command("hot_water_schedule", switch(nest.hot_water_schedule), out);
                if let Some(n) = &mut self.nest { n.hot_water_schedule = !nest.hot_water_schedule; }
            }
            Setting::Scale => {
                self.nest_command("temperature_scale", if nest.fahrenheit { "C" } else { "F" }, out);
            }
            Setting::NestDial => {
                self.nest_dial = !self.nest_dial;
                out.push(Effect::NestDial(self.nest_dial));
                self.dirty = true;
            }
            Setting::Theme => self.mode = Mode::ThemePick { fledge: self.fledge },
            Setting::Software => match nest.update {
                Some(_) => self.nest_command("update", "install", out),
                None => self.nest_command("check_update", "", out),
            },
            Setting::More => self.open_nest_settings(out),
            Setting::Done => self.go(Mode::Chooser(self.deck().len() - 1), out),
        }
    }

    /// nlclient's own Settings, from its menu on its home dial.
    fn open_nest_settings(&mut self, out: &mut Vec<Effect>) {
        self.go(Mode::Native, out);
        self.settings_open = true;
        out.push(Effect::OpenNestSettings);
    }

    /// The thermostat's ring turned (self.turn, counts since the last
    /// click): it clicks twice a step, its mark following the hand between
    /// (dial_offset, in steps); its target moves a whole step once the hand
    /// has gone one, as the Nest's does.
    fn turn_thermostat(&mut self, target: f64, out: &mut Vec<Effect>) {
        let Some(nest) = &self.nest else { return };
        let card = &nest.card;
        let clicks = self.turn / CLIMATE_CLICK;
        if clicks == 0 {
            return;
        }
        self.turn %= CLIMATE_CLICK;
        let mut offset = self.dial_offset + clicks as f64 * CLIMATE_CLICK as f64 / CLIMATE_STEP as f64;
        let mut next = target;
        while offset >= 1.0 - 1e-9 {
            next += card.step;
            offset -= 1.0;
        }
        while offset <= -1.0 + 1e-9 {
            next -= card.step;
            offset += 1.0;
        }
        if next >= card.max || next <= card.min {
            (next, offset) = (next.clamp(card.min, card.max), 0.0);
        }
        if next == target && offset == self.dial_offset {
            return;
        }
        out.push(Effect::Click);
        self.dial_offset = offset;
        self.mode = Mode::Thermostat { target: next };
        if next == target {
            return;
        }
        if nest.eco {
            // As on the Nest: setting a temperature leaves Eco.
            self.nest_command("preset", "none", out);
            if let Some(nest) = &mut self.nest {
                nest.eco = false;
                nest.card.preset = None;
            }
        }
        if let Some(nest) = &self.nest {
            self.pending = Some(Command::Nest("target".into(), nest.celsius(next).to_string()));
        }
    }

    /// The thermostat's target just sent: its card shows it at once.
    fn sent_target(&mut self, command: &Command) {
        if let (Command::Nest(name, _), Mode::Thermostat { target }, Some(nest)) = (command, &self.mode, &mut self.nest) {
            if name == "target" {
                nest.card.target = Some(on_step(*target, nest.card.step));
            }
        }
    }

    /// Does what a ring of actions' chosen item says.
    fn act(&mut self, view: Mode, action: Action, out: &mut Vec<Effect>) {
        match (action, view) {
            (Action::Back, view) => {
                if let Some(slot) = view.card_slot() {
                    self.go(Mode::Chooser(self.place(slot)), out);
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
                    self.mode = Mode::LightHue { slot, light, value: (card.lights[light].hue.unwrap_or(30) % 360).min(HUE_MAX) };
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
                self.go(Mode::Chooser(self.place(Slot::Card(slot))), out);
            }
            (Action::Members, Mode::LightDim { slot, .. }) => self.mode = Mode::Members { slot, selected: 0 },
            (Action::Eco(on), Mode::Thermostat { .. }) => {
                self.nest_command("preset", if on { "eco" } else { "none" }, out);
                if let Some(nest) = &mut self.nest {
                    nest.eco = on;
                    nest.card.preset = on.then(|| "eco".to_owned());
                }
                self.go(Mode::Chooser(self.place(Slot::Thermostat)), out);
            }
            (Action::Power(on), view @ Mode::Thermostat { .. }) => {
                if let Some(nest) = &mut self.nest {
                    let mode = if on { nest.on_mode() } else { "off" };
                    nest.card.mode = mode.to_owned();
                    self.nest_command("mode", mode, out);
                }
                // On or off, it stays open: off, its dial shows the temperature
                // now, and a turn asks for a mode.
                self.mode = view;
            }
            (Action::Boost(on), view @ Mode::Thermostat { .. }) => {
                let minutes = if on { BOOST_MINUTES.to_string() } else { "OFF".to_owned() };
                self.nest_command("hot_water_boost", minutes, out);
                if let Some(nest) = &mut self.nest {
                    nest.hot_water_boost = on.then_some(BOOST_MINUTES);
                }
                self.mode = view;
            }
            (Action::NestDial, _) => self.go(Mode::Native, out),
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
            // Held: as Cancel.
            Mode::ModePrompt { target, .. } => {
                out.push(Effect::Click);
                self.mode = Mode::Thermostat { target };
                self.dirty = true;
                return;
            }
            Mode::Menu { view, .. } => {
                out.push(Effect::Click);
                self.mode = *view;
                self.dirty = true;
                return;
            }
            Mode::Chooser(_) => Mode::Home,
            Mode::Settings { .. } => Mode::Chooser(self.deck().len() - 1),
            Mode::SettingSet { setting, .. } => self.settings_at(setting),
            Mode::ThemePick { .. } => self.settings_at(Setting::Theme),
            mode => match mode.card_slot() {
                Some(slot) => Mode::Chooser(self.place(slot)),
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
            // Lit by itself, as someone came near: for someone close, what
            // Home Assistant asked for them; across the room, the glance,
            // waking as a turn would, or nothing, as it asks.
            self.last_input = now;
            let close = self.near_at.is_some_and(|at| now.duration_since(at) < NEAR_LIT) && self.cards.near() != Near::None;
            match if close { if self.cards.near() == Near::Glance { Far::Glance } else { Far::Wake } } else { self.cards.far() } {
                Far::Glance => self.go(Mode::Glance, out),
                Far::Wake => self.wake(out),
                Far::Dark => {}
            }
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
        // Someone close keeps what shows, as a touch would.
        let idle = self.near_at.map_or(idle, |at| idle.min(now.duration_since(at)));
        // Left alone, the screen times out: the cards, or a device opened
        // from them, stay until then (going back is the hand's), and the
        // journey starts again from what waking shows.
        let timeout = self.cards.card_idle().unwrap_or(CARD_IDLE);
        let back = match self.mode {
            Mode::Glance if idle >= GLANCE_IDLE && !self.holds_screen() => Some(Mode::Home),
            Mode::Glance | Mode::Alert { .. } => None,
            Mode::Camera { .. } if idle >= timeout.max(CAMERA_IDLE) => Some(self.left_alone()),
            Mode::Camera { .. } => None,
            _ if idle >= timeout => Some(self.left_alone()),
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
        // The thermostat's target once the ring rests; Home Assistant's
        // devices as it turns.
        let settling = matches!(self.mode, Mode::Thermostat { .. }) && idle < SETTLE;
        if self.pending.is_some() && now.duration_since(self.last_sent) >= SEND_EVERY && !settling {
            self.last_sent = now;
            let command = self.pending.take().unwrap();
            self.sent_target(&command);
            out.push(Effect::Send(command));
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

        /// `rows` rows of a list, or icons of a ring of actions (an icon a
        /// step of the hand, as the ring takes them).
        fn rows(&mut self, rows: i32) -> Vec<Effect> {
            if matches!(self.machine.mode, Mode::Menu { .. }) {
                let mut out = Vec::new();
                for _ in 0..rows.abs() {
                    // A hand that lets each icon show.
                    self.now += MENU_EVERY;
                    out.extend(self.turn(rows.signum() * MENU_STEP));
                }
                out
            } else {
                self.turn(rows * ROW_STEP)
            }
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
    fn a_card_of_one_light_opens_its_dimmer_or_its_switch() {
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
        rig.press();
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "Back: to the cards, no list");
        rig.turn(CARD_STEP);
        assert!(Rig::sends(&rig.press()).is_empty(), "pressed: not switched");
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 1, light: 0, on: false, value: 100 }, "its own screen");
        assert!(!rig.turn(VALUE_STEP * 3).contains(&Effect::Click), "no dimmer to turn, no click");
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 1, light: 0, on: false, value: 100 });
        rig.press();
        let Mode::Menu { actions, .. } = rig.machine.mode.clone() else { panic!("pressed: its menu") };
        assert_eq!(actions, vec![Action::Back, Action::Switch(true)]);
        rig.input(Input::LongPress);
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 1, light: 0, on: false, value: 100 }, "held in the menu: out of it");
        assert_eq!(Rig::sends(&rig.input(Input::LongPress)), vec![Command::On("switch.fan".into())], "held: switched");
        assert_eq!(rig.machine.mode, Mode::LightDim { slot: 1, light: 0, on: true, value: 100 });
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
        rig.rows(2);
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
        rig.rows(3);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::LightHue { slot: 0, light: 0, value: 350 });
        rig.turn(VALUE_STEP * 4);
        assert_eq!(rig.machine.mode, Mode::LightHue { slot: 0, light: 0, value: HUE_MAX }, "it stops at the end");
        assert!(!rig.turn(VALUE_STEP).contains(&Effect::Click), "at the end: no click");
        rig.turn(-VALUE_STEP * 120);
        assert_eq!(rig.machine.mode, Mode::LightHue { slot: 0, light: 0, value: 0 }, "and at the start");
        rig.wait(CARD_IDLE - Duration::from_secs(1));
        assert!(matches!(rig.machine.mode, Mode::LightHue { .. }), "left alone: it stays");
        rig.press();
        assert!(matches!(rig.machine.mode, Mode::LightDim { .. }), "done: its dimmer");
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
        assert_eq!(rig.machine.mode, Mode::Home, "left: the screen times out");
    }

    fn deck_of(json: &[u8]) -> Rig {
        let mut rig = Rig::with(Cards::parse(json).unwrap());
        rig.lit = false;
        rig.press();
        rig.lit = true;
        rig
    }

    #[test]
    fn the_thermostat_goes_where_home_assistant_puts_it() {
        let two = r#"{"kind":"action","id":"scene.a","name":"A"},{"kind":"action","id":"scene.b","name":"B"}"#;
        let cards = |extra: &str| Cards::parse(format!(r#"{{"cards":[{two}]{extra}}}"#).as_bytes()).unwrap();
        let mut rig = Rig::with(cards(""));
        assert_eq!(rig.machine.deck(), vec![Slot::Thermostat, Slot::Card(0), Slot::Card(1), Slot::Settings], "first, unless it says");
        rig.machine.set_cards(cards(r#","thermostat":1"#), &mut vec![]);
        assert_eq!(rig.machine.deck(), vec![Slot::Card(0), Slot::Thermostat, Slot::Card(1), Slot::Settings]);
        rig.machine.set_cards(cards(r#","thermostat":9"#), &mut vec![]);
        assert_eq!(rig.machine.deck(), vec![Slot::Card(0), Slot::Card(1), Slot::Thermostat, Slot::Settings], "past the end: last before settings");

        let mut rig = deck_of(format!(r#"{{"cards":[{two}],"thermostat":1}}"#).as_bytes());
        assert_eq!(rig.machine.mode, Mode::Chooser(1), "waking to the chooser: on the thermostat, where it is");
        assert_eq!(rig.machine.position(), Some(1.0));
        rig.turn(-CARD_STEP);
        assert_eq!(rig.machine.mode, Mode::Chooser(0), "a card before it");
        rig.input(Input::LongPress);
        assert_eq!(rig.machine.mode, Mode::Home);
        assert_eq!(rig.machine.position(), Some(1.0), "the thermostat's dial is its card");
    }

    #[test]
    fn an_option_is_chosen_from_its_list() {
        let mut rig = deck_of(br#"{"cards":[{"kind":"options","id":"select.boost","name":"Hot water boost",
            "options":["Off","30 min","1 hour"],"current":"Off"}]}"#);
        rig.turn(CARD_STEP);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::OptionList { slot: 0, selected: 0 }, "on the current one");
        rig.rows(2);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Select("select.boost".into(), "1 hour".into())]);
        assert_eq!(rig.machine.mode, Mode::Chooser(1));
        assert!(matches!(&rig.machine.cards.cards[0], Card::Options(c) if c.current.as_deref() == Some("1 hour")));
        rig.press();
        rig.rows(9);
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
        rig.rows(2);
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
        rig.rows(1);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::Preset("climate.kitchen".into(), "eco".into())]);
        assert_eq!(rig.machine.mode, Mode::Chooser(1));
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        rig.rows(3);
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
        rig.rows(actions.len() as i32 - 1);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Members { slot: 0, selected: 0 });
        rig.rows(1);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::On("light.ceiling".into())], "each switched");
        rig.rows(1);
        rig.press();
        assert!(matches!(rig.machine.mode, Mode::LightDim { .. }), "Back: the group's dimmer");
    }

    #[test]
    fn coming_near_shows_the_glance_if_asked_for_and_a_touch_wakes() {
        let mut rig = Rig::with(Cards::parse(br#"{"cards":[{"kind":"climate","id":"climate.k","name":"K"}],
            "glance":{"offset":0}}"#).unwrap());
        rig.lit = false;
        rig.wait(Duration::from_secs(5));
        rig.lit = true;
        rig.tick();
        assert_eq!(rig.machine.mode, Mode::Glance, "lit by itself: the glance");
        rig.turn(CARD_STEP);
        assert_eq!(rig.machine.mode, Mode::Chooser(0), "a touch: what waking shows, the turn taken by it");
        rig.lit = false;
        rig.wait(Duration::from_secs(5));
        rig.lit = true;
        rig.tick();
        rig.wait(GLANCE_IDLE);
        assert_eq!(rig.machine.mode, Mode::Home, "untouched, it goes");
    }

    /// A rig whose screen patch lets the UI hold the display at rest.
    fn holding(cards: Cards) -> (Rig, Vec<Effect>) {
        let mut rig = Rig::with(cards);
        let mut out = vec![];
        rig.machine.set_rests(true, &mut out);
        (rig, out)
    }

    #[test]
    fn held_at_rest_the_display_is_never_handed_back_to_nlclient_s_dial() {
        let (mut rig, out) = holding(cards());
        assert_eq!(out, vec![Effect::Own(true)], "at rest, held");
        assert!(rig.machine.resting() && rig.machine.position().is_none(), "black, nlclient's timing putting it out");
        rig.turn(CARD_STEP);
        assert!(matches!(rig.machine.mode, Mode::Chooser(_)) && !rig.machine.resting(), "the cards keep it lit");
        let out = rig.wait(CARD_IDLE);
        assert_eq!(rig.machine.mode, Mode::Home);
        assert!(!out.contains(&Effect::Own(false)), "left alone: dark, not the Nest's dial");
        assert!(rig.machine.owns() && rig.machine.resting());
        rig.turn(CARD_STEP / 4);
        assert!(rig.machine.position().is_some() && !rig.machine.resting(), "a turn on the lit display: the cards follow");
    }

    #[test]
    fn held_at_rest_the_glance_lasts_as_long_as_the_display_is_lit() {
        let (mut rig, _) = holding(Cards::parse(br#"{"cards":[{"kind":"climate","id":"climate.k","name":"K"}],
            "glance":{"offset":0}}"#).unwrap());
        rig.lit = false;
        rig.wait(Duration::from_secs(5));
        rig.lit = true;
        rig.tick();
        assert_eq!(rig.machine.mode, Mode::Glance);
        assert!(rig.machine.resting(), "nlclient's own timing, as for Farsight");
        rig.wait(GLANCE_IDLE * 2);
        assert_eq!(rig.machine.mode, Mode::Glance, "still lit: still there");
        rig.lit = false;
        let out = rig.wait(Duration::from_secs(2));
        assert_eq!(rig.machine.mode, Mode::Home);
        assert!(!out.contains(&Effect::Own(false)));
    }

    #[test]
    fn with_nest_s_own_dial_the_display_is_handed_back_at_rest_as_before() {
        let mut rig = Rig::new();
        rig.machine.nest_dial = true;
        let mut out = vec![];
        rig.machine.set_rests(true, &mut out);
        assert!(out.is_empty() && !rig.machine.owns());
    }

    #[test]
    fn a_nest_alert_gets_the_nest_s_screen_and_the_cards_come_back_after() {
        let (mut rig, _) = holding(cards());
        rig.turn(CARD_STEP);
        let mut out = vec![];
        rig.machine.nest_alert(rig.now, &mut out);
        assert_eq!(rig.machine.mode, Mode::Native);
        assert!(out.contains(&Effect::Own(false)) && out.contains(&Effect::Hold(false)) && out.contains(&Effect::WakeDisplay));
        rig.dial = false;
        rig.wait(NATIVE_IDLE * 2);
        assert_eq!(rig.machine.mode, Mode::Native, "while the alert shows");
        rig.dial = true;
        let out = rig.wait(NATIVE_IDLE);
        assert_eq!(rig.machine.mode, Mode::Home);
        assert!(out.contains(&Effect::Own(true)), "its dial again: held, dark");
        let mut out = vec![];
        rig.machine.nest_alert(rig.now, &mut out);
        rig.machine.nest_alert(rig.now, &mut out);
        assert_eq!(out.iter().filter(|e| **e == Effect::Own(false)).count(), 1, "once");
    }

    fn presence(far: &str, near: &str) -> Cards {
        Cards::parse(format!(r#"{{"cards":[{{"kind":"climate","id":"climate.k","name":"K"}}],
            "glance":{{"offset":0}},"presence":{{"far":"{far}","near":"{near}"}},"card_idle":10}}"#).as_bytes()).unwrap()
    }

    #[test]
    fn across_the_room_the_glance_and_close_up_what_waking_shows() {
        let (mut rig, _) = holding(presence("glance", "wake"));
        rig.lit = false;
        rig.wait(Duration::from_secs(5));
        let mut out = vec![];
        rig.machine.sensed(Sensor::Motion, rig.now, false, &mut out);
        assert!(out.is_empty() && rig.machine.mode == Mode::Home, "the PIR alone lights nothing: nlclient's Farsight does");
        rig.lit = true;
        rig.tick();
        assert_eq!(rig.machine.mode, Mode::Glance, "lit across the room: the glance");
        rig.machine.sensed(Sensor::Near, rig.now, true, &mut out);
        assert!(matches!(rig.machine.mode, Mode::Chooser(_)), "close: the cards, untouched");
        for _ in 0..30 {
            rig.wait(Duration::from_secs(1));
            rig.machine.sensed(Sensor::Near, rig.now, true, &mut out);
        }
        assert!(matches!(rig.machine.mode, Mode::Chooser(_)), "they stay while someone's close");
        rig.wait(Duration::from_secs(11));
        assert_eq!(rig.machine.mode, Mode::Glance, "left alone, still lit: the glance (card_idle 10 s), not black");
        rig.lit = false;
        rig.wait(Duration::from_secs(1));
        assert_eq!(rig.machine.mode, Mode::Home);
    }

    #[test]
    fn motion_on_a_lit_black_display_brings_the_glance_back() {
        let (mut rig, _) = holding(presence("glance", "wake"));
        assert_eq!(rig.machine.mode, Mode::Home);
        let mut out = vec![];
        rig.machine.sensed(Sensor::Motion, rig.now, false, &mut out);
        assert_eq!(rig.machine.mode, Mode::Home, "dark: nlclient's Farsight lights it, and the glance shows then");
        rig.machine.sensed(Sensor::Motion, rig.now, true, &mut out);
        assert_eq!(rig.machine.mode, Mode::Glance, "lit and black: the glance");
        let (mut rig, _) = holding(presence("dark", "wake"));
        rig.machine.sensed(Sensor::Motion, rig.now, true, &mut out);
        assert_eq!(rig.machine.mode, Mode::Home, "across the room stays dark, as asked");
    }

    #[test]
    fn close_up_lights_a_dark_display_and_across_the_room_can_stay_dark() {
        let (mut rig, _) = holding(presence("dark", "wake"));
        rig.lit = false;
        rig.wait(Duration::from_secs(5));
        let mut out = vec![];
        rig.machine.sensed(Sensor::Near, rig.now, false, &mut out);
        assert!(out.contains(&Effect::WakeDisplay) && matches!(rig.machine.mode, Mode::Chooser(_)));
        let (mut rig, _) = holding(presence("dark", "none"));
        rig.lit = false;
        rig.wait(Duration::from_secs(5));
        rig.lit = true;
        rig.tick();
        assert_eq!(rig.machine.mode, Mode::Home, "lit across the room: black, as asked");
        rig.machine.sensed(Sensor::Near, rig.now, true, &mut out);
        assert_eq!(rig.machine.mode, Mode::Home, "close: nothing, as asked");
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
        assert!(rig.press().is_empty(), "Settings: our own, the display kept");
        assert_eq!(rig.machine.mode, Mode::Settings { selected: 0 });
    }

    #[test]
    fn more_nest_settings_opens_nlclient_s_and_comes_back_to_ours() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP * 4);
        rig.press();
        let more = rig.machine.settings().iter().position(|s| *s == Setting::More).unwrap();
        rig.rows(more as i32);
        assert_eq!(rig.press(), vec![Effect::Own(false), Effect::Hold(false), Effect::OpenNestSettings],
                   "nlclient's menu, on to its Settings");
        assert!(rig.machine.settings_open);
        rig.wait(NATIVE_IDLE * 3);
        assert_eq!(rig.machine.mode, Mode::Native, "staying while in its Settings, though its dial shows");
        rig.input(Input::SettingsDone);
        assert_eq!(rig.machine.mode, Mode::Settings { selected: more }, "left: ours again, on More");
        assert!(!rig.machine.settings_open);
        rig.press();
        rig.long();
        assert_eq!(rig.machine.mode, Mode::Settings { selected: more }, "a long press there: ours too");
        rig.long();
        assert_eq!(rig.machine.mode, Mode::Chooser(4), "and back out to the cards");
    }

    fn nest(extra: serde_json::Value) -> Nest {
        let mut state = serde_json::json!({
            "mode": "heat", "action": "idle", "target": 20.0, "current_temperature": 19.5, "preset": "none",
            "temperature_scale": "C", "can_heat": true, "can_cool": false, "has_hot_water_control": true,
            "hot_water_boost": "off", "away_temperature_low": 12.0, "learning_mode": true, "version": "v0.4.9",
            "update": null});
        for (k, v) in extra.as_object().unwrap() {
            state[k] = v.clone();
        }
        Nest::from_state(state.as_object().unwrap())
    }

    fn nest_rig() -> Rig {
        let mut rig = Rig::woken();
        rig.machine.set_nest(nest(serde_json::json!({})));
        rig
    }

    fn sent(effects: &[Effect]) -> Vec<(String, String)> {
        effects.iter().filter_map(|e| match e {
            Effect::Send(Command::Nest(name, arg)) => Some((name.clone(), arg.clone())),
            _ => None,
        }).collect()
    }

    #[test]
    fn the_thermostat_card_sets_its_target_once_the_ring_rests() {
        let mut rig = nest_rig();
        assert!(rig.press().is_empty(), "our own dial: the display and ring kept");
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 });
        rig.turn(CLIMATE_STEP * 3);
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 21.5 });
        assert!(sent(&rig.wait(SETTLE / 2)).is_empty(), "not while it turns");
        assert_eq!(sent(&rig.wait(SETTLE)), vec![("target".into(), "21.5".into())]);
        assert_eq!(rig.machine.nest.as_ref().unwrap().card.target, Some(21.5), "its card shows it at once");
        rig.turn(-CLIMATE_STEP * 40);
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 9.0 }, "no lower than the Nest goes");
        assert_eq!(sent(&rig.long()), vec![("target".into(), "9".into())], "going back sends it");
        assert_eq!(rig.machine.mode, Mode::Chooser(0));
    }

    #[test]
    fn the_thermostat_s_ring_clicks_between_its_steps_and_its_mark_follows_the_hand() {
        let mut rig = nest_rig();
        rig.press();
        let effects = rig.input_with_clicks(Input::Turn(CLIMATE_CLICK));
        assert_eq!(effects, vec![Effect::Click], "a click, nothing sent");
        assert_eq!((&rig.machine.mode, rig.machine.dial_offset), (&Mode::Thermostat { target: 20.0 }, 0.5),
                   "still 20, the mark half way to the next step");
        assert!(sent(&rig.wait(SETTLE * 2)).is_empty(), "nothing sent");
        assert_eq!(rig.machine.dial_offset, 0.5, "at rest, the mark stays where the hand left it");
        rig.input_with_clicks(Input::Turn(CLIMATE_CLICK));
        rig.input_with_clicks(Input::Turn(CLIMATE_CLICK));
        assert_eq!(sent(&rig.wait(SETTLE * 2)), vec![("target".into(), "20.5".into())], "a whole step: sent");
    }

    #[test]
    fn a_ring_of_actions_steps_an_icon_at_a_time_however_quick_the_hand() {
        let mut rig = nest_rig();
        rig.press();
        rig.press();
        assert!(matches!(rig.machine.mode, Mode::Menu { selected: 0, .. }));
        rig.turn(MENU_STEP * 3);
        assert!(matches!(rig.machine.mode, Mode::Menu { selected: 1, .. }), "one reading, one icon: {:?}", rig.machine.mode);
        for _ in 0..5 {
            // Readings 30 ms apart (each input moves the rig's clock 20 ms).
            rig.now += Duration::from_millis(10);
            rig.turn(MENU_STEP);
        }
        assert!(matches!(rig.machine.mode, Mode::Menu { selected: 1, .. }), "a spin quicker than an icon shows: still it: {:?}", rig.machine.mode);
        rig.now += MENU_EVERY;
        rig.turn(MENU_STEP - 1);
        assert!(matches!(rig.machine.mode, Mode::Menu { selected: 1, .. }), "the rest of a step didn't carry over");
        rig.turn(1);
        assert!(matches!(rig.machine.mode, Mode::Menu { selected: 2, .. }), "a whole step, in time: the next");
    }

    #[test]
    fn turning_the_thermostat_off_keeps_it_open() {
        let mut rig = nest_rig();
        rig.press();
        rig.press();
        rig.rows(2);
        let Mode::Menu { actions, selected, .. } = rig.machine.mode.clone() else { panic!() };
        assert_eq!(actions[selected], Action::Power(false));
        assert_eq!(sent(&rig.press()), vec![("mode".into(), "off".into())]);
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 }, "still on the thermostat");
    }

    #[test]
    fn turning_the_off_thermostat_asks_for_a_mode_as_the_nest_does() {
        let mut rig = nest_rig();
        rig.machine.set_nest(nest(serde_json::json!({"mode": "off"})));
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 });
        rig.turn(PROMPT_TURN - 1);
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 }, "a hand coming off the ring: no question");
        rig.press();
        rig.input(Input::LongPress);
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 }, "its menu, and out of it");
        rig.turn(PROMPT_TURN - 1);
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 }, "nor what was turned before a press");
        assert!(sent(&rig.turn(1)).is_empty(), "nothing set");
        assert_eq!(rig.machine.mode, Mode::ModePrompt { target: 20.0, selected: PROMPT_ON }, "Heat, Off, Cancel; Heat chosen");
        rig.rows(5);
        assert_eq!(rig.machine.mode, Mode::ModePrompt { target: 20.0, selected: PROMPT_CANCEL }, "no further than Cancel");
        assert!(sent(&rig.press()).is_empty(), "Cancel: nothing sent");
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 }, "back on its dial, still off");
        rig.turn(PROMPT_TURN);
        rig.rows(1);
        assert_eq!(rig.machine.mode, Mode::ModePrompt { target: 20.0, selected: PROMPT_OFF });
        assert!(sent(&rig.press()).is_empty(), "Off: left off");
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 });
        rig.turn(PROMPT_TURN);
        rig.input(Input::LongPress);
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 }, "held: as Cancel");
        rig.turn(PROMPT_TURN);
        rig.rows(-3);
        assert_eq!(rig.machine.mode, Mode::ModePrompt { target: 20.0, selected: PROMPT_ON }, "the question isn't a row");
        assert_eq!(sent(&rig.press()), vec![("mode".into(), "heat".into())], "Heat: on");
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 20.0 }, "on its dial to set");
        rig.turn(CLIMATE_STEP * 2);
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 21.0 }, "on, the ring sets it");
    }

    #[test]
    fn turning_in_eco_leaves_it_and_its_menu_switches_it() {
        let mut rig = nest_rig();
        rig.machine.set_nest(nest(serde_json::json!({"preset": "eco"})));
        rig.press();
        assert_eq!(sent(&rig.turn(CLIMATE_STEP)), vec![("preset".into(), "none".into())]);
        assert!(!rig.machine.nest.as_ref().unwrap().eco);
        let effects = rig.press();
        assert_eq!(sent(&effects), vec![("target".into(), "20.5".into())], "the press sends the target first");
        let Mode::Menu { actions, .. } = &rig.machine.mode else { panic!("{:?}", rig.machine.mode) };
        assert_eq!(actions, &vec![Action::Back, Action::Eco(true), Action::Power(false), Action::Boost(true), Action::NestDial]);
        rig.rows(1);
        assert_eq!(sent(&rig.press()), vec![("preset".into(), "eco".into())]);
        assert_eq!(rig.machine.mode, Mode::Chooser(0), "Eco: back to the cards");
        rig.press();
        rig.press();
        rig.rows(3);
        assert_eq!(sent(&rig.press()), vec![("hot_water_boost".into(), "60".into())]);
        assert_eq!(rig.machine.nest.as_ref().unwrap().hot_water_boost, Some(60));
        rig.press();
        rig.rows(4);
        assert_eq!(rig.press(), vec![Effect::Own(false), Effect::Hold(false)], "Nest's own dial");
        assert_eq!(rig.machine.mode, Mode::Native);
    }

    #[test]
    fn nest_s_own_dial_stands_in_for_the_card_when_chosen_or_unknown() {
        let mut rig = Rig::woken();
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Native, "no state from nlclient yet: its dial");
        let mut rig = nest_rig();
        rig.machine.nest_dial = true;
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Native, "chosen in Settings");
    }

    #[test]
    fn settings_switch_and_set_and_say_so() {
        let mut rig = nest_rig();
        rig.turn(CARD_STEP * 4);
        rig.press();
        let rows = rig.machine.settings();
        assert_eq!(rows, vec![Setting::EcoLow, Setting::HomeAway, Setting::Learning, Setting::EarlyOn, Setting::Sunblock,
                              Setting::HotWaterSchedule, Setting::Theme, Setting::Scale, Setting::NestDial, Setting::Software,
                              Setting::More, Setting::Done], "heating only, with hot water");
        assert!(rig.press().is_empty() && rig.machine.mode == Mode::SettingSet { setting: Setting::EcoLow, value: 12.0 });
        rig.turn(CLIMATE_STEP * 3);
        assert_eq!(sent(&rig.press()), vec![("away_temperature_low".into(), "13.5".into())]);
        assert_eq!(rig.machine.mode, Mode::Settings { selected: 0 });
        assert_eq!(rig.machine.nest.as_ref().unwrap().eco_low, Some(13.5));
        rig.rows(2);
        assert_eq!(sent(&rig.press()), vec![("learning_mode".into(), "OFF".into())], "Auto-Schedule off");
        assert!(!rig.machine.nest.as_ref().unwrap().learning);
        rig.rows(4);
        assert!(rig.press().is_empty() && rig.machine.mode == Mode::ThemePick { fledge: false }, "the two themes");
        rig.rows(1);
        assert_eq!(rig.machine.mode, Mode::ThemePick { fledge: true });
        assert_eq!(rig.press(), vec![Effect::Theme(true)], "Fledge's kept");
        assert!(rig.machine.fledge);
        assert_eq!(rig.machine.mode, Mode::Settings { selected: 6 }, "back on its row");
        rig.rows(2);
        assert_eq!(rig.press(), vec![Effect::NestDial(true)]);
        assert!(rig.machine.nest_dial);
        rig.rows(1);
        assert_eq!(sent(&rig.press()), vec![("check_update".into(), String::new())], "no update: look for one");
        rig.machine.set_nest(nest(serde_json::json!({"update": "v0.5.0"})));
        assert_eq!(sent(&rig.press()), vec![("update".into(), "install".into())], "one found: install it");
        rig.rows(9);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Chooser(4), "Done: the cards, on Settings");
    }

    #[test]
    fn a_fahrenheit_thermostat_turns_by_degrees_and_sends_celsius() {
        let mut rig = nest_rig();
        rig.machine.set_nest(nest(serde_json::json!({"temperature_scale": "F", "target": 68.0})));
        rig.press();
        rig.turn(CLIMATE_STEP * 2);
        assert_eq!(rig.machine.mode, Mode::Thermostat { target: 70.0 });
        assert_eq!(sent(&rig.long()), vec![("target".into(), "21".into())]);
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
        rig.rows(1);
        assert_eq!(Rig::sends(&rig.press()), vec![Command::On("switch.fan".into())], "on/off: switched in place");
        assert!(matches!(&rig.machine.cards.cards[0], Card::Lights(c) if c.lights[1].on), "shown on at once");
        rig.rows(5);
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
        rig.rows(1);
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
    fn an_opened_card_stays_until_the_screen_times_out_then_starts_again() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        rig.press();
        let menu = rig.machine.mode.clone();
        assert!(matches!(menu, Mode::Menu { .. }));
        rig.wait(CARD_IDLE - Duration::from_secs(1));
        assert_eq!(rig.machine.mode, menu, "left alone, it stays: going back is the hand's");
        rig.wait(Duration::from_secs(1));
        assert_eq!(rig.machine.mode, Mode::Home, "the screen times out");
        rig.lit = false;
        rig.tick();
        rig.press();
        rig.lit = true;
        rig.tick();
        assert_eq!(rig.machine.mode, Mode::Chooser(0), "woken again: from the start");
    }

    #[test]
    fn a_camera_stays_at_least_its_minute() {
        let mut rig = Rig::with(Cards::parse(br#"{"cards":[{"kind":"camera","id":"camera.front","name":"Front"}]}"#).unwrap());
        rig.lit = false;
        rig.press();
        rig.lit = true;
        rig.turn(CARD_STEP);
        rig.press();
        assert_eq!(rig.machine.mode, Mode::Camera { slot: 0 });
        rig.wait(CARD_IDLE);
        assert_eq!(rig.machine.mode, Mode::Camera { slot: 0 });
        rig.wait(CAMERA_IDLE - CARD_IDLE);
        assert_eq!(rig.machine.mode, Mode::Home);
    }

    #[test]
    fn a_turned_off_light_comes_on_from_low() {
        let mut rig = Rig::woken();
        rig.turn(CARD_STEP);
        rig.press();
        rig.press();
        rig.press();
        rig.rows(1);
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
        rig.rows(1);
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
        rig.rows(5);
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
        assert_eq!(clicks(rig.input_with_clicks(Input::Press)), 1, "Settings are ours");
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
