//! The Nest's home screen cards: Home Assistant on the thermostat.
//!
//! Turning the ring moves from the thermostat to a card for each thing Home
//! Assistant's nest_ui blueprint chose (lights, other thermostats, media
//! players) and on to Nest settings; pressing controls the card. The
//! thermostat itself is nlclient's own screen, untouched: a press there hands
//! the ring to nlclient, which sets the temperature as it always has, and Nest
//! settings opens nlclient's menus. nlclient keeps the heating, learning and
//! everything else throughout.
//!
//! nestlocal holds the ring and button (grabbed) while the thermostat's own
//! screen shows, so a turn moves to the cards rather than the temperature.
//! On a card it owns the display (device.rs) and keeps nlclient's display
//! awake with nudges. Left alone, the cards go back to the thermostat; a
//! display that went dark wakes onto the thermostat, as before. With no cards
//! from Home Assistant, or no nlclient (Doom), it leaves everything alone.

mod device;
mod icons;
mod model;
mod paint;
mod screens;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use model::Command;
use model::{Card, Cards};
use screens::{Position, Screen};

use crate::logf;

#[derive(Default)]
pub struct Ui {
    /// Home Assistant's cards and a count of their changes.
    cards: Mutex<(u64, Cards)>,
}

impl Ui {
    /// Takes Home Assistant's cards (<base>/set/ui).
    pub fn configure(&self, payload: &[u8]) -> Result<(), String> {
        let cards = Cards::parse(payload)?;
        let mut held = self.cards.lock().unwrap();
        if held.1 != cards {
            held.0 += 1;
            held.1 = cards;
        }
        Ok(())
    }

    fn generation(&self) -> u64 {
        self.cards.lock().unwrap().0
    }

    fn snapshot(&self) -> (u64, Cards) {
        self.cards.lock().unwrap().clone()
    }
}

/// What the UI sends Home Assistant.
pub trait Sender: Send + Sync + 'static {
    fn send(&self, command: &Command);
}

/// Starts the UI on a gen 3 unit: it waits for cards from Home Assistant.
pub fn start(ui: Arc<Ui>, sender: Arc<dyn Sender>) {
    let spawned = std::thread::Builder::new().name("ui".into()).spawn(move || {
        let fonts = match paint::Fonts::load(paint::FONT_DIR) {
            Ok(fonts) => fonts,
            Err(e) => return logf!("ui: {e}: no home screen cards"),
        };
        let (display, input) = match (device::Display::open(), device::Input::open()) {
            (Ok(display), Ok(input)) => (display, input),
            (Err(e), _) | (_, Err(e)) => return logf!("ui: {e}: no home screen cards"),
        };
        // An owner file from before a restart names a pid that may be reused.
        device::own(false);
        Runtime::new(ui, sender, fonts, display, input).run();
    });
    if let Err(e) = spawned {
        logf!("ui: {e}");
    }
}

/// Ring counts per step, by what a step moves: a card, a list row, a value.
/// REL_DIAL reports the ADC's raw counts; a step is a deliberate movement.
const CARD_STEP: i32 = 500;
const ROW_STEP: i32 = 250;
const VALUE_STEP: i32 = 60;
/// A pause this long ends a movement (for the log); this long, its counts
/// towards the next step.
const GESTURE: Duration = Duration::from_millis(400);
const TURN_KEPT: Duration = Duration::from_millis(1500);
/// How long each view waits, untouched, before going back a level.
const CONTROL_IDLE: Duration = Duration::from_secs(8);
const LIST_IDLE: Duration = Duration::from_secs(12);
const CARD_IDLE: Duration = Duration::from_secs(20);
/// Back to the cards this long after the last input on nlclient's screens,
/// once nlclient shows its home dial again.
const NATIVE_IDLE: Duration = Duration::from_secs(5);
/// Keeps nlclient's display awake well within its own timeout (about 12 s).
const NUDGE: Duration = Duration::from_secs(4);
/// The fewest milliseconds between commands while turning.
const SEND_EVERY: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    Thermostat,
    Card(usize),
    Settings,
}

#[derive(Clone, Debug, PartialEq)]
enum Mode {
    /// No cards, or no nlclient: everything as stock.
    Idle,
    /// nlclient's screen with the ring grabbed: the thermostat card.
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

struct Runtime {
    ui: Arc<Ui>,
    sender: Arc<dyn Sender>,
    fonts: paint::Fonts,
    display: device::Display,
    input: device::Input,
    mode: Mode,
    generation: u64,
    cards: Cards,
    /// nlclient's frame from before the UI owned the display.
    saved: Option<Vec<u8>>,
    dirty: bool,
    last_input: Instant,
    last_nudge: Instant,
    last_check: Instant,
    turn: i32,
    turned_at: Instant,
    /// Counts of the current movement, for tuning the steps.
    movement: i32,
    pending: Option<Command>,
    last_sent: Instant,
    nlclient: bool,
    holding: bool,
}

impl Runtime {
    fn new(ui: Arc<Ui>, sender: Arc<dyn Sender>, fonts: paint::Fonts, display: device::Display,
           input: device::Input) -> Runtime {
        let now = Instant::now();
        Runtime {
            ui, sender, fonts, display, input,
            mode: Mode::Idle, generation: u64::MAX, cards: Cards::default(), saved: None, dirty: false,
            last_input: now, last_nudge: now, last_check: now - Duration::from_secs(10),
            turn: 0, turned_at: now, movement: 0, pending: None, last_sent: now, nlclient: false,
            holding: false,
        }
    }

    fn deck(&self) -> Vec<Slot> {
        let mut deck = vec![Slot::Thermostat];
        deck.extend((0..self.cards.cards.len()).map(Slot::Card));
        deck.push(Slot::Settings);
        deck
    }

    fn run(mut self) {
        logf!("ui: ready for home screen cards");
        loop {
            let events = self.input.wait(Duration::from_millis(50));
            let now = Instant::now();
            if now.duration_since(self.last_check) >= Duration::from_secs(2) {
                self.last_check = now;
                self.nlclient = !crate::pids("nlclient").is_empty();
            }
            if self.ui.generation() != self.generation {
                let (generation, cards) = self.ui.snapshot();
                self.generation = generation;
                self.cards = cards;
                self.revalidate();
            }
            let active = self.nlclient && !self.cards.cards.is_empty();
            if !active {
                if self.mode != Mode::Idle {
                    logf!("ui: {}", if self.nlclient { "no cards: the thermostat as it was" } else { "nlclient stopped: standing aside" });
                    self.go(Mode::Idle);
                }
                continue;
            }
            if self.mode == Mode::Idle {
                logf!("ui: {} home screen cards", self.cards.cards.len());
                // nlclient may be anywhere; the cards start on its home dial.
                self.go(Mode::Native);
                self.last_input = now - NATIVE_IDLE;
            }
            for event in events {
                self.event(event, now);
            }
            self.tick(now);
            if self.dirty && device::owned() {
                self.draw();
            }
            self.flush(now, false);
        }
    }

    /// Cards changed under the mode: keep what still exists.
    fn revalidate(&mut self) {
        let count = self.cards.cards.len();
        let slot_ok = |slot: usize, cards: &Cards| slot < cards.cards.len();
        let fine = match &self.mode {
            Mode::Card(index) => *index < count + 2,
            Mode::LightList { slot, .. } | Mode::ClimateSet { slot, .. } | Mode::Volume { slot, .. } => slot_ok(*slot, &self.cards),
            Mode::LightDim { slot, light, .. } => match self.cards.cards.get(*slot) {
                Some(Card::Lights(card)) => *light < card.lights.len(),
                _ => false,
            },
            _ => true,
        };
        if !fine {
            self.go(Mode::Home);
        }
        self.dirty = true;
    }

    fn owning(mode: &Mode) -> bool {
        !matches!(mode, Mode::Idle | Mode::Home | Mode::Native)
    }

    /// Moves to `mode`, handing the display and the ring over as it needs.
    fn go(&mut self, mode: Mode) {
        let (was, will) = (Self::owning(&self.mode), Self::owning(&mode));
        if will && !was {
            self.saved = Some(self.display.save());
            device::own(true);
            self.last_nudge = Instant::now() - NUDGE;
        }
        if was && !will {
            if let Some(saved) = self.saved.take() {
                self.display.restore(&saved);
            }
            device::own(false);
        }
        let hold = !matches!(mode, Mode::Idle | Mode::Native);
        if hold != self.holding || will != was {
            logf!("ui: {} the ring, {} the display", if hold { "holds" } else { "leaves nlclient" },
                  if will { "owns" } else { "nlclient has" });
        }
        self.holding = hold;
        self.input.grab(hold);
        self.flush(Instant::now(), true);
        self.mode = mode;
        self.dirty = true;
    }

    fn event(&mut self, event: device::Event, now: Instant) {
        let previous = self.last_input;
        self.last_input = now;
        if self.mode == Mode::Native {
            return;
        }
        if self.mode == Mode::Home && !device::lit() {
            // Asleep: the turn or press only wakes nlclient's display.
            self.input.nudge();
            return;
        }
        match (event.kind, event.value) {
            (device::EV_KEY, 1) if event.code == device::KEY_ENTER => self.press(),
            (device::EV_REL, value) if event.code == device::REL_DIAL && value != 0 => {
                let pause = now.duration_since(previous);
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
                    _ => VALUE_STEP,
                };
                let steps = self.turn / step;
                if steps != 0 {
                    self.turn -= steps * step;
                    self.turned(steps);
                }
            }
            _ => {}
        }
        self.turned_at = now;
    }

    fn card(&self, slot: usize) -> Option<&Card> {
        self.cards.cards.get(slot)
    }

    fn turned(&mut self, steps: i32) {
        let deck = self.deck();
        match self.mode.clone() {
            Mode::Home | Mode::Card(_) => {
                let from = if let Mode::Card(index) = self.mode { index } else { 0 };
                let index = (from as i32 + steps).rem_euclid(deck.len() as i32) as usize;
                self.go(if index == 0 { Mode::Home } else { Mode::Card(index) });
            }
            Mode::LightList { slot, selected } => {
                let rows = match self.card(slot) {
                    Some(Card::Lights(card)) => card.lights.len() + 1,
                    _ => 1,
                };
                let selected = (selected as i32 + steps).clamp(0, rows as i32 - 1) as usize;
                self.mode = Mode::LightList { slot, selected };
            }
            Mode::LightDim { slot, light, on, value } => {
                let start = if on { value as i32 } else { 0 };
                let value = (start + steps * 2).clamp(1, 100) as u8;
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
                    let value = (value as i32 + steps * 2).clamp(0, 100) as u8;
                    self.pending = Some(Command::Volume(card.id.clone(), value));
                    self.mode = Mode::Volume { slot, value };
                }
            }
            Mode::Idle | Mode::Native => {}
        }
        self.dirty = true;
    }

    fn press(&mut self) {
        let deck = self.deck();
        match self.mode.clone() {
            Mode::Home => self.go(Mode::Native),
            Mode::Card(index) => match deck.get(index) {
                Some(Slot::Settings) => {
                    self.go(Mode::Native);
                    // nlclient is on its home dial: a press opens its menu,
                    // once it has its frame back on the display.
                    std::thread::sleep(Duration::from_millis(300));
                    self.input.press();
                }
                Some(Slot::Card(slot)) => {
                    let slot = *slot;
                    match self.card(slot) {
                        Some(Card::Lights(_)) => self.mode = Mode::LightList { slot, selected: 0 },
                        Some(Card::Climate(card)) => {
                            let target = card.target.unwrap_or((card.min + card.max) / 2.0);
                            self.mode = Mode::ClimateSet { slot, target };
                        }
                        Some(Card::Media(card)) => {
                            self.mode = Mode::Volume { slot, value: card.volume.unwrap_or(30) };
                        }
                        None => {}
                    }
                }
                _ => {}
            },
            Mode::LightList { slot, selected } => {
                let Some(Card::Lights(card)) = self.card(slot) else { return };
                let Some(light) = card.lights.get(selected).cloned() else {
                    self.mode = Mode::Card(slot + 1);
                    self.dirty = true;
                    return;
                };
                if light.dimmable {
                    self.mode = Mode::LightDim { slot, light: selected, on: light.on,
                                                 value: light.brightness.unwrap_or(100) };
                } else {
                    self.toggle_light(slot, selected, !light.on);
                }
            }
            Mode::LightDim { slot, light, on, value } => {
                self.toggle_light(slot, light, !on);
                self.mode = Mode::LightDim { slot, light, on: !on, value };
            }
            Mode::ClimateSet { slot, .. } => {
                self.flush(Instant::now(), true);
                self.mode = Mode::Card(slot + 1);
            }
            Mode::Volume { slot, .. } => {
                if let Some(Card::Media(card)) = self.cards.cards.get_mut(slot) {
                    self.sender.send(&Command::PlayPause(card.id.clone()));
                    card.state = if card.state == "playing" { "paused".into() } else { "playing".into() };
                }
            }
            Mode::Idle | Mode::Native => {}
        }
        self.dirty = true;
    }

    /// Switches a light, showing it switched until Home Assistant says.
    fn toggle_light(&mut self, slot: usize, index: usize, on: bool) {
        self.pending = None;
        if let Some(Card::Lights(card)) = self.cards.cards.get_mut(slot) {
            let light = &mut card.lights[index];
            light.on = on;
            let command = if on { Command::On(light.id.clone()) } else { Command::Off(light.id.clone()) };
            self.sender.send(&command);
        }
    }

    fn tick(&mut self, now: Instant) {
        let idle = now.duration_since(self.last_input);
        match self.mode.clone() {
            Mode::Native => {
                // Back to the cards once nlclient is home: dark, or its dial.
                if !device::lit() || (idle >= NATIVE_IDLE && self.home_dial()) {
                    self.go(Mode::Home);
                }
                return;
            }
            Mode::Idle | Mode::Home => return,
            _ => {}
        }
        if !device::lit() {
            // Something else put the display out: give it back.
            self.go(Mode::Home);
            return;
        }
        if now.duration_since(self.last_nudge) >= NUDGE {
            self.last_nudge = now;
            self.input.nudge();
            crate::wake::keep_awake_for(30);
        }
        let back = match self.mode.clone() {
            Mode::Card(_) if idle >= CARD_IDLE => Some(Mode::Home),
            Mode::LightList { slot, .. } if idle >= LIST_IDLE => Some(Mode::Card(slot + 1)),
            Mode::LightDim { slot, light, .. } if idle >= CONTROL_IDLE => Some(Mode::LightList { slot, selected: light }),
            Mode::ClimateSet { slot, .. } | Mode::Volume { slot, .. } if idle >= CONTROL_IDLE => Some(Mode::Card(slot + 1)),
            _ => None,
        };
        if let Some(mode) = back {
            self.last_input = now;
            self.go(mode);
        }
    }

    /// Whether nlclient shows its home dial: its ring lit around the edge.
    fn home_dial(&mut self) -> bool {
        let page = self.display.save();
        let stride = page.len() / 480;
        let (mut lit, mut total) = (0, 0);
        for angle in (0..360).step_by(3) {
            for radius in (205..232).step_by(4) {
                let (sin, cos) = (angle as f32).to_radians().sin_cos();
                let (x, y) = ((240.0 + radius as f32 * cos) as usize, (240.0 + radius as f32 * sin) as usize);
                let at = y * stride + x * 4;
                total += 1;
                lit += (page[at..at + 3].iter().max().copied().unwrap_or(0) > 60) as u32;
            }
        }
        lit as f32 / total as f32 > 0.2
    }

    /// Sends what turning asked for, at most every SEND_EVERY, or now.
    fn flush(&mut self, now: Instant, all: bool) {
        if self.pending.is_none() || (!all && now.duration_since(self.last_sent) < SEND_EVERY) {
            return;
        }
        if let Some(command) = self.pending.take() {
            self.sender.send(&command);
            self.last_sent = now;
        }
    }

    fn draw(&mut self) {
        self.dirty = false;
        let deck = self.deck();
        let position = |index: usize| Position { index, count: deck.len() };
        let card = |slot: usize| self.cards.cards.get(slot);
        let (screen, at) = match &self.mode {
            Mode::Card(index) => {
                let at = position(*index);
                match deck.get(*index) {
                    Some(Slot::Card(slot)) => match card(*slot) {
                        Some(Card::Lights(card)) => (Screen::LightsCard { card }, at),
                        Some(Card::Climate(card)) => (Screen::ClimateCard { card, target: None }, at),
                        Some(Card::Media(card)) => (Screen::MediaCard { card, volume: None }, at),
                        None => return,
                    },
                    _ => (Screen::SettingsCard, at),
                }
            }
            Mode::LightList { slot, selected } => match card(*slot) {
                Some(Card::Lights(card)) => (Screen::LightList { card, selected: *selected }, position(slot + 1)),
                _ => return,
            },
            Mode::LightDim { slot, light, on, value } => match card(*slot) {
                Some(Card::Lights(card)) => (Screen::LightDim { light: &card.lights[*light], on: *on, value: *value },
                                             position(slot + 1)),
                _ => return,
            },
            Mode::ClimateSet { slot, target } => match card(*slot) {
                Some(Card::Climate(card)) => (Screen::ClimateSet { card, target: *target }, position(slot + 1)),
                _ => return,
            },
            Mode::Volume { slot, value } => match card(*slot) {
                Some(Card::Media(card)) => (Screen::Volume { card, value: *value }, position(slot + 1)),
                _ => return,
            },
            _ => return,
        };
        let pixmap = screens::draw(&self.fonts, &screen, at);
        self.display.show(&pixmap);
    }
}

/// Every screen, drawn to PNGs in UNFR0ZEN_UI_SHOTS with the fonts in
/// UNFR0ZEN_UI_FONTS (the unit's /nestlabs/share/fonts): how they look,
/// without a unit. `cargo test -p nestlocal ui_shots -- --ignored`
#[cfg(test)]
mod shots {
    use super::*;

    fn save(pixmap: &tiny_skia::Pixmap, path: &str) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), pixmap.width(), pixmap.height());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(pixmap.data()).unwrap();
    }

    #[test]
    #[ignore]
    fn ui_shots() {
        let (Ok(fonts), Ok(out)) = (std::env::var("UNFR0ZEN_UI_FONTS"), std::env::var("UNFR0ZEN_UI_SHOTS")) else {
            panic!("set UNFR0ZEN_UI_FONTS and UNFR0ZEN_UI_SHOTS");
        };
        let fonts = paint::Fonts::load(&fonts).unwrap();
        let cards = Cards::parse(br#"{"cards":[
            {"kind":"lights","name":"Bedroom","icon":"mdi:lightbulb-group","lights":[
                {"id":"light.ben_s_lamp","name":"Ben's Lamp","icon":"m3s:table-lamp","on":true,"dimmable":true,"brightness":60,"rgb":[255,169,87]},
                {"id":"light.alex_s_lamp","name":"Alex's Lamp","icon":"m3o:table-lamp","on":false,"dimmable":true,"brightness":null,"rgb":null},
                {"id":"light.bedroom_lights","name":"Bedroom Lights","icon":"mdi:lightbulb-group","on":true,"dimmable":true,"brightness":40,"rgb":[255,214,170]}]},
            {"kind":"climate","id":"climate.living","name":"Living room","icon":"","mode":"heat","action":"heating","target":21.5,"current":19.5,"min":7,"max":30,"step":0.5},
            {"kind":"media","id":"media_player.kitchen","name":"Kitchen","icon":"","state":"playing","title":"Sunday Morning Mix","artist":"Various artists","volume":35}]}"#).unwrap();
        let [Card::Lights(lights), Card::Climate(climate), Card::Media(media)] = &cards.cards[..] else { panic!() };
        let at = |index| Position { index, count: 5 };
        let shots = [
            ("1-lights", Screen::LightsCard { card: lights }, at(1)),
            ("2-climate", Screen::ClimateCard { card: climate, target: None }, at(2)),
            ("3-media", Screen::MediaCard { card: media, volume: None }, at(3)),
            ("4-settings", Screen::SettingsCard, at(4)),
            ("5-list", Screen::LightList { card: lights, selected: 0 }, at(1)),
            ("6-dim", Screen::LightDim { light: &lights.lights[0], on: true, value: 60 }, at(1)),
            ("7-climate-set", Screen::ClimateSet { card: climate, target: 22.0 }, at(2)),
            ("8-volume", Screen::Volume { card: media, value: 35 }, at(3)),
        ];
        for (name, screen, at) in shots {
            save(&screens::draw(&fonts, &screen, at), &format!("{out}/{name}.png"));
        }
    }
}
