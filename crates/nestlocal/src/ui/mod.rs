//! The Nest's home screen cards: Home Assistant on the thermostat.
//!
//! Waking shows the thermostat among a card for each device the Fledge
//! integration chose (lights, other thermostats, media players) and
//! Settings, zoomed out: turning browses, pressing opens. The thermostat is
//! a card of our own, drawn from nestlocal's buckets: its dial sets the
//! target, its menu Eco, heating on or off and a hot water boost, as Home
//! Assistant would. Settings are ours too (Eco temperatures, Home/Away
//! Assist, Auto-Schedule, ...), with More Nest settings for nlclient's own
//! menus. Nest's own dial stays a choice in Settings, and stands in until
//! nlclient has reported. nlclient keeps the heating, learning and
//! everything else throughout.
//!
//! nestlocal holds the ring and button (grabbed) while the thermostat's own
//! screen shows, so a turn moves to the cards rather than the temperature.
//! On a card it owns the display (device.rs), which nlclient keeps awake.
//! Left alone, the cards go back to the thermostat. With no cards from Home
//! Assistant, or no nlclient (Doom), it leaves everything alone.

mod device;
mod icons;
mod machine;
mod model;
mod paint;
mod screens;
mod zoom;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use model::Command;
use machine::{Action, Effect, Input, Machine, Mode, Slot};
use model::{Alert, Art, Card, Cards, Nest};
use screens::Screen;

use crate::logf;

/// What of the cards this build understands, for the Fledge integration to
/// check (<base>/ui/protocol). Raise it with each addition Fledge relies on:
///   1  device cards (one light each), wake, album art (<base>/set/art/...)
///   2  a light's warmth and colour (kelvin, kelvins, hues, hue; the
///      color_temp and hue commands), a track's progress (position,
///      position_at, duration), screen_on and screen_wake
///   3  action (scene, script, button) and camera cards (run, snapshot),
///      notifications (<base>/set/alert; alert_pressed, alert_dismissed),
///      pictures up to 480 wide
///   4  option (select) and cover cards (select, position, open, close),
///      presets (preset), previous and next, a light group's members, and
///      the glance
pub const PROTOCOL: u32 = 4;

#[derive(Default)]
pub struct Ui {
    /// Home Assistant's cards and a count of their changes.
    cards: Mutex<(u64, Cards)>,
    /// Media players' album art, by entity id.
    art: Mutex<HashMap<String, Arc<Art>>>,
    /// Home Assistant's: keep the display lit (<base>/set/screen_on), and
    /// how many times it asked to wake it (<base>/set/screen_wake).
    keep_on: std::sync::atomic::AtomicBool,
    wakes: std::sync::atomic::AtomicU64,
    /// Home Assistant's latest notification (<base>/set/alert), counted.
    alert: Mutex<(u64, Option<Alert>)>,
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

    /// Takes a media player's album art (<base>/set/art/<entity id>): a
    /// PNG, or nothing for none.
    pub fn set_art(&self, entity: &str, payload: &[u8]) -> Result<(), String> {
        let art = if payload.is_empty() { None } else { Some(Arc::new(Art::decode(payload)?)) };
        let mut held = self.art.lock().unwrap();
        if entity.starts_with("alert:") {
            // A notification's picture is only wanted while it shows: one
            // at a time, not one for every notification since it started.
            held.retain(|key, _| !key.starts_with("alert:") || key == entity);
        }
        let changed = match art {
            Some(art) => held.insert(entity.to_owned(), art.clone()).is_none_or(|old| old != art),
            None => held.remove(entity).is_some(),
        };
        drop(held);
        if changed {
            self.cards.lock().unwrap().0 += 1;
        }
        Ok(())
    }

    /// Keeps the display lit (true) or leaves it to go out as it does.
    pub fn keep_on(&self, on: bool) {
        self.keep_on.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// Wakes the display onto what it wakes to, as a turn would.
    pub fn wake(&self) {
        self.wakes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn generation(&self) -> u64 {
        self.cards.lock().unwrap().0
    }

    /// The cards, each media player's with its art.
    fn snapshot(&self) -> (u64, Cards) {
        let (generation, mut cards) = self.cards.lock().unwrap().clone();
        let art = self.art.lock().unwrap();
        for card in &mut cards.cards {
            match card {
                Card::Media(media) => media.art = art.get(&media.id).cloned(),
                Card::Camera(camera) => camera.art = art.get(&camera.id).cloned(),
                _ => {}
            }
        }
        (generation, cards)
    }

    /// Takes a notification (<base>/set/alert), or Home Assistant taking
    /// one back.
    pub fn set_alert(&self, payload: &[u8]) -> Result<(), String> {
        let alert = Alert::parse(payload)?;
        let mut held = self.alert.lock().unwrap();
        *held = (held.0 + 1, Some(alert));
        Ok(())
    }

    /// A picture by its key among the art (a notification's).
    fn art(&self, key: &str) -> Option<Arc<Art>> {
        self.art.lock().unwrap().get(key).cloned()
    }
}

/// What the UI sends Home Assistant, and the Nest's own settings it follows.
pub trait Sender: Send + Sync + 'static {
    fn send(&self, command: &Command);
    /// Whether the Nest clicks as the ring turns (its click sound setting).
    fn clicks(&self) -> bool;
    /// This thermostat's state, as Home Assistant's entities read it, with
    /// can_heat, can_cool, has_hot_water_control, version and update (a
    /// newer release): its own card and Settings. None until it reports.
    fn nest(&self) -> Option<serde_json::Map<String, serde_json::Value>> {
        None
    }
    /// One of this thermostat's own commands, as <base>/set/<name> takes it.
    fn nest_command(&self, _name: &str, _arg: &str) {}
}

/// Kept while the thermostat shows Nest's own dial rather than its card.
const NEST_DIAL: &str = "/media/data/fledge-nest-dial";

/// Starts the UI on a gen 3 unit: it waits for cards from Home Assistant.
pub fn start(ui: Arc<Ui>, sender: Arc<dyn Sender>) {
    let handle = tokio::runtime::Handle::try_current().ok();
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
        // The thermostat's own commands (an update's install) may start work
        // on nestlocal's runtime.
        let _runtime = handle.as_ref().map(|h| h.enter());
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
    /// nlclient's frame from before the UI owned the display, and it round
    /// (zoom::round) as a card among the others.
    saved: Option<device::Frame>,
    home: Option<device::Frame>,
    /// How the display moves to what the mode shows next.
    transition: Transition,
    /// Where the cards show, on the way to machine.position(): a place in
    /// the deck, not wrapped, so a turn past the last card runs on.
    view: Option<f32>,
    /// Each card drawn, by its place in the deck (the thermostat is saved),
    /// and zoomed out (zoom::shrink; the thermostat's under 0).
    cards: std::collections::HashMap<usize, device::Frame>,
    shrunk: std::collections::HashMap<usize, device::Frame>,
    /// Each media card's progress bar as drawn, in pixels, and when they
    /// were last looked at: a card is drawn again as its bar moves.
    progress: std::collections::HashMap<usize, u32>,
    progress_checked: Instant,
    /// The wakes Home Assistant asked for that were done, and when the
    /// display was last kept lit for it.
    wakes: u64,
    kept_on: Instant,
    /// The notifications taken, and the scene last shown as run.
    alerts: u64,
    ran: Option<(usize, Instant)>,
    /// The minute the glance's clock last showed.
    glance_minute: i64,
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
    /// When the button went down, whether its hold was taken as a long
    /// press, and whether the display was lit then.
    pressed: Option<(Instant, bool, bool)>,
    /// On nlclient's screens: whether its menu's Back was chosen as the
    /// button went down, and since when the press has waited for its dial.
    back_chosen: bool,
    back_since: Option<Instant>,
    /// In nlclient's Settings, opened from ours: the last input there, when
    /// its dial was last looked for, and how many looks in a row found it.
    native_input: Instant,
    settings_checked: Instant,
    settings_seen: u32,
    /// When the thermostat's own state was last asked for.
    nest_checked: Instant,
    /// The thermostat's dial: when its pulse (from now to the target) began,
    /// and when the next is due.
    pulse: Option<Instant>,
    next_pulse: Instant,
    /// A list's highlight as it glides between rows: the list's layers
    /// (Layer::Plain, Layer::Chosen), what they show, and where it is.
    glide: Option<Glide>,
    /// Something changed that the machine doesn't know of: draw.
    force_draw: bool,
    /// nlclient's dial as it last showed lit, and when: the thermostat's
    /// card when the display wakes, its own page dark until nlclient draws.
    dial: Option<device::Frame>,
    last_dial: Instant,
    /// Frames of the animation under way: how many, how long all told, the
    /// longest.
    frames: (u32, Duration, Duration),
}

struct Glide {
    key: String,
    plain: paint::Canvas,
    chosen: paint::Canvas,
    /// The highlight's top, and where it's going.
    y: f32,
    target: f32,
    drawn: Instant,
}

/// How long the thermostat's pulse runs, and how often.
const PULSE: Duration = Duration::from_millis(1800);
const PULSE_EVERY: Duration = Duration::from_secs(30);
/// How fast a list's highlight glides to its row: most of the way in this.
const GLIDE: f32 = 0.05;
/// How long fading from one screen to the next takes.
const FADE: Duration = Duration::from_millis(280);

/// How long the button is held for a long press.
const LONG_PRESS: Duration = Duration::from_millis(600);
/// Looks in a row, 200 ms apart, that find nlclient's home dial after its
/// Settings: it was left.
const SETTINGS_LEFT: u32 = 4;

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
            generation: u64::MAX, saved: None, home: None, transition: Transition::Cut, view: None,
            cards: Default::default(), shrunk: Default::default(), zoom: 1.0,
            progress: Default::default(), progress_checked: Instant::now(), wakes: 0, kept_on: Instant::now(),
            alerts: 0, ran: None, glance_minute: 0,
            scratch: vec![0; (paint::SIZE * paint::SIZE * 4) as usize], sweep: None, live: None, last_frame: now,
            last_check: now - Duration::from_secs(10), nlclient: false, frames: Default::default(),
            pressed: None, back_chosen: false, back_since: None, native_input: now, settings_checked: now, settings_seen: 0,
            nest_checked: now - Duration::from_secs(10), pulse: None, next_pulse: now, glide: None, force_draw: false,
            dial: None, last_dial: now - Duration::from_secs(60),
        }
    }

    fn run(mut self) {
        logf!("ui: ready for home screen cards");
        hold_state(false);
        self.machine.nest_dial = std::path::Path::new(NEST_DIAL).exists();
        loop {
            // A frame each 8 ms while anything moves; otherwise input or time.
            // Quick while anything moves; a slower beat when there's nothing
            // to show (no cards, or asleep on nlclient's dial), to wake the
            // unit less, as input still ends the wait at once.
            let idle = matches!(self.machine.mode, Mode::Idle) || (self.machine.mode == Mode::Home && !device::lit());
            let wait = if self.animating() || self.pressed.is_some() || self.back_since.is_some() {
                8
            } else if idle {
                250
            } else {
                50
            };
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
                // A notification's picture may come after it.
                let key = self.machine.alert.as_ref().and_then(|a| a.picture.clone());
                if let (Some(alert), Some(key)) = (&mut self.machine.alert, key) {
                    alert.art = self.ui.art(&key);
                }
            }
            self.machine.set_active(self.nlclient, now, &mut effects);
            if now.duration_since(self.nest_checked) >= Duration::from_secs(1) {
                self.nest_checked = now;
                self.refresh_nest();
            }
            let lit = device::lit();
            let before = self.machine.mode.clone();
            let turned = !events.is_empty();
            if turned && self.machine.mode == Mode::Native {
                self.native_input = now;
                self.settings_seen = 0;
            }
            for event in events {
                let input = match (event.kind, event.code, event.value) {
                    // The ADC counts anticlockwise up: clockwise is positive here.
                    (device::EV_REL, device::REL_DIAL, value) => Input::Turn(-value),
                    // A press is taken when the button comes up, unless it was
                    // held long enough to be a long press.
                    // Whether the display was lit is taken as the button goes
                    // down: pressing it lights the display before it comes up.
                    (device::EV_KEY, device::KEY_ENTER, 1) => {
                        self.pressed = Some((now, false, lit));
                        if self.machine.mode == Mode::Native {
                            self.back_chosen = back_chosen(&self.display.save());
                        }
                        continue;
                    }
                    (device::EV_KEY, device::KEY_ENTER, 0) => match self.pressed.take() {
                        Some((_, false, was_lit)) => {
                            if self.machine.mode == Mode::Native && std::mem::take(&mut self.back_chosen) {
                                self.back_since = Some(now);
                            }
                            self.machine.input(Input::Press, now, was_lit, &mut effects);
                            continue;
                        }
                        _ => continue,
                    },
                    _ => continue,
                };
                self.machine.input(input, now, lit, &mut effects);
            }
            let (alerts, alert) = self.ui.alert.lock().unwrap().clone();
            if alerts != self.alerts {
                self.alerts = alerts;
                if let Some(mut alert) = alert {
                    alert.art = alert.picture.as_deref().and_then(|key| self.ui.art(key));
                    logf!("ui: notification {}", alert.id);
                    self.machine.show_alert(alert, now, lit, &mut effects);
                }
            }
            if self.machine.ran != self.ran {
                // A scene's card says it ran, then goes back.
                for (slot, _) in [self.ran, self.machine.ran].into_iter().flatten() {
                    self.cards.remove(&(slot + 1));
                    self.shrunk.remove(&(slot + 1));
                }
                self.ran = self.machine.ran;
            }
            let wakes = self.ui.wakes.load(std::sync::atomic::Ordering::Relaxed);
            if wakes != self.wakes {
                self.wakes = wakes;
                logf!("ui: Home Assistant wakes the display");
                self.machine.wake_up(now, lit, &mut effects);
            }
            if self.ui.keep_on.load(std::sync::atomic::Ordering::Relaxed)
                && now.duration_since(self.kept_on) >= Duration::from_secs(4) {
                // Kept on by Home Assistant: nlclient's display too.
                self.kept_on = now;
                device::wake_display();
            }
            if let Some((down, false, was_lit)) = self.pressed {
                if now.duration_since(down) >= LONG_PRESS {
                    self.pressed = Some((down, true, was_lit));
                    self.machine.input(Input::LongPress, now, was_lit, &mut effects);
                }
            }
            if self.machine.mode == Mode::Native && self.machine.settings_open
                && now.duration_since(self.native_input) >= Duration::from_millis(600)
                && now.duration_since(self.settings_checked) >= Duration::from_millis(200) {
                // In nlclient's Settings opened from ours: left (its Done, or
                // its Back) once its home dial shows, steady, untouched. A
                // page on its way somewhere, or a tile that looks like the
                // dial, doesn't hold for long enough.
                self.settings_checked = now;
                let page = self.display.save();
                self.settings_seen = if dial(&page) && !settings_tile(&page) { self.settings_seen + 1 } else { 0 };
                if self.settings_seen >= SETTINGS_LEFT {
                    self.settings_seen = 0;
                    logf!("ui: nlclient's Settings left");
                    self.machine.input(Input::SettingsDone, now, lit, &mut effects);
                }
            }
            if let Some(since) = self.back_since {
                // Back pressed in nlclient's menu: once its dial shows, the cards.
                if self.machine.mode != Mode::Native || now.duration_since(since) > Duration::from_millis(1500) {
                    self.back_since = None;
                } else if now.duration_since(since) > Duration::from_millis(250) && home_dial(&mut self.display) {
                    self.back_since = None;
                    self.machine.input(Input::NativeBack, now, lit, &mut effects);
                }
            }
            let display = &mut self.display;
            self.machine.tick(now, lit, &mut || home_dial(display), &mut effects);
            if std::mem::discriminant(&before) != std::mem::discriminant(&self.machine.mode)
                || matches!(self.machine.mode, Mode::Chooser(_)) && before != self.machine.mode {
                mode_state(&self.machine.mode);
            }
            self.schedule_pulse(&before, now);
            self.transition = transition(&before, &self.machine.mode, turned && lit);
            if std::mem::take(&mut self.machine.woke) {
                // Woken: the thermostat as it lit, zooming out to the cards.
                self.zoom = 1.0;
                self.transition = Transition::Cut;
            }
            if matches!(before, Mode::Chooser(_)) && !matches!(self.machine.mode, Mode::Chooser(_) | Mode::Home) {
                // Opened: into the card before what it opens shows.
                self.zoom_in();
            }
            for effect in effects {
                self.apply(effect);
            }
            let elapsed = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
            self.last_frame = now;
            let swept = self.sweep(elapsed);
            let mut dirty = self.machine.take_dirty() | std::mem::take(&mut self.force_draw) || self.pulse.is_some()
                || self.gliding();
            if self.machine.mode == Mode::Glance && unix_now() as i64 / 60 != self.glance_minute {
                // The glance's clock, as the minute turns.
                self.glance_minute = unix_now() as i64 / 60;
                dirty = true;
            }
            if now.duration_since(self.progress_checked) >= Duration::from_millis(500) {
                self.progress_checked = now;
                dirty |= self.progress_moved();
            }
            if self.machine.owns() && self.display.lost() {
                // The display was panned away (as it lit): show the UI again, whole.
                dirty = true;
                self.live = None;
            }
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
            if matches!(self.machine.mode, Mode::Home | Mode::Native) && lit && !self.machine.owns()
                && now.duration_since(self.last_dial) >= Duration::from_secs(3) {
                // The thermostat as it shows, for the next wake.
                self.last_dial = now;
                let page = self.display.save();
                if dial(&page) {
                    self.dial = Some(page);
                }
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
        following || zooming || self.sweeping() || self.pulse.is_some() || self.gliding()
    }

    /// Whether a list's highlight is on its way to its row.
    fn gliding(&self) -> bool {
        self.glide.as_ref().is_some_and(|g| (g.y - g.target).abs() > 0.3)
            && screens::list_place(&match Self::screen(&self.machine, &self.machine.mode) {
                Some(screen) => screen,
                None => return false,
            }).is_some()
    }

    /// The thermostat's dial pulses from the temperature now to its target,
    /// soon after it opens and then every PULSE_EVERY, while it heats or
    /// cools towards it.
    fn schedule_pulse(&mut self, before: &Mode, now: Instant) {
        let reaching = match (&self.machine.mode, &self.machine.nest) {
            (Mode::Thermostat { target }, Some(nest)) => {
                matches!(nest.card.action.as_str(), "heating" | "cooling")
                    && nest.card.current.is_some_and(|c| (c - target).abs() >= nest.card.step)
            }
            _ => false,
        };
        if !reaching {
            self.pulse = None;
            return;
        }
        if !matches!(before, Mode::Thermostat { .. }) {
            self.next_pulse = now + Duration::from_millis(700);
        }
        if self.pulse.is_some_and(|at| now.duration_since(at) >= PULSE) {
            self.pulse = None;
            self.machine_dirty();
        }
        if self.pulse.is_none() && now >= self.next_pulse {
            self.pulse = Some(now);
            self.next_pulse = now + PULSE_EVERY;
        }
    }

    /// Draw again: the pulse just ended, its last frame to settle.
    fn machine_dirty(&mut self) {
        self.force_draw = true;
    }

    /// Zoomed out on the cards, the chooser; full size settling back onto
    /// the thermostat, or opening a card (zoom_in).
    fn zoom_target(&self) -> f32 {
        match self.machine.mode {
            Mode::Chooser(_) => zoom::ZOOMED,
            Mode::Home if self.machine.owns() => zoom::ZOOMED,
            _ => 1.0,
        }
    }

    /// Zooms into the card the cards rest on, to full size: a card opening.
    fn zoom_in(&mut self) {
        let Some(view) = self.view else { return };
        let at = view.round();
        let mut last = Instant::now();
        while self.zoom < 1.0 {
            std::thread::sleep(Duration::from_millis(8));
            let elapsed = last.elapsed().as_secs_f32();
            last = Instant::now();
            let mut next = self.zoom + (1.0 - self.zoom) * (1.0 - (-elapsed / ZOOM_EASE).exp());
            if 1.0 - next < 0.002 {
                next = 1.0;
            }
            self.zoom = next;
            self.show_cards(at);
        }
        self.view = Some(at);
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
        let count = self.machine.deck().len() as i64;
        if left < 0 || right >= count {
            // Past an end: only the end card.
            let frame = self.card_frame(at.round().clamp(0.0, count as f32 - 1.0) as i64);
            self.display.show(&frame);
            return;
        }
        self.prepare(left);
        if part < 0.001 {
            let frame = self.card_frame(left);
            self.display.show(&frame);
            return;
        }
        self.prepare(right);
        let (cards, home) = (&self.cards, &self.home);
        let get = |at: i64| match at.rem_euclid(count) as usize {
            0 => home.as_deref(),
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
        for k in (at.floor() as i64 - 2).max(0)..=(at.ceil() as i64 + 2).min(count - 1) {
            self.prepare(k);
            let index = k.rem_euclid(count) as usize;
            if shrunk && !self.shrunk.contains_key(&index) {
                let full = if index == 0 { self.home.clone() } else { self.cards.get(&index).cloned() };
                if let Some(full) = full {
                    self.shrunk.insert(index, zoom::shrink(&full));
                }
            }
        }
        let (cards, shrunk_cards, home) = (&self.cards, &self.shrunk, &self.home);
        let get = |k: i64| -> Option<&[u8]> {
            if !(0..count).contains(&k) {
                return None; // the cards stop at either end
            }
            let index = k as usize;
            if shrunk {
                shrunk_cards.get(&index).map(Vec::as_slice)
            } else if index == 0 {
                home.as_deref()
            } else {
                cards.get(&index).map(Vec::as_slice)
            }
        };
        zoom::compose(&mut self.scratch, at, z, if shrunk { zoom::ZOOMED } else { 1.0 }, get);
        // The chosen card's name above and the dots under, as they zoom
        // out; the name fades out and in between cards.
        let fade = ((1.0 - self.zoom) / (1.0 - zoom::ZOOMED)).clamp(0.0, 1.0);
        let nearest = at.round().clamp(0.0, count as f32 - 1.0) as usize;
        let title = fade * (1.0 - 2.0 * (at - at.round()).abs()).clamp(0.0, 1.0);
        let name = self.name(nearest);
        paint::around_cards(&mut self.scratch, &self.fonts, &name, title, count as usize, nearest, fade);
        if self.machine.hint {
            paint::hint(&mut self.scratch, &self.fonts, "Turn to browse · press to open");
        }
        self.display.show(&self.scratch);
    }

    /// What the card at `index` in the deck is called, above it.
    fn name(&self, index: usize) -> String {
        match self.machine.deck().get(index) {
            Some(Slot::Thermostat) => "Thermostat".into(),
            Some(Slot::Settings) => "Settings".into(),
            Some(&Slot::Card(slot)) => match self.machine.cards.cards.get(slot) {
                Some(Card::Lights(card)) => card.name.clone(),
                Some(Card::Climate(card)) => card.name.clone(),
                Some(Card::Media(card)) => card.name.clone(),
                Some(Card::Action(card)) => card.name.clone(),
                Some(Card::Camera(card)) => card.name.clone(),
                Some(Card::Options(card)) => card.name.clone(),
                Some(Card::Cover(card)) => card.name.clone(),
                Some(Card::Unknown) | None => String::new(),
            },
            None => String::new(),
        }
    }

    /// Forgets each media card whose progress bar has moved a pixel, so it
    /// is drawn again; whether any had.
    fn progress_moved(&mut self) -> bool {
        let now = unix_now();
        let mut moved = false;
        for (index, slot) in self.machine.deck().into_iter().enumerate() {
            let Slot::Card(slot) = slot else { continue };
            let Some(Card::Media(card)) = self.machine.cards.cards.get(slot) else { continue };
            let Some(done) = card.progress(now) else { continue };
            let pixels = (done * screens::PROGRESS_WIDTH) as u32;
            if self.progress.insert(index, pixels).is_some_and(|was| was != pixels) {
                self.cards.remove(&index);
                self.shrunk.remove(&index);
                moved = true;
            }
        }
        moved
    }

    /// Draws card `at` (wrapping round the deck) if it isn't yet.
    fn prepare(&mut self, at: i64) {
        let deck = self.machine.deck();
        let index = at.rem_euclid(deck.len() as i64) as usize;
        if index == 0 || self.cards.contains_key(&index) {
            return;
        }
        let card = |slot: usize| self.machine.cards.cards.get(slot);
        let screen = match deck[index] {
            Slot::Card(slot) => match card(slot) {
                Some(Card::Lights(card)) => Screen::LightsCard { card },
                Some(Card::Climate(card)) => Screen::ClimateCard { card, target: None },
                Some(Card::Media(card)) => Screen::MediaCard { card, volume: None, progress: card.progress(unix_now()) },
                Some(Card::Action(card)) => {
                    Screen::ActionCard { card, done: self.machine.ran.is_some_and(|(ran, _)| ran == slot) }
                }
                Some(Card::Camera(card)) => Screen::CameraCard { card },
                Some(Card::Options(card)) => Screen::OptionsCard { card },
                Some(Card::Cover(card)) => Screen::CoverCard { card },
                Some(Card::Unknown) | None => return,
            },
            _ => Screen::SettingsCard,
        };
        let mut frame = device::frame(screens::draw(&self.fonts, &screen, None));
        zoom::round(&mut frame);
        self.cards.insert(index, frame);
    }

    /// Card `at` as a frame (nlclient's for the thermostat).
    fn card_frame(&mut self, at: i64) -> device::Frame {
        self.prepare(at);
        let index = at.rem_euclid(self.machine.deck().len() as i64) as usize;
        let frame = if index == 0 { self.home.clone() } else { self.cards.get(&index).cloned() };
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
            Mode::LightWarmth { slot, light, value } => match card(*slot) {
                Some(Card::Lights(c)) => {
                    let [low, high] = c.lights.get(*light)?.kelvins.unwrap_or([2000, 6500]);
                    Some((*value as f32 - low as f32) / (high as f32 - low as f32).max(1.0))
                }
                _ => None,
            },
            Mode::LightHue { value, .. } => Some(*value as f32 / 360.0),
            Mode::CoverSet { value, .. } => Some(*value as f32 / 100.0),
            Mode::Thermostat { target } => {
                let card = &self.machine.nest.as_ref()?.card;
                Some(((target - card.min) / (card.max - card.min).max(0.5)) as f32)
            }
            Mode::SettingSet { setting, value } => {
                let (low, high, _) = setting.range(self.machine.nest.as_ref()?.fahrenheit)?;
                Some(((value - low) / (high - low).max(0.5)) as f32)
            }
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
                // nlclient's page. Just woken, its page is dark until
                // nlclient draws its dial: wait for that (as the display
                // lights), else use the dial as it last showed.
                let mut page = self.display.save();
                let waiting = Instant::now();
                while !dial(&page) && blank(&page) && waiting.elapsed() < Duration::from_millis(1200) {
                    std::thread::sleep(Duration::from_millis(40));
                    page = self.display.save();
                }
                self.saved = Some(match &self.dial {
                    Some(last) if !dial(&page) => last.clone(),
                    _ => page,
                });
                device::own(true);
                self.display.begin();
                // The thermostat as a card, and zoomed out, before the first
                // frame needs it: our own, or nlclient's as it showed.
                if self.machine.own_thermostat() {
                    self.draw_home();
                } else if let Some(saved) = &self.saved {
                    let mut home = saved.clone();
                    zoom::round(&mut home);
                    self.shrunk.insert(0, zoom::shrink(&home));
                    self.home = Some(home);
                }
            }
            Effect::Own(false) => {
                self.shrunk.remove(&0);
                // Back to the thermostat: the cards run on to it as the ring
                // turned, or fade to it when left alone; opened, it is already
                // there. Then nlclient's frame back before nlclient may
                // present again.
                if let Some(saved) = self.saved.clone() {
                    match (self.transition, self.view) {
                        _ if self.machine.mode == Mode::Native => {}
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
                self.home = None;
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
            Effect::Chime => {
                if self.sender.clicks() {
                    self.beeper.chime();
                }
            }
            Effect::KeepAwake => crate::wake::keep_awake_for(30),
            Effect::NestDial(on) => {
                let kept = if on { std::fs::write(NEST_DIAL, b"1") } else { std::fs::remove_file(NEST_DIAL) };
                if let Err(e) = kept.or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
                    logf!("ui: {NEST_DIAL}: {e}");
                }
                logf!("ui: the thermostat shows {}", if on { "Nest's own dial" } else { "its card" });
                self.home = None;
                self.shrunk.remove(&0);
                if self.machine.owns() {
                    if on {
                        if let Some(saved) = &self.saved {
                            let mut home = saved.clone();
                            zoom::round(&mut home);
                            self.home = Some(home);
                        }
                    } else {
                        self.draw_home();
                    }
                }
            }
            Effect::OpenNestSettings => {
                // Its menu, then on to Settings: each step only once the
                // display shows the one before, so a turn only ever moves
                // the menu, never the temperature. Else the menu stays.
                std::thread::sleep(Duration::from_millis(300));
                self.input.press();
                let waiting = Instant::now();
                while !back_chosen(&self.display.save()) {
                    if waiting.elapsed() > Duration::from_millis(1500) {
                        logf!("ui: nlclient's menu didn't show Back: left at the menu");
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
                // Once the menu has finished opening (it takes no turn while
                // it animates), Settings is the next item anticlockwise.
                std::thread::sleep(Duration::from_millis(600));
                self.input.hand_turn(40, 13);
                std::thread::sleep(Duration::from_millis(450));
                if settings_chosen(&self.display.save()) {
                    self.input.press();
                    self.native_input = Instant::now();
                    self.settings_seen = 0;
                    logf!("ui: nlclient's Settings");
                } else {
                    logf!("ui: nlclient's menu didn't show Settings: left at the menu");
                }
            }
            Effect::Send(Command::Nest(name, arg)) => {
                logf!("ui: {name} {arg}");
                self.sender.nest_command(&name, &arg);
                self.refresh_nest();
            }
            Effect::Send(command) => self.sender.send(&command),
        }
    }

    /// Takes the thermostat's state as nestlocal has it now; its card is
    /// drawn again when that changed.
    fn refresh_nest(&mut self) {
        let Some(state) = self.sender.nest() else { return };
        let nest = Nest::from_state(&state);
        if self.machine.nest.as_ref() == Some(&nest) {
            return;
        }
        // While it's being set, the dial's own target stands.
        self.machine.set_nest(nest);
        if self.machine.own_thermostat() && self.home.is_some() {
            self.draw_home();
        }
    }

    /// The thermostat's own card, as the first of the cards.
    fn draw_home(&mut self) {
        let Some(nest) = &self.machine.nest else { return };
        let screen = Screen::NestCard { nest, now: unix_now() };
        let mut frame = device::frame(screens::draw(&self.fonts, &screen, None));
        zoom::round(&mut frame);
        self.shrunk.insert(0, zoom::shrink(&frame));
        self.home = Some(frame);
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

    /// A device's own screen for `mode`.
    fn screen<'a>(machine: &'a Machine, mode: &Mode) -> Option<Screen<'a>> {
        let card = |slot: usize| machine.cards.cards.get(slot);
        Some(match mode {
            Mode::LightList { slot, selected } => match card(*slot)? {
                Card::Lights(card) => Screen::LightList { card, selected: *selected },
                _ => return None,
            },
            Mode::Members { slot, selected } => match card(*slot)? {
                Card::Lights(card) => Screen::Members { card, selected: *selected },
                _ => return None,
            },
            Mode::OptionList { slot, selected } => match card(*slot)? {
                Card::Options(card) => Screen::OptionList { card, selected: *selected },
                _ => return None,
            },
            Mode::CoverSet { slot, value } => match card(*slot)? {
                Card::Cover(card) => Screen::CoverSet { card, value: *value },
                _ => return None,
            },
            Mode::LightDim { slot, light, on, value } => match card(*slot)? {
                Card::Lights(card) => Screen::LightDim { light: card.lights.get(*light)?, on: *on, value: *value },
                _ => return None,
            },
            Mode::LightWarmth { slot, light, value } => match card(*slot)? {
                Card::Lights(card) => Screen::LightWarmth { light: card.lights.get(*light)?, value: *value },
                _ => return None,
            },
            Mode::LightHue { slot, light, value } => match card(*slot)? {
                Card::Lights(card) => Screen::LightHue { light: card.lights.get(*light)?, value: *value },
                _ => return None,
            },
            Mode::ClimateSet { slot, target } => match card(*slot)? {
                Card::Climate(card) => Screen::ClimateSet { card, target: *target },
                _ => return None,
            },
            Mode::Volume { slot, value } => match card(*slot)? {
                Card::Media(card) => Screen::Volume { card, value: *value },
                _ => return None,
            },
            Mode::Camera { slot } => match card(*slot)? {
                Card::Camera(card) => Screen::CameraView { card },
                _ => return None,
            },
            Mode::Alert { .. } => Screen::Alert { alert: machine.alert.as_ref()? },
            Mode::Glance => Screen::Glance { glance: machine.cards.glance.as_ref()?, cards: &machine.cards.cards,
                                             now: unix_now() as i64 },
            Mode::Thermostat { target } => Screen::NestSet { nest: machine.nest.as_ref()?, target: *target, pulse: None },
            Mode::Settings { selected } => Screen::Settings { rows: machine.settings(), nest: machine.nest.as_ref(),
                                                              nest_dial: machine.nest_dial, selected: *selected },
            Mode::SettingSet { setting, value } => Screen::SettingSet { setting: *setting, nest: machine.nest.as_ref()?,
                                                                        value: *value },
            _ => return None,
        })
    }

    /// A device's ring of actions, over its dimmed screen.
    fn draw_menu(&mut self, view: &Mode, actions: &[Action], selected: usize) {
        let Some(screen) = Self::screen(&self.machine, view) else { return };
        let title = match &screen {
            Screen::LightDim { light, .. } => light.name.clone(),
            Screen::Volume { card, .. } => card.name.clone(),
            Screen::ClimateSet { card, .. } => card.name.clone(),
            Screen::CoverSet { card, .. } => card.name.clone(),
            Screen::NestSet { nest, .. } => nest.card.name.clone(),
            _ => String::new(),
        };
        let presets = match &screen {
            Screen::ClimateSet { card, .. } => card.presets.clone(),
            _ => Vec::new(),
        };
        let mut canvas = screens::draw(&self.fonts, &screen, None);
        let items: Vec<(String, String)> = actions.iter().map(|action| {
            let (icon, label) = match action {
                Action::Back => ("mdi:arrow-left", "Back to devices"),
                Action::Switch(true) => ("mdi:power", "Turn on"),
                Action::Switch(false) => ("mdi:power", "Turn off"),
                Action::Play(true) => ("mdi:play", "Play"),
                Action::Play(false) => ("mdi:pause", "Pause"),
                Action::Warmth => ("mdi:white-balance-sunny", "Warmth"),
                Action::Colour => ("mdi:palette", "Colour"),
                Action::Next => ("mdi:skip-next", "Next"),
                Action::Previous => ("mdi:skip-previous", "Previous"),
                Action::Members => ("mdi:format-list-bulleted", "Each light"),
                Action::Open => ("mdi:arrow-up-bold", "Open"),
                Action::Close => ("mdi:arrow-down-bold", "Close"),
                Action::Eco(true) => ("mdi:leaf", "Eco"),
                Action::Eco(false) => ("mdi:leaf", "Leave Eco"),
                Action::Power(true) => ("mdi:power", "Turn heating on"),
                Action::Power(false) => ("mdi:power", "Turn off"),
                Action::Boost(true) => ("mdi:water-boiler", "Boost hot water, 1 hour"),
                Action::Boost(false) => ("mdi:water-boiler", "Stop the boost"),
                Action::NestDial => ("mdi:thermometer", "Nest's own dial"),
                Action::Preset(i) => {
                    let name = presets.get(*i as usize).cloned().unwrap_or_default();
                    return (screens::preset_icon(&name).to_owned(), screens::capitalise(&name.replace('_', " ")));
                }
            };
            (icon.to_owned(), label.to_owned())
        }).collect();
        let items: Vec<(&str, &str)> = items.iter().map(|(i, l)| (i.as_str(), l.as_str())).collect();
        screens::menu(&mut canvas, &self.fonts, &title, &items, selected);
        let frame = device::frame(canvas);
        // The device's screen redraws whole when the menu closes.
        self.live = None;
        if std::mem::replace(&mut self.transition, Transition::Cut) == Transition::Fade {
            let from = self.display.save();
            self.animate(&from, &frame, Transition::Fade);
        }
        self.display.show(&frame);
    }

    fn draw(&mut self) {
        if let Mode::Menu { view, actions, selected } = self.machine.mode.clone() {
            return self.draw_menu(&view, &actions, selected);
        }
        let mode = self.machine.mode.clone();
        let Some(mut screen) = Self::screen(&self.machine, &mode) else { return };
        if let Screen::NestSet { pulse, .. } = &mut screen {
            *pulse = self.pulse.map(|at| at.elapsed().as_secs_f32() / PULSE.as_secs_f32());
        }
        let sweep = self.sweep.as_ref().map(|(_, shown)| *shown);
        let fade = self.transition == Transition::Fade;
        self.transition = Transition::Cut;
        if let Some((selected, first)) = screens::list_place(&screen) {
            let frame = Self::draw_list(&mut self.glide, &self.fonts, &screen, selected - first, fade);
            if fade {
                let from = self.display.save();
                self.animate(&from, &frame, Transition::Fade);
            }
            self.live = None;
            self.display.show(&frame);
            return;
        }
        self.glide = None;
        // A value screen draws its still part once, then only what changes.
        if let Some(key) = screens::still_key(&screen) {
            let fresh = !matches!(&self.live, Some((kept, ..)) if *kept == key);
            if fresh {
                let still = screens::still(&self.fonts, &screen);
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
        let frame = device::frame(screens::draw(&self.fonts, &screen, sweep));
        if fade {
            let from = self.display.save();
            self.animate(&from, &frame, Transition::Fade);
        }
        self.display.show(&frame);
    }

    /// A list, its highlight on its way to row `slot` of those showing:
    /// gliding there from where it was, or there at once on a new list.
    fn draw_list(glide: &mut Option<Glide>, fonts: &paint::Fonts, screen: &Screen, slot: usize, fresh: bool) -> device::Frame {
        let key = screens::list_key(screen).unwrap_or_default();
        let target = screens::LIST_TOP + slot as f32 * screens::ROW_PITCH;
        // The same list (its kind and its first row), only what it shows changed: kept where it is.
        let same = |g: &Glide| g.key.split(' ').take(2).eq(key.split(' ').take(2));
        match glide {
            Some(g) if g.key == key => {}
            Some(g) if same(g) && !fresh => {
                g.plain = screens::list_layer(fonts, screen, screens::Layer::Plain).expect("a list");
                g.chosen = screens::list_layer(fonts, screen, screens::Layer::Chosen).expect("a list");
                g.key = key;
            }
            _ => {
                *glide = Some(Glide {
                    plain: screens::list_layer(fonts, screen, screens::Layer::Plain).expect("a list"),
                    chosen: screens::list_layer(fonts, screen, screens::Layer::Chosen).expect("a list"),
                    key, y: target, target, drawn: Instant::now(),
                });
            }
        }
        let g = glide.as_mut().expect("just made");
        let elapsed = g.drawn.elapsed().as_secs_f32().min(0.1);
        g.drawn = Instant::now();
        g.target = target;
        let mut next = g.y + (target - g.y) * (1.0 - (-elapsed / GLIDE).exp());
        if (next - target).abs() < 0.3 {
            next = target;
        }
        g.y = next;
        let mut canvas = g.plain.clone();
        let (x, w, h) = screens::PILL;
        paint::pill_window(&mut canvas, &g.chosen, x, g.y, w, h);
        device::frame(canvas)
    }

    /// Fades the display from `from` to `to`.
    fn animate(&mut self, from: &[u8], to: &[u8], transition: Transition) {
        if transition == Transition::Cut {
            return;
        }
        let started = Instant::now();
        let length = FADE;
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
        Mode::LightWarmth { slot, light, .. } => Some((3, slot, light)),
        Mode::LightHue { slot, light, .. } => Some((4, slot, light)),
        Mode::CoverSet { slot, .. } => Some((5, slot, 0)),
        Mode::Volume { slot, .. } => Some((1, slot, 0)),
        Mode::ClimateSet { slot, .. } => Some((2, slot, 0)),
        Mode::Thermostat { .. } => Some((6, 0, 0)),
        Mode::SettingSet { setting, .. } => Some((7, setting as usize, 0)),
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
    let cards = |mode: &Mode| matches!(mode, Mode::Home | Mode::Chooser(_));
    let same_view = std::mem::discriminant(before) == std::mem::discriminant(after) && !cards(before);
    if before == after || same_view || !(before.owning() || after.owning()) || *after == Mode::Native {
        return Transition::Cut;
    }
    if cards(before) && cards(after) && turned {
        return Transition::Cut;
    }
    Transition::Fade
}

/// The wall clock, in Unix seconds.
fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn going_in_and_out_fades_and_the_rest_cuts() {
        assert_eq!(transition(&Mode::Home, &Mode::Chooser(1), true), Transition::Cut, "the cards follow the ring");
        assert_eq!(transition(&Mode::Chooser(4), &Mode::Home, true), Transition::Cut);
        assert_eq!(transition(&Mode::Chooser(2), &Mode::Home, false), Transition::Fade, "left alone");
        let list = Mode::LightList { slot: 0, selected: 0 };
        assert_eq!(transition(&Mode::Chooser(1), &list, true), Transition::Fade);
        assert_eq!(transition(&list, &Mode::Chooser(1), false), Transition::Fade);
        assert_eq!(transition(&Mode::Chooser(4), &Mode::Native, true), Transition::Cut, "nlclient's menu");
        assert_eq!(transition(&list, &Mode::LightList { slot: 0, selected: 1 }, true), Transition::Cut);
    }

    fn png(size: u32, colour: png::ColorType, pixel: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        let mut encoder = png::Encoder::new(&mut data, size, size);
        encoder.set_color(colour);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..size * size).flat_map(|_| pixel.iter().copied()).collect();
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
        data
    }

    #[test]
    fn a_camera_s_jpeg_is_cropped_square_from_its_middle() {
        // 64x48: red on the left half, blue on the right.
        let art = Art::decode(include_bytes!("testdata/still.jpg")).unwrap();
        assert_eq!(art.size, 48, "square, as tall as it is");
        let at = |x: usize, y: usize| &art.rgb[(y * 48 + x) * 3..(y * 48 + x) * 3 + 3];
        assert!(at(4, 24)[0] > 150 && at(4, 24)[2] < 90, "red at the left: {:?}", at(4, 24));
        assert!(at(44, 24)[2] > 150 && at(44, 24)[0] < 90, "blue at the right: {:?}", at(44, 24));
    }

    #[test]
    fn a_picture_too_large_is_refused_before_it_is_decoded() {
        let error = Art::decode(include_bytes!("testdata/wide.jpg")).unwrap_err();
        assert!(error.contains("at most"), "{error}");
        assert!(Art::decode(&[0xFF, 0xD8, 0, 1, 2]).is_err(), "not a JPEG at all");
    }

    #[test]
    fn only_the_latest_notification_s_picture_is_kept() {
        let ui = Ui::default();
        let picture = png(4, png::ColorType::Rgb, &[1, 2, 3]);
        ui.set_art("camera.front", &picture).unwrap();
        for tag in ["a", "b", "c"] {
            ui.set_art(&format!("alert:{tag}"), &picture).unwrap();
        }
        let held = ui.art.lock().unwrap();
        let mut keys: Vec<&String> = held.keys().collect();
        keys.sort();
        assert_eq!(keys, ["alert:c", "camera.front"], "a camera's stays; one notification's");
    }

    #[test]
    fn a_media_player_s_art_joins_its_card() {
        let ui = Ui::default();
        ui.configure(br#"{"cards":[{"kind":"media","id":"media_player.speaker","name":"Speaker"}]}"#).unwrap();
        let (before, _) = ui.snapshot();
        ui.set_art("media_player.speaker", &png(4, png::ColorType::Rgba, &[10, 20, 30, 255])).unwrap();
        let (after, cards) = ui.snapshot();
        assert!(after > before, "new art redraws");
        let Card::Media(card) = &cards.cards[0] else { panic!() };
        assert_eq!(card.art.as_deref(), Some(&Art { size: 4, rgb: [10, 20, 30].repeat(16) }), "alpha dropped");
        ui.set_art("media_player.speaker", &png(4, png::ColorType::Rgba, &[10, 20, 30, 255])).unwrap();
        assert_eq!(ui.snapshot().0, after, "the same art: nothing to redraw");
        assert!(ui.set_art("media_player.speaker", &png(600, png::ColorType::Rgb, &[0, 0, 0])).is_err(), "too big");
        assert!(ui.set_art("media_player.speaker", b"not a png").is_err());
        ui.set_art("media_player.speaker", b"").unwrap();
        let (_, cards) = ui.snapshot();
        assert!(matches!(&cards.cards[0], Card::Media(card) if card.art.is_none()), "empty: none");
    }
}

/// Whether the ring is held, for tools that drive the unit's screen
/// (utils/nest-screen.py): a turn while it isn't reaches the temperature.
fn hold_state(hold: bool) {
    let _ = crate::settings_ui::publish("ui.state", if hold { b"holds" } else { b"leaves" });
}

/// What shows, for tools that drive the unit's screen: they check where
/// they are before each step, so a press never lands on the wrong card.
fn mode_state(mode: &Mode) {
    let name = match mode {
        Mode::Chooser(index) => format!("Chooser {index}"),
        Mode::Menu { view, .. } => format!("Menu {}", format!("{view:?}").split([' ', '{', '(']).next().unwrap_or("")),
        other => format!("{other:?}").split([' ', '{', '(']).next().unwrap_or("").to_owned(),
    };
    let _ = crate::settings_ui::publish("ui.mode", name.as_bytes());
}

/// Whether nlclient shows its home dial: its ring lit around the edge.
fn home_dial(display: &mut device::Display) -> bool {
    dial(&display.save())
}

/// Whether a frame is a tile of nlclient's Settings: a blue disc in the
/// middle, black near the edge above and below it (a cooling dial is blue
/// all over).
fn settings_tile(page: &[u8]) -> bool {
    let stride = page.len() / 480;
    let blue = |x: f32, y: f32| {
        let (x, y) = (x.round() as usize, y.round() as usize);
        let at = x * stride + (479 - y) * 4;
        let (b, r) = (page[at] as i32, page[at + 2] as i32);
        b > 150 && b > r + 80
    };
    let (mut middle, mut middle_blue) = (0, 0);
    for r in (0..70).step_by(6) {
        for i in 0..24 {
            let a = i as f32 * std::f32::consts::PI / 12.0;
            middle += 1;
            middle_blue += blue(240.0 + r as f32 * a.cos(), 240.0 + r as f32 * a.sin()) as u32;
        }
    }
    let (mut edge, mut edge_blue) = (0, 0);
    for r in (175..196).step_by(5) {
        for d in (-30..=30).step_by(6) {
            let a = (d as f32).to_radians();
            for side in [1.0, -1.0] {
                edge += 1;
                edge_blue += blue(240.0 + r as f32 * a.sin(), 240.0 - r as f32 * a.cos() * side) as u32;
            }
        }
    }
    middle_blue * 2 > middle && edge_blue * 5 < edge
}

/// Whether nlclient's menu shows with Back chosen: its arrow, lower left,
/// blue (unchosen it is grey).
fn back_chosen(page: &[u8]) -> bool {
    let stride = page.len() / 480;
    let mut blue = 0;
    for x in 78..146 {
        for y in 330..404 {
            // Upright (x, y) is framebuffer row x, column 479 - y, as BGRX.
            let at = x * stride + (479 - y) * 4;
            let (b, r) = (page[at] as i32, page[at + 2] as i32);
            blue += (b > 150 && b > r + 80) as u32;
        }
    }
    blue > 300
}

/// Whether nlclient's menu shows Settings chosen: its cog, below the
/// date, lit white.
fn settings_chosen(page: &[u8]) -> bool {
    let stride = page.len() / 480;
    let mut white = 0;
    for x in 160..222 {
        for y in 384..446 {
            let at = x * stride + (479 - y) * 4;
            white += page[at..at + 3].iter().all(|&c| c > 200) as u32;
        }
    }
    white > 250
}

/// Whether a frame is (nearly) all black: nlclient's page, the display
/// just woken, before nlclient draws.
fn blank(page: &[u8]) -> bool {
    page.as_chunks::<4>().0.iter().step_by(97).filter(|px| px[..3].iter().any(|&c| c > 24)).count() < 20
}

/// Whether a frame shows nlclient's home dial: its ring lit around the edge.
fn dial(page: &[u8]) -> bool {
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
    use machine::Setting;

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
        let shots = [
            ("1-lights", Screen::LightsCard { card: lights }),
            ("2-climate", Screen::ClimateCard { card: climate, target: None }),
            ("3-media", Screen::MediaCard { card: media, volume: None, progress: Some(0.4) }),
            ("4-settings", Screen::SettingsCard),
            ("5-list", Screen::LightList { card: lights, selected: 0 }),
            ("6-dim", Screen::LightDim { light: &lights.lights[0], on: true, value: 60 }),
            ("7-climate-set", Screen::ClimateSet { card: climate, target: 22.0 }),
            ("8-volume", Screen::Volume { card: media, value: 35 }),
        ];
        for (name, screen) in shots {
            save(&screens::draw(&fonts, &screen, None), &format!("{out}/{name}.png"));
        }
        let mut menu = screens::draw(&fonts, &Screen::LightDim { light: &lights.lights[0], on: true, value: 60 }, None);
        screens::menu(&mut menu, &fonts, "Reading Lamp", &[("mdi:arrow-left", "Back to devices"), ("mdi:power", "Turn off")], 0);
        save(&menu, &format!("{out}/9-menu.png"));
        let lamp = model::LightsCard { name: "Reading Lamp".into(), icon: String::new(), lights: vec![lights.lights[0].clone()], members: vec![] };
        save(&screens::draw(&fonts, &Screen::LightsCard { card: &lamp }, None), &format!("{out}/1b-lamp.png"));
        let scene = model::ActionCard { id: "scene.goodnight".into(), name: "Goodnight".into(), icon: "mdi:weather-night".into() };
        save(&screens::draw(&fonts, &Screen::ActionCard { card: &scene, done: false }, None), &format!("{out}/a1-scene.png"));
        save(&screens::draw(&fonts, &Screen::ActionCard { card: &scene, done: true }, None), &format!("{out}/a2-scene-done.png"));
        let size = 480;
        let still = Arc::new(Art { size, rgb: (0..size * size).flat_map(|i| {
            let (x, y) = (i % size, i / size);
            [(60 + x / 6) as u8, (70 + y / 5) as u8, (90 + (x + y) / 12) as u8]
        }).collect() });
        let camera = model::CameraCard { id: "camera.front_door".into(), name: "Front door".into(), icon: String::new(), art: Some(still.clone()) };
        save(&screens::draw(&fonts, &Screen::CameraCard { card: &camera }, None), &format!("{out}/a3-camera.png"));
        let note = model::Alert { id: "wash".into(), title: "Washing's done".into(),
            message: "The washing machine finished at 21:40. Hang it out before bed.".into(), icon: "mdi:washing-machine".into(), ..Default::default() };
        save(&screens::draw(&fonts, &Screen::Alert { alert: &note }, None), &format!("{out}/a4-alert.png"));
        let ring = model::Alert { id: "door".into(), title: "Someone's at the front door".into(), art: Some(still), ..Default::default() };
        save(&screens::draw(&fonts, &Screen::Alert { alert: &ring }, None), &format!("{out}/a5-doorbell.png"));
        let boost = model::OptionsCard { id: "select.hot_water_boost".into(), name: "Hot water boost".into(),
            icon: "mdi:water-boiler".into(), options: vec!["Off".into(), "30 minutes".into(), "1 hour".into(), "2 hours".into()],
            current: Some("Off".into()) };
        save(&screens::draw(&fonts, &Screen::OptionsCard { card: &boost }, None), &format!("{out}/b1-options.png"));
        save(&screens::draw(&fonts, &Screen::OptionList { card: &boost, selected: 2 }, None), &format!("{out}/b2-option-list.png"));
        let blind = model::CoverCard { id: "cover.blind".into(), name: "Bedroom blind".into(), icon: String::new(),
            state: "open".into(), position: Some(60), positionable: true };
        save(&screens::draw(&fonts, &Screen::CoverCard { card: &blind }, None), &format!("{out}/b3-cover.png"));
        save(&screens::draw(&fonts, &Screen::CoverSet { card: &blind, value: 60 }, None), &format!("{out}/b4-cover-set.png"));
        let group = model::LightsCard { name: "Bedroom Lights".into(), icon: String::new(), lights: vec![lights.lights[2].clone()],
            members: lights.lights.clone() };
        save(&screens::draw(&fonts, &Screen::Members { card: &group, selected: 1 }, None), &format!("{out}/b5-members.png"));
        let glance = model::Glance { offset: 0, weather: Some(model::Weather { condition: "partlycloudy".into(),
            temperature: Some(8.4), unit: "°C".into() }) };
        save(&screens::draw(&fonts, &Screen::Glance { glance: &glance, cards: &cards.cards, now: 1_794_172_380 }, None),
             &format!("{out}/b6-glance.png"));
        let mut tunable = lights.lights[0].clone();
        (tunable.kelvin, tunable.kelvins, tunable.hues, tunable.hue) = (Some(2700), Some([2200, 6500]), true, Some(200));
        save(&screens::draw(&fonts, &Screen::LightWarmth { light: &tunable, value: 2700 }, None), &format!("{out}/9b-warmth.png"));
        save(&screens::draw(&fonts, &Screen::LightHue { light: &tunable, value: 200 }, None), &format!("{out}/9c-colour.png"));
        let mut with_art = media.clone();
        let size = model::ART_MAX * 7 / 10;
        let rgb = (0..size * size).flat_map(|i| [(i % size * 255 / size) as u8, (i / size * 255 / size) as u8, 160]);
        with_art.art = Some(Arc::new(Art { size, rgb: rgb.collect() }));
        save(&screens::draw(&fonts, &Screen::MediaCard { card: &with_art, volume: None, progress: Some(0.4) }, None), &format!("{out}/3b-art.png"));
        let state = |extra: serde_json::Value| {
            let mut s = serde_json::json!({"mode": "heat", "action": "idle", "target": 20.5, "current_temperature": 19.5,
                "preset": "none", "temperature_scale": "C", "can_heat": true, "has_hot_water_control": true,
                "hot_water_boost": "off", "away_temperature_low": 12.0, "learning_mode": true, "auto_away_enable": true,
                "version": "v0.4.9", "update": null});
            for (k, v) in extra.as_object().unwrap() {
                s[k] = v.clone();
            }
            Nest::from_state(s.as_object().unwrap())
        };
        let now = 1_794_172_380.0;
        let heating = state(serde_json::json!({"action": "heating", "target": 21.5,
            "time_to_target": "2026-11-09T08:10:00Z", "hot_water_boost": "60"}));
        let shots = [
            ("t1-nest-idle", state(serde_json::json!({}))),
            ("t2-nest-heating", heating.clone()),
            ("t3-nest-eco", state(serde_json::json!({"preset": "eco"}))),
            ("t4-nest-off", state(serde_json::json!({"mode": "off", "action": "off"}))),
        ];
        for (name, nest) in &shots {
            save(&screens::draw(&fonts, &Screen::NestCard { nest, now }, None), &format!("{out}/{name}.png"));
        }
        save(&screens::draw(&fonts, &Screen::NestSet { nest: &heating, target: 22.0, pulse: None }, None), &format!("{out}/t5-nest-set.png"));
        let mut menu = screens::draw(&fonts, &Screen::NestSet { nest: &heating, target: 21.5, pulse: None }, None);
        screens::menu(&mut menu, &fonts, "Thermostat", &[("mdi:arrow-left", "Back to devices"), ("mdi:leaf", "Eco"),
            ("mdi:power", "Turn off"), ("mdi:water-boiler", "Stop the boost"), ("mdi:thermometer", "Nest's own dial")], 1);
        save(&menu, &format!("{out}/t6-nest-menu.png"));
        let idle = state(serde_json::json!({"update": "v0.5.0"}));
        let mut machine = Machine::new(Instant::now());
        machine.set_nest(idle.clone());
        for (name, selected) in [("t7-settings", 0), ("t8-settings-end", 9)] {
            save(&screens::draw(&fonts, &Screen::Settings { rows: machine.settings(), nest: Some(&idle), nest_dial: false, selected }, None),
                 &format!("{out}/{name}.png"));
        }
        save(&screens::draw(&fonts, &Screen::SettingSet { setting: Setting::EcoLow, nest: &idle, value: 12.0 }, None),
             &format!("{out}/t9-eco-set.png"));
        let cold = state(serde_json::json!({"action": "heating", "target": 22.0, "current_temperature": 19.5,
            "time_to_target": "2026-11-09T08:13:00Z"}));
        for (name, phase) in [("p1-pulse-start", Some(0.15)), ("p2-pulse-mid", Some(0.4)), ("p3-pulse-fade", Some(0.85)), ("p0-rest", None)] {
            save(&screens::draw(&fonts, &Screen::NestSet { nest: &cold, target: 22.0, pulse: phase }, None),
                 &format!("{out}/{name}.png"));
        }
        let mut zoomed = vec![0u8; (paint::SIZE * paint::SIZE * 4) as usize];
        let frames: Vec<device::Frame> = [Screen::NestCard { nest: &cold, now }, Screen::SettingsCard,
            Screen::ActionCard { card: &scene, done: false }].iter().map(|screen| {
                let mut frame = screens::draw(&fonts, screen, None).pixmap.data().to_vec();
                zoom::round(&mut frame);
                zoom::shrink(&frame)
            }).collect();
        zoom::compose(&mut zoomed, 1.0, zoom::ZOOMED, zoom::ZOOMED, |k| frames.get(k as usize).map(Vec::as_slice));
        let canvas = paint::Canvas::from_frame(zoomed);
        save(&canvas, &format!("{out}/z1-zoomed.png"));
    }
}
