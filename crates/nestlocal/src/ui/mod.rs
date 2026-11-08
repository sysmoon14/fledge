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
mod machine;
mod model;
mod paint;
mod screens;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use model::Command;
use machine::{Effect, Input, Machine, Mode, Slot};
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

/// What the UI sends Home Assistant, and the Nest's own settings it follows.
pub trait Sender: Send + Sync + 'static {
    fn send(&self, command: &Command);
    /// Whether the Nest clicks as the ring turns (its click sound setting).
    fn clicks(&self) -> bool;
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

struct Runtime {
    ui: Arc<Ui>,
    sender: Arc<dyn Sender>,
    fonts: paint::Fonts,
    display: device::Display,
    input: device::Input,
    beeper: device::Beeper,
    machine: Machine,
    generation: u64,
    /// nlclient's frame from before the UI owned the display.
    saved: Option<Vec<u8>>,
    /// What the display shows, while the UI owns it.
    shown: Option<device::Frame>,
    /// How the display moves to what the mode shows next.
    transition: Transition,
    last_check: Instant,
    nlclient: bool,
}

impl Runtime {
    fn new(ui: Arc<Ui>, sender: Arc<dyn Sender>, fonts: paint::Fonts, display: device::Display,
           input: device::Input) -> Runtime {
        let now = Instant::now();
        Runtime {
            ui, sender, fonts, display, input, beeper: device::Beeper::open(), machine: Machine::new(now), generation: u64::MAX, saved: None,
            shown: None, transition: Transition::Cut,
            last_check: now - Duration::from_secs(10), nlclient: false,
        }
    }

    fn run(mut self) {
        logf!("ui: ready for home screen cards");
        hold_state(false);
        loop {
            let events = self.input.wait(Duration::from_millis(50));
            let now = Instant::now();
            let mut effects = Vec::new();
            if now.duration_since(self.last_check) >= Duration::from_secs(2) {
                self.last_check = now;
                self.nlclient = !crate::pids("nlclient").is_empty();
            }
            if self.ui.generation() != self.generation {
                let (generation, cards) = self.ui.snapshot();
                self.generation = generation;
                self.machine.set_cards(cards, &mut effects);
            }
            self.machine.set_active(self.nlclient, now, &mut effects);
            let lit = device::lit();
            let before = self.machine.mode.clone();
            let turned = !events.is_empty();
            for event in events {
                let input = match (event.kind, event.code, event.value) {
                    // The ADC counts anticlockwise up: clockwise is positive here.
                    (device::EV_REL, device::REL_DIAL, value) => Input::Turn(-value),
                    (device::EV_KEY, device::KEY_ENTER, 1) => Input::Press,
                    _ => continue,
                };
                self.machine.input(input, now, lit, &mut effects);
            }
            let display = &mut self.display;
            self.machine.tick(now, lit, &mut || home_dial(display), &mut effects);
            self.transition = transition(&before, &self.machine.mode, self.machine.deck().len(), turned && lit);
            for effect in effects {
                self.apply(effect);
            }
            if self.machine.take_dirty() && self.machine.mode.owning() {
                self.draw();
            }
        }
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::Own(true) => {
                self.saved = Some(self.display.save());
                device::own(true);
            }
            Effect::Own(false) => {
                // nlclient's frame back before nlclient may present again.
                if let Some(saved) = self.saved.take() {
                    if let Some(from) = self.shown.take() {
                        self.animate(&from, &saved, self.transition);
                    }
                    self.display.show(&saved);
                }
                self.shown = None;
                device::own(false);
            }
            Effect::Hold(hold) => {
                self.input.grab(hold);
                hold_state(hold);
            }
            Effect::WakeDisplay => device::wake_display(),
            Effect::Click => {
                if self.sender.clicks() {
                    self.beeper.click();
                }
            }
            Effect::KeepAwake => crate::wake::keep_awake_for(30),
            Effect::PressNlclient => {
                // Once nlclient has its frame back on the display.
                std::thread::sleep(Duration::from_millis(300));
                self.input.press();
            }
            Effect::Send(command) => self.sender.send(&command),
        }
    }

    fn draw(&mut self) {
        let machine = &self.machine;
        let deck = machine.deck();
        let position = |index: usize| Position { index, count: deck.len() };
        let card = |slot: usize| machine.cards.cards.get(slot);
        let (screen, at) = match &machine.mode {
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
        let started = Instant::now();
        let pixmap = screens::draw(&self.fonts, &screen, at);
        let drawn = started.elapsed();
        let frame = device::frame(&pixmap);
        let turned = started.elapsed() - drawn;
        // From the card before, or from nlclient's frame on the way in.
        if let Some(from) = self.shown.take().or_else(|| self.saved.clone()) {
            self.animate(&from, &frame, self.transition);
        }
        let shown = Instant::now();
        self.display.show(&frame);
        if started.elapsed() > Duration::from_millis(30) {
            logf!("ui: drew in {} ms, turned in {} ms, showed in {} ms", drawn.as_millis(), turned.as_millis(),
                  shown.elapsed().as_millis());
        }
        self.shown = Some(frame);
        self.transition = Transition::Cut;
    }

    /// Moves the display from `from` to `to`: about a fifth of a second.
    fn animate(&mut self, from: &[u8], to: &[u8], transition: Transition) {
        if transition == Transition::Cut {
            return;
        }
        let started = Instant::now();
        let length = Duration::from_millis(match transition { Transition::Fade => 160, _ => 220 });
        for frames in 0.. {
            let t = started.elapsed().as_secs_f32() / length.as_secs_f32();
            if t >= 1.0 {
                if frames < 6 {
                    logf!("ui: {transition:?} in {frames} frames");
                }
                return;
            }
            match transition {
                Transition::Slide { clockwise } => {
                    self.display.slide(from, to, (ease(t) * paint::SIZE as f32) as usize, clockwise)
                }
                _ => self.display.fade(from, to, (ease(t) * 256.0) as u16),
            }
        }
    }
}

/// How the display moves from one screen to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Transition {
    Cut,
    /// The next screen slides in from the right (clockwise) or left.
    Slide { clockwise: bool },
    Fade,
}

/// Turning between cards slides, the way the ring turned; going into a card
/// or back out of one fades; handing nlclient its menus cuts.
fn transition(before: &Mode, after: &Mode, deck: usize, turned: bool) -> Transition {
    let index = |mode: &Mode| match mode {
        Mode::Home => Some(0),
        Mode::Card(index) => Some(*index),
        _ => None,
    };
    let same_view = std::mem::discriminant(before) == std::mem::discriminant(after) && index(before).is_none();
    if before == after || same_view || !(before.owning() || after.owning()) {
        // A row or a value moved, or nothing of the UI's shows.
        return Transition::Cut;
    }
    match (index(before), index(after)) {
        (Some(from), Some(to)) if turned => Transition::Slide { clockwise: to == (from + 1) % deck },
        _ if *after == Mode::Native => Transition::Cut,
        _ => Transition::Fade,
    }
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_slides_the_way_the_ring_went_and_the_rest_fades() {
        let slide = |clockwise| Transition::Slide { clockwise };
        assert_eq!(transition(&Mode::Home, &Mode::Card(1), 5, true), slide(true));
        assert_eq!(transition(&Mode::Home, &Mode::Card(4), 5, true), slide(false));
        assert_eq!(transition(&Mode::Card(4), &Mode::Home, 5, true), slide(true), "past the last card");
        assert_eq!(transition(&Mode::Card(2), &Mode::Card(1), 5, true), slide(false));
        assert_eq!(transition(&Mode::Card(2), &Mode::Home, 5, false), Transition::Fade, "left alone");
        let list = Mode::LightList { slot: 0, selected: 0 };
        assert_eq!(transition(&Mode::Card(1), &list, 5, true), Transition::Fade);
        assert_eq!(transition(&Mode::Card(4), &Mode::Native, 5, true), Transition::Cut, "nlclient's menu");
        assert_eq!(transition(&Mode::Home, &Mode::Native, 5, true), Transition::Cut);
        assert_eq!(transition(&list, &Mode::LightList { slot: 0, selected: 1 }, 5, true), Transition::Cut);
    }
}

/// Whether the ring is held, for tools that drive the unit's screen
/// (utils/nest-screen.py): a turn while it isn't reaches the temperature.
fn hold_state(hold: bool) {
    let _ = crate::settings_ui::publish("ui.state", if hold { b"holds" } else { b"leaves" });
}

/// Whether nlclient shows its home dial: its ring lit around the edge.
fn home_dial(display: &mut device::Display) -> bool {
    let page = display.save();
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
            {"kind":"media","id":"media_player.kitchen","name":"Bedroom speaker","icon":"","state":"playing","title":"Sunday Morning Mix","artist":"Various artists","volume":35}]}"#).unwrap();
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
