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
mod zoom;

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
        paint::for_panel();
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
    saved: Option<device::Frame>,
    /// How the display moves to what the mode shows next.
    transition: Transition,
    /// Where the cards show, on the way to machine.position(): a place in
    /// the deck, not wrapped, so a turn past the last card runs on.
    view: Option<f32>,
    /// Each card drawn, by its place in the deck (the thermostat is saved),
    /// and zoomed out (zoom::shrink; the thermostat's under 0).
    cards: std::collections::HashMap<usize, device::Frame>,
    shrunk: std::collections::HashMap<usize, device::Frame>,
    /// How big the cards show (1 full size, zoom::ZOOMED while moving).
    zoom: f32,
    /// A frame to compose zoomed cards in.
    scratch: device::Frame,
    /// How far the ring shows on a screen with a value, on its way to it.
    sweep: Option<(SweepKey, f32)>,
    /// A value screen kept between frames (paint::Live): what it was drawn
    /// from, the frame, and what changes it.
    live: Option<(String, paint::Canvas, paint::Live)>,
    last_frame: Instant,
    last_check: Instant,
    nlclient: bool,
    /// When the button went down, and whether its hold was taken as a
    /// long press.
    pressed: Option<(Instant, bool)>,
    /// Frames of the animation under way: how many, how long all told, the
    /// longest.
    frames: (u32, Duration, Duration),
}

/// How long the button is held for a long press.
const LONG_PRESS: Duration = Duration::from_millis(600);

/// How fast what shows catches up with the ring: most of the way in this.
const FOLLOW: f32 = 0.04;
/// How fast the cards zoom out as the ring starts, and in as they settle.
const ZOOM_EASE: f32 = 0.05;

impl Runtime {
    fn new(ui: Arc<Ui>, sender: Arc<dyn Sender>, fonts: paint::Fonts, display: device::Display,
           input: device::Input) -> Runtime {
        let now = Instant::now();
        Runtime {
            ui, sender, fonts, display, input, beeper: device::Beeper::open(), machine: Machine::new(now),
            generation: u64::MAX, saved: None, transition: Transition::Cut, view: None,
            cards: Default::default(), shrunk: Default::default(), zoom: 1.0,
            scratch: vec![0; (paint::SIZE * paint::SIZE * 4) as usize], sweep: None, live: None, last_frame: now,
            last_check: now - Duration::from_secs(10), nlclient: false, frames: Default::default(),
            pressed: None,
        }
    }

    fn run(mut self) {
        logf!("ui: ready for home screen cards");
        hold_state(false);
        loop {
            // A frame each 8 ms while anything moves; otherwise input or time.
            let wait = if self.animating() || self.pressed.is_some() { 8 } else { 50 };
            let events = self.input.wait(Duration::from_millis(wait));
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
                self.cards.clear();
                self.shrunk.clear();
            }
            self.machine.set_active(self.nlclient, now, &mut effects);
            let lit = device::lit();
            let before = self.machine.mode.clone();
            let turned = !events.is_empty();
            for event in events {
                let input = match (event.kind, event.code, event.value) {
                    // The ADC counts anticlockwise up: clockwise is positive here.
                    (device::EV_REL, device::REL_DIAL, value) => Input::Turn(-value),
                    // A press is taken when the button comes up, unless it was
                    // held long enough to be a long press.
                    (device::EV_KEY, device::KEY_ENTER, 1) => {
                        self.pressed = Some((now, false));
                        continue;
                    }
                    (device::EV_KEY, device::KEY_ENTER, 0) => match self.pressed.take() {
                        Some((_, false)) => Input::Press,
                        _ => continue,
                    },
                    _ => continue,
                };
                self.machine.input(input, now, lit, &mut effects);
            }
            if let Some((down, false)) = self.pressed {
                if now.duration_since(down) >= LONG_PRESS {
                    self.pressed = Some((down, true));
                    self.machine.input(Input::LongPress, now, lit, &mut effects);
                }
            }
            let display = &mut self.display;
            self.machine.tick(now, lit, &mut || home_dial(display), &mut effects);
            self.transition = transition(&before, &self.machine.mode, turned && lit);
            for effect in effects {
                self.apply(effect);
            }
            let elapsed = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
            self.last_frame = now;
            let swept = self.sweep(elapsed);
            let dirty = self.machine.take_dirty();
            if dirty && self.machine.position().is_none() {
                // A light switched in a list shows on its card.
                self.cards.clear();
                self.shrunk.clear();
            }
            if self.machine.owns() {
                let started = Instant::now();
                let drew = match self.machine.position() {
                    Some(target) => self.follow(target, elapsed, dirty),
                    None => {
                        self.view = None;
                        (dirty || swept) && {
                            self.draw();
                            true
                        }
                    }
                };
                if drew {
                    let took = started.elapsed();
                    self.frames = (self.frames.0 + 1, self.frames.1 + took, self.frames.2.max(took));
                }
            } else {
                self.view = None;
                self.zoom = 1.0;
            }
            if !self.animating() && self.machine.mode != Mode::Idle {
                // One card at a time while nothing moves, so the first turn
                // finds them all drawn.
                let deck = self.machine.deck().len();
                if let Some(index) = (1..deck).find(|i| !self.cards.contains_key(i)) {
                    self.prepare(index as i64);
                } else if let Some(index) = (1..deck).find(|i| !self.shrunk.contains_key(i)) {
                    let shrunk = zoom::shrink(&self.cards[&index]);
                    self.shrunk.insert(index, shrunk);
                }
            }
            if !self.animating() {
                // Only a movement with a frame that missed the display's
                // 60 Hz: anything else is as it should be.
                let (count, total, longest) = std::mem::take(&mut self.frames);
                if longest > Duration::from_millis(17) {
                    logf!("ui: {count} frames, {:.1} ms each, the longest {} ms",
                          total.as_secs_f32() * 1000.0 / count as f32, longest.as_millis());
                }
            }
        }
    }

    /// Whether something on the display is on its way somewhere.
    fn animating(&self) -> bool {
        let following = match (self.view, self.machine.position()) {
            (Some(view), Some(target)) => (view - self.nearest(view, target)).abs() > 0.001,
            _ => false,
        };
        let zooming = self.view.is_some() && (self.zoom - self.zoom_target()).abs() > 0.002;
        following || zooming || self.sweeping()
    }

    /// Zoomed out while the cards move, full size once they settle.
    fn zoom_target(&self) -> f32 {
        let moving = match (self.view, self.machine.position()) {
            (Some(view), Some(target)) => {
                (view - self.nearest(view, target)).abs() > 0.001 || target.fract().abs() > 0.001
            }
            _ => false,
        };
        if moving { zoom::ZOOMED } else { 1.0 }
    }

    /// `target`'s place in the deck nearest `view`: the deck goes round.
    fn nearest(&self, view: f32, target: f32) -> f32 {
        let count = self.machine.deck().len() as f32;
        target + count * ((view - target) / count).round()
    }

    /// Moves the cards towards `target`, as the hand turns. Returns whether
    /// it drew.
    fn follow(&mut self, target: f32, elapsed: f32, dirty: bool) -> bool {
        let Some(view) = self.view else {
            // Onto the cards: from a turn on the thermostat, where it is;
            // back from a card's own screens, faded in.
            let start = if self.transition == Transition::Fade { target } else { target.round() };
            self.view = Some(start);
            if self.transition == Transition::Fade {
                let from = self.display.save();
                let to = self.card_frame(start.round() as i64);
                self.animate(&from, &to, Transition::Fade);
            }
            self.show_cards(start);
            return true;
        };
        let target = self.nearest(view, target);
        let mut next = view + (target - view) * (1.0 - (-elapsed / FOLLOW).exp());
        if (next - target).abs() < 0.001 {
            next = target;
        }
        self.view = Some(next);
        let zoomed = self.step_zoom(elapsed);
        if next != view || zoomed || dirty {
            self.show_cards(next);
            return true;
        }
        false
    }

    /// Eases the zoom towards zoom_target(); returns whether it changed.
    fn step_zoom(&mut self, elapsed: f32) -> bool {
        let target = self.zoom_target();
        let mut next = self.zoom + (target - self.zoom) * (1.0 - (-elapsed / ZOOM_EASE).exp());
        if (next - target).abs() < 0.002 {
            next = target;
        }
        let changed = next != self.zoom;
        self.zoom = next;
        changed
    }

    /// Shows the cards at place `at` in the deck: one, or two side by side;
    /// zoomed out, as many as show.
    fn show_cards(&mut self, at: f32) {
        if self.zoom < 0.999 {
            return self.show_zoomed(at);
        }
        let whole = at.floor();
        let part = at - whole;
        let (left, right) = (whole as i64, whole as i64 + 1);
        self.prepare(left);
        if part < 0.001 {
            let frame = self.card_frame(left);
            self.display.show(&frame);
            return;
        }
        self.prepare(right);
        let count = self.machine.deck().len() as i64;
        let (cards, saved) = (&self.cards, &self.saved);
        let get = |at: i64| match at.rem_euclid(count) as usize {
            0 => saved.as_deref(),
            index => cards.get(&index).map(Vec::as_slice),
        };
        if let (Some(from), Some(to)) = (get(left), get(right)) {
            self.display.slide(from, to, (part * paint::SIZE as f32) as usize, true);
        }
    }

    fn show_zoomed(&mut self, at: f32) {
        // From the shrunk cards until nearly full size; see zoom::compose.
        let shrunk = self.zoom < 0.95;
        let z = if (self.zoom - zoom::ZOOMED).abs() < 0.003 { zoom::ZOOMED } else { self.zoom };
        let count = self.machine.deck().len() as i64;
        for k in at.floor() as i64 - 2..=at.ceil() as i64 + 2 {
            self.prepare(k);
            let index = k.rem_euclid(count) as usize;
            if shrunk && !self.shrunk.contains_key(&index) {
                let full = if index == 0 { self.saved.clone() } else { self.cards.get(&index).cloned() };
                if let Some(full) = full {
                    self.shrunk.insert(index, zoom::shrink(&full));
                }
            }
        }
        let (cards, shrunk_cards, saved) = (&self.cards, &self.shrunk, &self.saved);
        let get = |k: i64| -> Option<&[u8]> {
            let index = k.rem_euclid(count) as usize;
            if shrunk {
                shrunk_cards.get(&index).map(Vec::as_slice)
            } else if index == 0 {
                saved.as_deref()
            } else {
                cards.get(&index).map(Vec::as_slice)
            }
        };
        zoom::compose(&mut self.scratch, at, z, if shrunk { zoom::ZOOMED } else { 1.0 }, get);
        self.display.show(&self.scratch);
    }

    /// Draws card `at` (wrapping round the deck) if it isn't yet.
    fn prepare(&mut self, at: i64) {
        let deck = self.machine.deck();
        let index = at.rem_euclid(deck.len() as i64) as usize;
        if index == 0 || self.cards.contains_key(&index) {
            return;
        }
        let position = Position { index, count: deck.len() };
        let card = |slot: usize| self.machine.cards.cards.get(slot);
        let screen = match deck[index] {
            Slot::Card(slot) => match card(slot) {
                Some(Card::Lights(card)) => Screen::LightsCard { card },
                Some(Card::Climate(card)) => Screen::ClimateCard { card, target: None },
                Some(Card::Media(card)) => Screen::MediaCard { card, volume: None },
                None => return,
            },
            _ => Screen::SettingsCard,
        };
        let frame = device::frame(screens::draw(&self.fonts, &screen, position, None));
        self.cards.insert(index, frame);
    }

    /// Card `at` as a frame (nlclient's for the thermostat).
    fn card_frame(&mut self, at: i64) -> device::Frame {
        self.prepare(at);
        let index = at.rem_euclid(self.machine.deck().len() as i64) as usize;
        let frame = if index == 0 { self.saved.clone() } else { self.cards.get(&index).cloned() };
        frame.unwrap_or_else(|| vec![0; (paint::SIZE * paint::SIZE * 4) as usize])
    }

    /// How far the ring of the mode's value should show (0..1), if it has one.
    fn value(&self) -> Option<f32> {
        let card = |slot: usize| self.machine.cards.cards.get(slot);
        match &self.machine.mode {
            Mode::LightDim { on: true, value, .. } | Mode::Volume { value, .. } => Some(*value as f32 / 100.0),
            Mode::ClimateSet { slot, target } => match card(*slot) {
                Some(Card::Climate(c)) => Some(((target - c.min) / (c.max - c.min).max(0.5)) as f32),
                _ => None,
            },
            _ => None,
        }
    }

    fn sweeping(&self) -> bool {
        matches!((&self.sweep, self.value()), (Some((_, shown)), Some(value)) if *shown != value)
    }

    /// Moves the ring towards its value, as the Nest's dial follows the
    /// ring. A new screen starts at its value. Returns whether it moved.
    fn sweep(&mut self, elapsed: f32) -> bool {
        let Some(value) = self.value() else {
            self.sweep = None;
            return false;
        };
        let key = sweep_key(&self.machine.mode);
        match self.sweep {
            Some((was, shown)) if Some(was) == key => {
                let mut next = shown + (value - shown) * (1.0 - (-elapsed / FOLLOW).exp());
                if (next - value).abs() < 0.002 {
                    next = value;
                }
                self.sweep = key.map(|key| (key, next));
                next != shown
            }
            _ => {
                self.sweep = key.map(|key| (key, value));
                false
            }
        }
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::Own(true) => {
                self.saved = Some(self.display.save());
                device::own(true);
                self.display.begin();
                // The thermostat zoomed out too, before the first frame needs it.
                if let Some(saved) = &self.saved {
                    self.shrunk.insert(0, zoom::shrink(saved));
                }
            }
            Effect::Own(false) => {
                self.shrunk.remove(&0);
                // Back to the thermostat: the cards run on to it as the ring
                // turned, or fade to it when left alone. Then nlclient's frame
                // back before nlclient may present again.
                if let Some(saved) = self.saved.clone() {
                    match (self.transition, self.view) {
                        (Transition::Fade, _) => {
                            let from = self.display.save();
                            self.animate(&from, &saved, Transition::Fade);
                        }
                        (_, Some(_)) => self.settle(),
                        _ => {}
                    }
                    self.display.end(&saved);
                }
                self.saved = None;
                self.view = None;
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

    /// Runs the cards on to the thermostat before handing it back.
    fn settle(&mut self) {
        let mut last = Instant::now();
        while let Some(view) = self.view {
            let target = self.nearest(view, 0.0);
            if (view - target).abs() < 0.001 && self.zoom >= 1.0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(8));
            let elapsed = last.elapsed().as_secs_f32();
            last = Instant::now();
            let mut next = view + (target - view) * (1.0 - (-elapsed / FOLLOW).exp());
            if (next - target).abs() < 0.001 {
                next = target;
            }
            self.view = Some(next);
            self.step_zoom(elapsed);
            self.show_cards(next);
        }
    }

    fn draw(&mut self) {
        let machine = &self.machine;
        let deck = machine.deck();
        let position = |index: usize| Position { index, count: deck.len() };
        let card = |slot: usize| machine.cards.cards.get(slot);
        let (screen, at) = match &machine.mode {
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
        let sweep = self.sweep.as_ref().map(|(_, shown)| *shown);
        let fade = self.transition == Transition::Fade;
        self.transition = Transition::Cut;
        // A value screen draws its still part once, then only what changes.
        if let Some(key) = screens::still_key(&screen) {
            let fresh = !matches!(&self.live, Some((kept, ..)) if *kept == key);
            if fresh {
                let still = screens::still(&self.fonts, &screen, at);
                let live = paint::Live::new(&still);
                self.live = Some((key, still, live));
            }
            let (_, canvas, live) = self.live.as_mut().expect("just made");
            live.columns = None;
            screens::moving(canvas, &self.fonts, &screen, sweep, Some(live));
            if fresh || fade {
                let frame = canvas.pixmap.data().to_vec();
                if fade {
                    let from = self.display.save();
                    self.animate(&from, &frame, Transition::Fade);
                }
                self.display.show(&frame);
            } else if let Some((first, last)) = live.columns {
                // An upright column is a framebuffer row.
                self.display.show_rows(canvas.pixmap.data(), first as usize, last as usize);
            }
            return;
        }
        self.live = None;
        let frame = device::frame(screens::draw(&self.fonts, &screen, at, sweep));
        if fade {
            let from = self.display.save();
            self.animate(&from, &frame, Transition::Fade);
        }
        self.display.show(&frame);
    }

    /// Fades the display from `from` to `to`, in about a sixth of a second.
    fn animate(&mut self, from: &[u8], to: &[u8], transition: Transition) {
        if transition == Transition::Cut {
            return;
        }
        let started = Instant::now();
        let length = Duration::from_millis(160);
        loop {
            let t = started.elapsed().as_secs_f32() / length.as_secs_f32();
            if t >= 1.0 {
                return;
            }
            self.display.fade(from, to, (ease(t) * 256.0) as u16);
        }
    }
}

/// Which screen a sweep belongs to: the kind of value, and whose.
type SweepKey = (u8, usize, usize);

fn sweep_key(mode: &Mode) -> Option<SweepKey> {
    match *mode {
        Mode::LightDim { slot, light, .. } => Some((0, slot, light)),
        Mode::Volume { slot, .. } => Some((1, slot, 0)),
        Mode::ClimateSet { slot, .. } => Some((2, slot, 0)),
        _ => None,
    }
}

/// How the display moves from one screen to the next. Turning between the
/// cards isn't one: the cards follow the ring (Runtime::follow).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Transition {
    Cut,
    Fade,
}

/// Going into a card or back out of one fades, as does going back when left
/// alone; handing nlclient its menus, and anything within one view, cuts.
fn transition(before: &Mode, after: &Mode, turned: bool) -> Transition {
    let cards = |mode: &Mode| matches!(mode, Mode::Home | Mode::Card(_));
    let same_view = std::mem::discriminant(before) == std::mem::discriminant(after) && !cards(before);
    if before == after || same_view || !(before.owning() || after.owning()) || *after == Mode::Native {
        return Transition::Cut;
    }
    if cards(before) && cards(after) && turned {
        return Transition::Cut;
    }
    Transition::Fade
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn going_in_and_out_fades_and_the_rest_cuts() {
        assert_eq!(transition(&Mode::Home, &Mode::Card(1), true), Transition::Cut, "the cards follow the ring");
        assert_eq!(transition(&Mode::Card(4), &Mode::Home, true), Transition::Cut);
        assert_eq!(transition(&Mode::Card(2), &Mode::Home, false), Transition::Fade, "left alone");
        let list = Mode::LightList { slot: 0, selected: 0 };
        assert_eq!(transition(&Mode::Card(1), &list, true), Transition::Fade);
        assert_eq!(transition(&list, &Mode::Card(1), false), Transition::Fade);
        assert_eq!(transition(&Mode::Card(4), &Mode::Native, true), Transition::Cut, "nlclient's menu");
        assert_eq!(transition(&list, &Mode::LightList { slot: 0, selected: 1 }, true), Transition::Cut);
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

    fn save(canvas: &paint::Canvas, path: &str) {
        let pixmap = &canvas.pixmap;
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
                {"id":"light.reading_lamp","name":"Reading Lamp","icon":"m3s:table-lamp","on":true,"dimmable":true,"brightness":60,"rgb":[255,169,87]},
                {"id":"light.desk_lamp","name":"Desk Lamp","icon":"m3o:table-lamp","on":false,"dimmable":true,"brightness":null,"rgb":null},
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
            save(&screens::draw(&fonts, &screen, at, None), &format!("{out}/{name}.png"));
        }
    }
}
