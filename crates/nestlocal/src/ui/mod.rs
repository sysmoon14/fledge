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
//! Left alone, the screen times out where it is; the next wake starts again. With no cards from Home
//! Assistant, or no nlclient (Doom), it leaves everything alone.

mod device;
#[cfg(test)]
mod film;
mod icons;
mod machine;
mod model;
mod motion;
mod nlstate;
use nlstate::Shows;
mod paint;
mod prof;
mod screens;
mod zoom;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use model::Command;
use machine::{Action, Dial, Effect, Input, Machine, Mode, Slot};
use model::{AlarmResult, Alert, Art, Card, Cards, ClimateChoice, Nest, OptionsCard};
use std::borrow::Cow;
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
///   5  the thermostat anywhere among the cards (thermostat)
///   6  what someone across the room and someone close bring (presence:
///      far glance, wake or dark; near wake, glance or none), and how long
///      the screen stays untouched (card_idle)
///   7  the thermostat's own name (thermostat_name); the screen stays on
///      whatever was opened until it times out
///   8  turning a dimmer down past its lowest turns the light off, a
///      switch's ring turns it off and on (dim_off, default on)
///   9  alarm cards (arm, disarm; <base>/set/alarm_result)
///  10  the thermostat's card hidden (show_thermostat false); and the
///      act_as_thermostat switch, off taking the card away too
///  11  the glance's layouts, 12-hour clock, lines and night
///      (glance.layout/clock/lines/big/event/colour/night/dim), and a glance
///      to try for a while (<base>/set/glance_try)
pub const PROTOCOL: u32 = 11;

/// The alerts the Nest has shown, as its log tells them (motion.rs):
/// each hands it its own screen until it's done.
static NEST_ALERTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The Nest shows one of its own alerts.
pub fn nest_alert() {
    NEST_ALERTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// What the thermostat's sensors have seen (motion.rs): someone across the
/// room, and someone close, counted.
static SEEN_FAR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SEEN_NEAR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The thermostat's sensors saw someone.
pub fn sensed(sensor: crate::motion::Sensor) {
    let seen = match sensor {
        crate::motion::Sensor::Motion => &SEEN_FAR,
        crate::motion::Sensor::Near => &SEEN_NEAR,
    };
    seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

#[derive(Default)]
pub struct Ui {
    /// Home Assistant's cards and a count of their changes.
    cards: Mutex<(u64, Cards)>,
    /// Each light's colour as last reported, for while it's off (Cards::keep_colours).
    colours: Mutex<HashMap<String, [u8; 3]>>,
    /// Media players' album art, by entity id.
    art: Mutex<HashMap<String, Arc<Art>>>,
    /// Home Assistant's: keep the display lit (<base>/set/screen_on), and
    /// how many times it asked to wake it (<base>/set/screen_wake).
    keep_on: std::sync::atomic::AtomicBool,
    wakes: std::sync::atomic::AtomicU64,
    /// Home Assistant's latest notification (<base>/set/alert), counted.
    alert: Mutex<(u64, Option<Alert>)>,
    /// Its latest answer to an alarm command (<base>/set/alarm_result), counted.
    alarm_result: Mutex<(u64, Option<AlarmResult>)>,
    /// A glance to try for a while (<base>/set/glance_try), counted; None
    /// puts the saved one back.
    glance_try: Mutex<(u64, Option<model::Glance>)>,
}

impl Ui {
    /// Takes Home Assistant's cards (<base>/set/ui).
    pub fn configure(&self, payload: &[u8]) -> Result<(), String> {
        let mut cards = Cards::parse(payload)?;
        cards.keep_colours(&mut self.colours.lock().unwrap());
        let mut held = self.cards.lock().unwrap();
        if held.1 != cards {
            held.0 += 1;
            held.1 = cards;
        }
        Ok(())
    }

    /// Whether Home Assistant's cards say what someone across the room or
    /// close brings: then Fledge, not the Nest, decides (keep_farsight).
    pub fn has_presence(&self) -> bool {
        self.cards.lock().unwrap().1.has_presence()
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

    /// Takes Home Assistant's answer to an alarm command
    /// (<base>/set/alarm_result).
    pub fn set_alarm_result(&self, payload: &[u8]) -> Result<(), String> {
        let result = AlarmResult::parse(payload)?;
        let mut held = self.alarm_result.lock().unwrap();
        *held = (held.0 + 1, Some(result));
        Ok(())
    }

    /// Takes a glance to try for a while (<base>/set/glance_try): the
    /// panel's unsaved glance on the real screen; empty ends it.
    pub fn try_glance(&self, payload: &[u8]) -> Result<(), String> {
        let glance = if payload.iter().all(u8::is_ascii_whitespace) {
            None
        } else {
            Some(serde_json::from_slice::<model::Glance>(payload).map_err(|e| e.to_string())?)
        };
        let mut held = self.glance_try.lock().unwrap();
        *held = (held.0 + 1, glance);
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
    /// can_heat, can_cool, has_hot_water_control, has_fan, version and update (a
    /// newer release): its own card and Settings. None until it reports.
    fn nest(&self) -> Option<serde_json::Map<String, serde_json::Value>> {
        None
    }
    /// One of this thermostat's own commands, as <base>/set/<name> takes it.
    fn nest_command(&self, _name: &str, _arg: &str) {}
}

/// Kept while the thermostat shows Nest's own dial rather than its card.
const NEST_DIAL: &str = "/media/data/fledge-nest-dial";
/// The theme the screens wear: "nest" for the Nest's own look; Fledge's
/// otherwise (the default), or if it isn't there.
const THEME: &str = "/media/data/fledge-theme";

/// Whether the screens wear Fledge's theme, as `kept` (THEME's contents) says.
fn fledge_theme(kept: Option<&str>) -> bool {
    kept.is_none_or(|kept| kept.trim() != "nest")
}

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
        // One left off by a nestlocal that stopped would keep the Nest dark.
        device::backlight(true);
        device::favour();
        paint::for_panel();
        let _ = std::thread::Builder::new().name("ui-warm".into()).spawn(paint::warm);
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
    /// What each card's frame was drawn from, and the cards whose frames
    /// are out of date: kept showing while the cards move (drawing one
    /// takes several frames' time), drawn again once they rest.
    drawn: std::collections::HashMap<usize, Drawn>,
    stale: std::collections::HashSet<usize>,
    /// When the cards were last checked against what they were drawn from
    /// (a media card's progress bar moves by itself).
    progress_checked: Instant,
    /// Whether the last turn of the loop showed a frame: the display paces
    /// the next (device::Display's flip waits for it), so no wait before it.
    showed: bool,
    /// The wakes Home Assistant asked for that were done, and when the
    /// display was last kept lit for it.
    wakes: u64,
    kept_on: Instant,
    /// The notifications taken, and the scene last shown as run.
    alerts: u64,
    /// The glances to try taken (Ui::try_glance).
    glance_tries: u64,
    /// Home Assistant's alarm answers taken so far.
    alarm_results: u64,
    ran: Option<(usize, Instant)>,
    /// The minute the glance's clock last showed.
    glance_minute: i64,
    /// The Nest's own alerts taken (NEST_ALERTS), and what its sensors saw
    /// (SEEN_FAR, SEEN_NEAR).
    nest_alerts: u64,
    seen: (u64, u64),
    /// Whether the backlight was last switched off (Machine::backlight_off).
    backlight_off: bool,
    /// Whether nlclient was last told the display rests (device::rest), and
    /// the owner's heartbeat, last beaten when.
    rest: bool,
    /// Whether the display shows the UI's black at rest, as last drawn.
    black: bool,
    beat: u32,
    last_beat: Instant,
    /// How big the cards show (1 full size, zoom::zoomed() while moving).
    zoom: f32,
    /// A frame to compose zoomed cards in.
    scratch: device::Frame,
    /// How far the ring shows on a screen with a value, on its way to it.
    sweep: Option<(SweepKey, f32)>,
    /// A value screen kept between frames (paint::Live): what it was drawn
    /// from, the frame, and what changes it.
    live: Option<(String, paint::Canvas, paint::Live)>,
    /// The glance, while its line of what's on scrolls.
    ticker: Option<Ticker>,
    last_frame: Instant,
    last_check: Instant,
    nlclient: bool,
    /// When the button went down, whether its hold was taken as a long
    /// press, and whether the display was lit then.
    pressed: Option<(Instant, bool, bool)>,
    /// On nlclient's screens: whether its menu's Back was chosen as the
    /// button went down, and since when the press has waited for its dial.
    /// Pressed in nlclient's menu, as nlclient told it (true), or with Back
    /// chosen as its pixels showed (false: a patch that doesn't tell).
    back_chosen: Option<bool>,
    back_since: Option<(Instant, bool)>,
    /// In nlclient's Settings, opened from ours: the last input there, when
    /// its dial was last looked for, and how many looks in a row found it.
    native_input: Instant,
    settings_checked: Instant,
    settings_seen: u32,
    /// nlclient's Settings seen since its thermostat was opened from the
    /// cards (as nlclient tells it): its dial next is Done, back to the cards.
    native_settings: bool,
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
    /// When the ring last clicked.
    last_click: Instant,
    /// The cards drawn are out of date (a device changed on its own
    /// screen): drawn again when the cards next show.
    cards_stale: bool,
    /// What the thermostat's own card (home) was last drawn from.
    home_from: Option<Nest>,
    /// Whether the thermostat's dial says what it's set to (machine.set_at).
    set_words: bool,
    /// Whether a media player's page shows its volume large (the ring just
    /// turned), and when its track's progress was last drawn.
    volume_view: bool,
    progress_drawn: Instant,
    /// The next frame fades in (the media page's track back).
    fade_next: bool,
    /// The thermostat's dial changing colour: the frame it fades from, and
    /// when it began; and the still parts kept for it, by still_key.
    recolour: Option<(device::Frame, Instant)>,
    stills: HashMap<String, paint::Canvas>,
    /// How far the dial's temperature now has faded in, and where it's going
    /// (1 while the target is another, else 0).
    now_shown: (f32, f32),
    /// Which side of its tick it shows (1 clockwise, -1 anticlockwise).
    now_side: f64,
    /// nlclient's dial as it last showed lit, and when: the thermostat's
    /// card when the display wakes, its own page dark until nlclient draws.
    dial: Option<device::Frame>,
    last_dial: Instant,
    /// A value shown on its way to where it is: a light swept to Off or
    /// back on (held, or from a menu), or a number Home Assistant changed,
    /// counting to it.
    count: Option<Count>,
    /// The screen lit while the button is down (D2): drawn whole again as
    /// it comes up.
    press_lit: bool,
    /// Frames of the animation under way: how many, how long all told, the
    /// longest.
    frames: (u32, Duration, Duration),
    /// Where those frames' time went (prof), all told and in the longest.
    spent: ([u32; prof::PARTS], [u32; prof::PARTS]),
    /// When the last frame showed, the time between frames one after the
    /// other all told, how many those were, and how many came later than
    /// the display's next refresh but one.
    paced: (Option<Instant>, Duration, u32, u32),
}

/// What a card's frame shows: its card (none for Settings), whether a
/// scene's says it ran, and a media card's progress bar in pixels.
#[derive(Clone, PartialEq)]
struct Drawn {
    card: Option<Card>,
    done: bool,
    progress: Option<u32>,
}

impl Drawn {
    /// Whether `other` is the same card showing something else, rather than
    /// another card in its place.
    fn same_card(&self, other: &Drawn) -> bool {
        match (&self.card, &other.card) {
            (None, None) => true,
            (Some(a), Some(b)) => std::mem::discriminant(a) == std::mem::discriminant(b) && card_name(a) == card_name(b),
            _ => false,
        }
    }
}

fn card_name(card: &Card) -> &str {
    match card {
        Card::Lights(card) => &card.name,
        Card::Climate(card) => &card.name,
        Card::Media(card) => &card.name,
        Card::Action(card) => &card.name,
        Card::Camera(card) => &card.name,
        Card::Options(card) => &card.name,
        Card::Cover(card) => &card.name,
        Card::Alarm(card) => &card.name,
        Card::Unknown => "",
    }
}

/// The glance with a line too long to show whole, scrolling through its
/// place: what it shows (screens::glance_key), its still part and the frame
/// with the words on it, how far the words go before they are back where
/// they began, since when they have been going, and how far along they
/// were last drawn.
struct Ticker {
    key: String,
    still: paint::Canvas,
    canvas: paint::Canvas,
    round: f32,
    since: Instant,
    shown: f32,
}

/// How long the glance's line rests at its start, and how fast it then
/// scrolls, in pixels a second: slow enough to read across a room.
const TICKER_REST: f32 = 1.8;
const TICKER_SPEED: f32 = 36.0;

/// How far along the line is, `elapsed` seconds after it began, going
/// `round` pixels before it's back at its start.
fn ticker_offset(elapsed: f32, round: f32) -> f32 {
    let along = elapsed % (TICKER_REST + round / TICKER_SPEED);
    if along < TICKER_REST { 0.0 } else { (along - TICKER_REST) * TICKER_SPEED }
}

struct Glide {
    key: String,
    plain: paint::Canvas,
    chosen: paint::Canvas,
    /// The same list's other windows, drawn while it rested (a list's two
    /// layers cost a few frames to draw on the unit): by their keys.
    kept: Vec<(String, paint::Canvas, paint::Canvas)>,
    /// The highlight's top, and where it's going.
    y: f32,
    target: f32,
    drawn: Instant,
    /// Scrolled a row: the window it left (its two layers), which way
    /// (1: on, the rows moving up), and since when, for the rows to slide.
    scroll: Option<(paint::Canvas, paint::Canvas, f32, Instant)>,
}

/// A value screen's number and ring on their way from `from` to `to`
/// (as the mode holds them), for the screen keyed `key`; a light switched
/// off shows on until it reaches nothing.
struct Count {
    key: SweepKey,
    from: f64,
    to: f64,
    since: Instant,
    length: Duration,
    lit: bool,
    /// A light switched off or on (not a number Home Assistant changed).
    switch: bool,
}

/// How long the thermostat's pulse runs, and how often.
const PULSE: Duration = Duration::from_millis(1800);
const PULSE_EVERY: Duration = Duration::from_secs(30);
/// How fast a list's highlight glides to its row: most of the way in this.
const GLIDE: f32 = 0.065;
/// A list's windows kept drawn besides the one showing: two on, two back.
const KEPT_WINDOWS: usize = 4;
/// How long the thermostat's dial takes to change colour: slow enough to
/// read as the heating lighting, not a switch.
const RECOLOUR: Duration = motion::SLOW;

/// The least time between clicks.
const CLICK_EVERY: Duration = Duration::from_millis(45);

/// How long the button is held for a long press.
const LONG_PRESS: Duration = Duration::from_millis(600);
/// Looks in a row, 200 ms apart, that find nlclient's home dial after its
/// Settings: it was left.
const SETTINGS_LEFT: u32 = 4;
/// The most items nlclient's menu has (Mode, Eco, Hot water or Fan,
/// Schedule, History, Settings, Back, and one to spare): a lap of it.
const MENU_ITEMS: usize = 8;

/// A frame this long after the one before came late: the display, at 60 Hz,
/// showed the one before twice. And a gap this long is no movement at all.
const LATE: Duration = Duration::from_millis(21);
const LULL: Duration = Duration::from_millis(150);

/// How fast what shows catches up with the ring: most of the way in this.
const FOLLOW: f32 = 0.04;
/// How fast the cards zoom out as the ring starts, and in as they settle.
const ZOOM_EASE: f32 = 0.08;

impl Runtime {
    fn new(ui: Arc<Ui>, sender: Arc<dyn Sender>, fonts: paint::Fonts, display: device::Display,
           input: device::Input) -> Runtime {
        let now = Instant::now();
        Runtime {
            ui, sender, fonts, display, input, beeper: device::Beeper::open(), machine: Machine::new(now),
            generation: u64::MAX, saved: None, home: None, transition: Transition::Cut, view: None,
            cards: Default::default(), shrunk: Default::default(), zoom: 1.0,
            drawn: Default::default(), stale: Default::default(), progress_checked: Instant::now(), showed: false,
            wakes: 0, kept_on: Instant::now(),
            alerts: 0, alarm_results: 0, glance_tries: 0, ran: None, glance_minute: 0, backlight_off: false, nest_alerts: NEST_ALERTS.load(std::sync::atomic::Ordering::Relaxed),
            seen: (SEEN_FAR.load(std::sync::atomic::Ordering::Relaxed), SEEN_NEAR.load(std::sync::atomic::Ordering::Relaxed)),
            rest: false, black: false, beat: 0, last_beat: now,
            scratch: vec![0; (paint::SIZE * paint::SIZE * 4) as usize], sweep: None, live: None, ticker: None, last_frame: now,
            last_check: now - Duration::from_secs(10), nlclient: false, frames: Default::default(), spent: Default::default(), paced: Default::default(),
            pressed: None, back_chosen: None, back_since: None, native_input: now, settings_checked: now, settings_seen: 0, native_settings: false,
            nest_checked: now - Duration::from_secs(10), pulse: None, next_pulse: now, glide: None, force_draw: false, last_click: now, cards_stale: false, home_from: None, set_words: false, volume_view: false, progress_drawn: now, fade_next: false,
            recolour: None, stills: HashMap::new(), now_shown: (0.0, 0.0), now_side: 1.0,
            dial: None, last_dial: now - Duration::from_secs(60), count: None, press_lit: false,
        }
    }

    fn run(mut self) {
        logf!("ui: ready for home screen cards");
        hold_state(false);
        device::rest(false);
        self.machine.nest_dial = std::path::Path::new(NEST_DIAL).exists();
        self.machine.fledge = fledge_theme(std::fs::read_to_string(THEME).ok().as_deref());
        paint::set_theme(if self.machine.fledge { paint::Theme::Fledge } else { paint::Theme::Nest });
        loop {
            // A frame each 8 ms while anything moves (the cards as fast as the
            // display takes them); otherwise input or time.
            // Quick while anything moves; a slower beat when there's nothing
            // to show (no cards, or asleep on nlclient's dial), to wake the
            // unit less, as input still ends the wait at once.
            let idle = matches!(self.machine.mode, Mode::Idle) || (self.machine.mode == Mode::Home && !device::lit());
            let wait = if self.moving() && std::mem::take(&mut self.showed) {
                // The cards on the move: the display's flip paces the frames.
                0
            } else if self.animating() || self.pressed.is_some() || self.back_since.is_some() {
                8
            } else if idle {
                250
            } else {
                50
            };
            let events = self.input.wait(Duration::from_millis(wait));
            let now = Instant::now();
            let mut effects = Vec::new();
            // Not while the cards move: looking through /proc costs a frame.
            if now.duration_since(self.last_check) >= Duration::from_secs(2) && !self.moving() {
                self.last_check = now;
                self.nlclient = !crate::pids("nlclient").is_empty();
                let rests = device::rests();
                if rests != self.machine.rests {
                    logf!("ui: {}", if rests { "holds the display at rest" } else { "hands the display back at rest" });
                    self.machine.set_rests(rests, &mut effects);
                }
            }
            if self.ui.generation() != self.generation {
                let (generation, cards) = self.ui.snapshot();
                self.generation = generation;
                self.machine.set_cards(cards, &mut effects);
                self.revalidate();
                // A notification's picture may come after it.
                let key = self.machine.alert.as_ref().and_then(|a| a.picture.clone());
                if let (Some(alert), Some(key)) = (&mut self.machine.alert, key) {
                    alert.art = self.ui.art(&key);
                }
            }
            self.machine.set_active(self.nlclient, now, &mut effects);
            if now.duration_since(self.nest_checked) >= Duration::from_secs(1) && !self.moving() {
                self.nest_checked = now;
                self.refresh_nest();
            }
            let lit = device::lit();
            let before = self.machine.mode.clone();
            let turned = !events.is_empty();
            // The ring itself, not the button: a count gives way only to the hand.
            let rotated = events.iter().any(|e| e.kind == device::EV_REL);
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
                        if lit {
                            self.light_press();
                        }
                        if self.machine.mode == Mode::Native {
                            // Its menu shows: choosing Back leaves it for the
                            // dial at once, with no other screen between.
                            self.back_chosen = match nlstate::read() {
                                Some(report) => (report.shows() == Shows::Menu).then_some(true),
                                None => back_chosen(&self.display.save()).then_some(false),
                            };
                        }
                        continue;
                    }
                    (device::EV_KEY, device::KEY_ENTER, 0) => match (self.unlight_press(), self.pressed.take()).1 {
                        Some((_, false, was_lit)) => {
                            if let Some(told) = self.back_chosen.take().filter(|_| self.machine.mode == Mode::Native) {
                                self.back_since = Some((now, told));
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
            let (tries, tried) = self.ui.glance_try.lock().unwrap().clone();
            if tries != self.glance_tries {
                self.glance_tries = tries;
                logf!("ui: glance to try: {}", if tried.is_some() { "showing" } else { "ended" });
                self.machine.try_glance(tried, now, lit, &mut effects);
            }
            let (results, result) = self.ui.alarm_result.lock().unwrap().clone();
            if results != self.alarm_results {
                self.alarm_results = results;
                if let Some(result) = result {
                    logf!("ui: alarm {}: {}", result.alarm, if result.ok { "done" } else { result.error.as_str() });
                    self.machine.alarm_result(result, now, &mut effects);
                }
            }
            if self.machine.ran != self.ran {
                // A scene's card says it ran, then goes back.
                self.ran = self.machine.ran;
                self.revalidate();
            }
            let nest_alerts = NEST_ALERTS.load(std::sync::atomic::Ordering::Relaxed);
            if nest_alerts != self.nest_alerts {
                self.nest_alerts = nest_alerts;
                self.machine.nest_alert(now, &mut effects);
            }
            let seen = (SEEN_FAR.load(std::sync::atomic::Ordering::Relaxed), SEEN_NEAR.load(std::sync::atomic::Ordering::Relaxed));
            if seen.0 != self.seen.0 {
                self.machine.sensed(crate::motion::Sensor::Motion, now, lit, &mut effects);
            }
            if seen.1 != self.seen.1 {
                self.machine.sensed(crate::motion::Sensor::Near, now, lit, &mut effects);
            }
            self.seen = seen;
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
                    // Held: what it does shows, unlit.
                    self.unlight_press();
                    self.machine.input(Input::LongPress, now, was_lit, &mut effects);
                }
            }
            if self.machine.mode != Mode::Native {
                self.native_settings = false;
            } else if now.duration_since(self.native_input) >= Duration::from_millis(600)
                && now.duration_since(self.settings_checked) >= Duration::from_millis(200) {
                // In nlclient's Settings: left (its Done, or its Back) once
                // its home dial shows after them, untouched. Opened from ours, back to
                // ours; opened from its own menu (as nlclient tells it), back
                // to the cards on the thermostat. By its pixels (a patch that
                // doesn't tell), only from ours, and the dial must hold: a page
                // on its way somewhere, or a tile that looks like the dial,
                // doesn't hold for long enough.
                self.settings_checked = now;
                let left = match nlstate::read() {
                    Some(report) => {
                        let shows = report.shows();
                        self.native_settings |= shows == Shows::Settings;
                        // Seen first: opened from ours, nlclient is still on
                        // its dial until its menu and Settings open.
                        (shows == Shows::Dial && self.native_settings).then_some(SETTINGS_LEFT)
                    }
                    None if self.machine.settings_open => {
                        let page = self.display.save();
                        (dial(&page) && !settings_tile(&page)).then_some(self.settings_seen + 1)
                    }
                    None => None,
                };
                self.settings_seen = left.unwrap_or(0);
                if self.settings_seen >= SETTINGS_LEFT {
                    self.settings_seen = 0;
                    self.native_settings = false;
                    logf!("ui: nlclient's Settings left");
                    let input = if self.machine.settings_open { Input::SettingsDone } else { Input::NativeBack };
                    self.machine.input(input, now, lit, &mut effects);
                }
            }
            if let Some((since, told)) = self.back_since {
                // Pressed in nlclient's menu: once its dial shows, the cards.
                // As nlclient tells it, Back goes straight from the menu to
                // the dial; another choice (a mode, Settings) shows something
                // else first, and is left to nlclient. By its pixels, Back was
                // chosen as the button went down.
                let shows = if told { nlstate::read().map(|report| report.shows()) } else { None };
                if self.machine.mode != Mode::Native || now.duration_since(since) > Duration::from_millis(1500)
                    || told && !matches!(shows, Some(Shows::Menu | Shows::Dial)) {
                    self.back_since = None;
                } else if if told { shows == Some(Shows::Dial) }
                          else { now.duration_since(since) > Duration::from_millis(250) && home_dial(&mut self.display) } {
                    self.back_since = None;
                    self.machine.input(Input::NativeBack, now, lit, &mut effects);
                }
            }
            let display = &mut self.display;
            self.machine.tick(now, lit, &mut || home_dial(display), &mut effects);
            let off = self.machine.backlight_off();
            if off != self.backlight_off {
                // Kept dark for someone across the room: off, not black.
                self.backlight_off = off;
                device::backlight(!off);
            }
            if std::mem::discriminant(&before) != std::mem::discriminant(&self.machine.mode)
                || matches!(self.machine.mode, Mode::Chooser(_) | Mode::Settings { .. }) && before != self.machine.mode {
                mode_state(&self.machine.mode);
            }
            self.schedule_pulse(&before, now);
            self.fade_now_label(&before);
            let set_words = self.machine.set_at.is_some_and(|at| now.duration_since(at) < machine::SET_SHOWN);
            if std::mem::replace(&mut self.set_words, set_words) != set_words {
                self.force_draw = true;
            }
            if let Mode::Volume { .. } = self.machine.mode {
                // The track back once the ring rests, faded; its progress
                // bar drawn on as it plays.
                let volume = self.machine.volume_at.is_some_and(|at| now.duration_since(at) < machine::VOLUME_SHOWN);
                if std::mem::replace(&mut self.volume_view, volume) && !volume {
                    self.force_draw = true;
                    self.fade_next = true;
                }
                if !volume && now.duration_since(self.progress_drawn) >= Duration::from_secs(1) {
                    self.progress_drawn = now;
                    self.force_draw = true;
                }
            }
            self.transition = transition(&before, &self.machine.mode, turned && lit, &self.machine);
            if std::mem::take(&mut self.machine.woke) {
                // Woken: the thermostat as it lit, zooming out to the cards;
                // from the glance, faded through to it first.
                self.zoom = 1.0;
                if before != Mode::Glance {
                    self.transition = Transition::Cut;
                }
            }
            self.start_count(&before, rotated, now);
            if matches!(before, Mode::Chooser(_)) && !matches!(self.machine.mode, Mode::Chooser(_) | Mode::Home) {
                // Opened: into the card before what it opens shows.
                self.zoom_in();
            }
            for effect in effects {
                self.apply(effect);
            }
            if self.machine.mode != Mode::Home || !self.machine.owns() || self.machine.position().is_some() {
                self.black = false;
            }
            let rest = self.machine.owns() && self.machine.resting();
            if rest != self.rest {
                self.rest = rest;
                device::rest(rest);
            }
            if self.machine.owns() && now.duration_since(self.last_beat) >= Duration::from_secs(1) {
                // Alive: nlclient's patch gives the display back to a hung UI.
                self.last_beat = now;
                self.beat = self.beat.wrapping_add(1) % 1_000_000;
                device::beat(self.beat);
            }
            let elapsed = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
            self.last_frame = now;
            let swept = self.sweep(elapsed);
            if self.machine.mode != Mode::Glance {
                self.ticker = None;
            }
            let counted = self.count.is_some();
            if self.count.as_ref().is_some_and(|c| c.since.elapsed() >= c.length) {
                // There: the screen as it is now, faded to if it went out.
                let lit = self.count.take().is_some_and(|c| c.lit);
                self.fade_next |= !lit && !matches!(self.machine.mode, Mode::LightDim { on: true, .. });
            }
            let mut dirty = self.machine.take_dirty() | std::mem::take(&mut self.force_draw) || self.pulse.is_some()
                || self.gliding() || self.recolour.is_some() || self.ticker_due() || counted;
            if self.machine.mode == Mode::Glance && unix_now() as i64 / 60 != self.glance_minute {
                // The glance's clock, as the minute turns.
                self.glance_minute = unix_now() as i64 / 60;
                dirty = true;
            }
            if now.duration_since(self.progress_checked) >= Duration::from_millis(500) {
                // A media card's progress bar has moved a pixel.
                self.progress_checked = now;
                dirty |= self.revalidate();
            }
            if self.machine.owns() && self.display.lost() {
                // The display was panned away (as it lit): show the UI again, whole.
                dirty = true;
                self.live = None;
                self.ticker = None;
            }
            if dirty && self.machine.position().is_none() {
                // A light switched in a list shows on its card: the cards
                // drawn again once they show, not on every frame of a dial.
                self.cards_stale = true;
            }
            if self.cards_stale && self.machine.position().is_some() {
                self.cards_stale = false;
                self.revalidate();
            }
            if self.machine.position().is_some() && self.machine.own_thermostat() && self.home.is_some()
                && !self.moving() && self.home_from.as_ref() != self.machine.nest.as_ref() {
                // The thermostat changed from its own dial (its target, Eco):
                // its card shows it before the cards do, not when its state
                // next differs from what the dial already set.
                self.draw_home();
            }
            if self.machine.owns() {
                let started = Instant::now();
                let drew = match self.machine.position() {
                    Some(target) => self.follow(target, elapsed, dirty),
                    None => {
                        self.view = None;
                        if self.machine.mode == Mode::Home {
                            // At rest, dark: a turn starts from the thermostat, full size.
                            self.zoom = 1.0;
                        }
                        if self.press_lit && (dirty || swept) {
                            // The button down, its screen lit: nothing drawn
                            // over it (a dial's live frames put back only
                            // what moved, leaving the light round them),
                            // until it comes up and the screen is drawn whole.
                            self.force_draw = true;
                            false
                        } else {
                            (dirty || swept) && {
                                self.draw();
                                true
                            }
                        }
                    }
                };
                let spent = prof::take();
                self.showed = drew;
                if drew {
                    let took = started.elapsed();
                    if took > self.frames.2 {
                        self.spent.1 = spent;
                    }
                    for (sum, part) in self.spent.0.iter_mut().zip(spent) {
                        *sum += part;
                    }
                    self.frames = (self.frames.0 + 1, self.frames.1 + took, self.frames.2.max(took));
                    let shown = Instant::now();
                    if let Some(gap) = self.paced.0.map(|last| shown.duration_since(last)).filter(|gap| *gap < LULL) {
                        let late = (gap > LATE) as u32;
                        self.paced = (Some(shown), self.paced.1 + gap, self.paced.2 + 1, self.paced.3 + late);
                    } else {
                        self.paced.0 = Some(shown);
                    }
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
                if dial_page(&page) {
                    self.dial = Some(page);
                }
            }
            if !self.animating() && matches!(self.machine.mode, Mode::Dial { .. })
                && self.machine.idle(now) >= Duration::from_millis(400) {
                // The dial in its other colour ready while it rests, so the
                // target crossing the temperature now costs no more than a turn.
                if let Some(screen) = Self::screen(&self.machine, &self.machine.mode) {
                    if let Some((key, still)) = Self::other_still(&self.fonts, &screen, &self.stills) {
                        self.stills.insert(key, still);
                    }
                }
            }
            let warmed = !self.animating() && self.warm_list();
            if !warmed && !self.animating() && self.machine.mode != Mode::Idle && !self.cards_stale {
                // One card at a time while nothing moves, so the first turn
                // finds them all drawn and up to date: those nearest what
                // shows first.
                let deck = self.machine.deck().len() as i64;
                let at = self.view.map_or(0, |view| view.round() as i64);
                let nearest = (0..deck).flat_map(|d| [at - d, at + d + 1]).filter(|k| (0..deck).contains(k));
                let keys: Vec<usize> = nearest.map(|k| self.key(k)).filter(|&index| index != 0).collect();
                if let Some(&index) = keys.iter().find(|i| self.stale.contains(i) || !self.cards.contains_key(i)) {
                    self.prepare_key(index);
                    // Shown, if the cards are: it may be one of those showing.
                    self.force_draw |= self.view.is_some();
                } else if let Some(&index) = keys.iter().find(|i| !self.shrunk.contains_key(i) && self.cards.contains_key(i)) {
                    let shrunk = prof::time(prof::Part::Shrink, || zoom::shrink(&self.cards[&index]));
                    self.shrunk.insert(index, shrunk);
                }
            }
            if !self.animating() {
                // Only a movement with a frame that missed the display's
                // 60 Hz: anything else is as it should be.
                let (count, total, longest) = std::mem::take(&mut self.frames);
                let (all, worst) = std::mem::take(&mut self.spent);
                let (_, span, gaps, late) = std::mem::take(&mut self.paced);
                if longest > Duration::from_millis(17) || late * 10 > gaps {
                    let rate = if gaps > 0 { gaps as f32 / span.as_secs_f32() } else { 0.0 };
                    logf!("ui: {count} frames, {:.1} ms each ({}), the longest {} ms ({}); {rate:.0} a second, {late} late",
                          total.as_secs_f32() * 1000.0 / count as f32, prof::describe(&all, count),
                          longest.as_millis(), prof::describe(&worst, 1));
                }
            }
        }
    }

    /// A list at rest: the windows it scrolls to next drawn, one a call, so
    /// the ring finds them ready (and those it won't go to let go). Whether
    /// one was drawn.
    fn warm_list(&mut self) -> bool {
        let Some(g) = self.glide.as_mut() else { return false };
        let Some(screen) = Self::screen(&self.machine, &self.machine.mode) else { return false };
        if screens::list_key(&screen).as_deref() != Some(g.key.as_str()) {
            return false;
        }
        let next: Vec<(String, Screen)> = screens::list_neighbours(&screen).into_iter()
            .filter_map(|s| Some((screens::list_key(&s)?, s))).collect();
        g.kept.retain(|(key, ..)| next.iter().any(|(k, _)| k == key));
        let Some((key, screen)) = next.into_iter().find(|(k, _)| g.kept.iter().all(|(kept, ..)| kept != k)) else {
            return false;
        };
        // Not counted as a frame's drawing: nothing shows for it.
        let plain = screens::list_layer(&self.fonts, &screen, screens::Layer::Plain);
        let chosen = screens::list_layer(&self.fonts, &screen, screens::Layer::Chosen);
        if let (Some(plain), Some(chosen)) = (plain, chosen) {
            g.kept.push((key, plain, chosen));
        }
        true
    }

    /// Whether the cards are on their way somewhere: following the ring,
    /// or zooming.
    fn moving(&self) -> bool {
        let following = match (self.view, self.machine.position()) {
            (Some(view), Some(target)) => (view - self.nearest(view, target)).abs() > 0.001,
            _ => false,
        };
        let zooming = self.view.is_some() && (self.zoom - self.zoom_target()).abs() > 0.002;
        following || zooming
    }

    /// Whether the glance's scrolling line has moved on since it was drawn.
    fn ticker_due(&self) -> bool {
        self.ticker.as_ref().is_some_and(|t| ticker_offset(t.since.elapsed().as_secs_f32(), t.round) != t.shown)
    }

    /// Whether something on the display is on its way somewhere.
    fn animating(&self) -> bool {
        self.moving() || self.sweeping() || self.pulse.is_some() || self.gliding() || self.ticker.is_some()
            || (self.now_shown.0 - self.now_shown.1).abs() > 0.01 || self.recolour.is_some() || self.count.is_some()
    }

    /// The button down: what it will press lit a little, at once, before
    /// whatever it does shows.
    fn light_press(&mut self) {
        if !self.machine.owns() || matches!(self.machine.mode, Mode::Home | Mode::Glance | Mode::Native | Mode::Idle) {
            return;
        }
        let pressed = match &self.machine.mode {
            Mode::Chooser(_) => motion::Pressed::Card(paint::SIZE as f32 / 2.0 * self.zoom),
            _ => match &self.glide {
                Some(g) if Self::screen(&self.machine, &self.machine.mode).is_some_and(|s| screens::list_place(&s).is_some()) => {
                    let (x, w, h) = screens::PILL;
                    motion::Pressed::Row(x, g.y, w, h)
                }
                _ => motion::Pressed::Screen,
            },
        };
        let mut frame = self.display.current();
        motion::press(&mut frame, pressed);
        self.display.show(&frame);
        self.press_lit = true;
    }

    /// The button up: the screen drawn whole again, unlit.
    fn unlight_press(&mut self) {
        if std::mem::take(&mut self.press_lit) {
            self.live = None;
            self.ticker = None;
            self.force_draw = true;
        }
    }

    /// A light switched off or on sweeps its ring and number down to
    /// nothing or up to its brightness (B1); a number Home Assistant
    /// changed, not the ring, counts to it (D3).
    fn start_count(&mut self, before: &Mode, turned: bool, now: Instant) {
        let after = &self.machine.mode;
        let (Some(key), Some(was), Some(is)) = (sweep_key(before), number(before), number(after)) else {
            if sweep_key(after).is_none() {
                self.count = None;
            }
            return;
        };
        if sweep_key(after) != Some(key) {
            self.count = None;
            return;
        }
        // From where it shows now, if it was on its way already.
        let shown = self.count.as_ref().map_or(was, |c| c.from + (c.to - c.from) * motion::curve(c.since.elapsed().as_secs_f32() / c.length.as_secs_f32()) as f64);
        let switched = match (before, after) {
            (Mode::LightDim { on: a, .. }, Mode::LightDim { on: b, .. }) => (a != b).then_some(*b),
            _ => None,
        };
        if let Some(on) = switched {
            let from = if on { 0.0 } else { shown };
            self.count = Some(Count { key, from, to: if on { is } else { 0.0 }, since: now, length: motion::SWEEP, lit: on, switch: true });
        } else if let Some(c) = self.count.as_mut().filter(|c| c.switch && c.lit && !turned) {
            // Home Assistant's own brightness for a light sweeping on: where it ends.
            c.to = is;
        } else if !turned && (is - was).abs() > f64::EPSILON && self.count.as_ref().is_none_or(|c| c.lit) {
            self.count = Some(Count { key, from: shown, to: is, since: now, length: motion::STANDARD, lit: true, switch: false });
        } else if turned {
            // The hand has it: the ring follows the hand.
            self.count = None;
        }
    }

    /// The mode as it shows: with its number where its count has got to.
    fn shown_mode(&self) -> Mode {
        let mode = self.machine.mode.clone();
        let Some(count) = &self.count else { return mode };
        if sweep_key(&mode) != Some(count.key) {
            return mode;
        }
        let t = (count.since.elapsed().as_secs_f32() / count.length.as_secs_f32()).min(1.0);
        let at = count.from + (count.to - count.from) * motion::curve(t) as f64;
        with_number(mode, at)
    }

    /// The dial's temperature now fades out where it is as the target comes
    /// to it, and back in as it leaves: on the side away from the target,
    /// moving sides only once it has gone.
    fn fade_now_label(&mut self, before: &Mode) {
        let (visible, side) = match &self.machine.mode {
            Mode::Dial { of, target } => {
                let Some(card) = self.machine.climate(*of) else { return };
                let target = (target / card.step).round() * card.step;
                let Some(now) = card.current else { return };
                ((now - target).abs() >= card.step / 2.0, if target < now { 1.0 } else { -1.0 })
            }
            _ => return,
        };
        if !matches!(before, Mode::Dial { .. }) {
            let shown = visible as u8 as f32;
            (self.now_shown, self.now_side) = ((shown, shown), side);
            return;
        }
        let (shown, _) = self.now_shown;
        if shown <= 0.0 {
            self.now_side = side;
        }
        let want = if visible && side == self.now_side { 1.0 } else { 0.0 };
        let mut next = shown + (want - shown).signum() * 0.12;
        if (next - want).abs() < 0.12 {
            next = want;
        }
        if next != shown {
            self.force_draw = true;
        }
        self.now_shown = (next, want);
    }

    /// Whether a list's highlight is on its way to its row.
    fn gliding(&self) -> bool {
        self.glide.as_ref().is_some_and(|g| (g.y - g.target).abs() > 0.3 || g.scroll.is_some())
            && screens::list_place(&match Self::screen(&self.machine, &self.machine.mode) {
                Some(screen) => screen,
                None => return false,
            }).is_some()
    }

    /// The thermostat's dial pulses from the temperature now to its target,
    /// soon after it opens and then every PULSE_EVERY, while it heats or
    /// cools towards it.
    fn schedule_pulse(&mut self, before: &Mode, now: Instant) {
        let reaching = match &self.machine.mode {
            Mode::Dial { of, target } => self.machine.climate(*of).is_some_and(|card| {
                card.mode != "off" && matches!(card.action.as_str(), "heating" | "cooling")
                    && card.current.is_some_and(|c| (c - target).abs() >= card.step)
            }),
            _ => false,
        };
        if !reaching {
            self.pulse = None;
            return;
        }
        if !matches!(before, Mode::Dial { .. }) {
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
            Mode::Chooser(_) => zoom::zoomed(),
            Mode::Home if self.machine.owns() => zoom::zoomed(),
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
            let start = if self.transition != Transition::Cut { target } else { target.round() };
            self.view = Some(start);
            if self.transition != Transition::Cut {
                let to = self.card_frame(start.round() as i64);
                self.transit(&to, self.transition);
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
        let (from, to) = (self.key(left), self.key(right));
        let (cards, home) = (&self.cards, &self.home);
        let get = |key: usize| match key {
            0 => home.as_deref(),
            key => cards.get(&key).map(Vec::as_slice),
        };
        if let (Some(from), Some(to)) = (get(from), get(to)) {
            self.display.slide(from, to, (part * paint::SIZE as f32) as usize, true);
        }
    }

    fn show_zoomed(&mut self, at: f32) {
        // From the shrunk cards until nearly full size; see zoom::compose.
        let shrunk = self.zoom < 0.95;
        let z = if (self.zoom - zoom::zoomed()).abs() < 0.003 { zoom::zoomed() } else { self.zoom };
        let count = self.machine.deck().len() as i64;
        // Only the cards that show: the rest are drawn while nothing moves.
        let reach = zoom::reach(z);
        for k in ((at - reach).ceil() as i64).max(0)..=((at + reach).floor() as i64).min(count - 1) {
            self.prepare(k);
            let index = self.key(k);
            if shrunk && !self.shrunk.contains_key(&index) {
                let full = if index == 0 { self.home.clone() } else { self.cards.get(&index).cloned() };
                if let Some(full) = full {
                    self.shrunk.insert(index, prof::time(prof::Part::Shrink, || zoom::shrink(&full)));
                }
            }
        }
        let keys: Vec<usize> = (0..count).map(|k| self.key(k)).collect();
        let (cards, shrunk_cards, home) = (&self.cards, &self.shrunk, &self.home);
        let get = |k: i64| -> Option<&[u8]> {
            if !(0..count).contains(&k) {
                return None; // the cards stop at either end
            }
            let index = keys[k as usize];
            if shrunk {
                shrunk_cards.get(&index).map(Vec::as_slice)
            } else if index == 0 {
                home.as_deref()
            } else {
                cards.get(&index).map(Vec::as_slice)
            }
        };
        let scale = if shrunk { zoom::zoomed() } else { 1.0 };
        prof::time(prof::Part::Compose, || zoom::compose(&mut self.scratch, at, z, scale, get));
        let accent = self.accent(at.round().clamp(0.0, count as f32 - 1.0) as usize);
        prof::time(prof::Part::Overlay, || zoom::focus(&mut self.scratch, at, z, accent));
        // The chosen card's name above and the dots under, as they zoom
        // out; the name fades out and in between cards.
        let fade = ((1.0 - self.zoom) / (1.0 - zoom::zoomed())).clamp(0.0, 1.0);
        let nearest = at.round().clamp(0.0, count as f32 - 1.0) as usize;
        let title = fade * (1.0 - 2.0 * (at - at.round()).abs()).clamp(0.0, 1.0);
        let name = self.name(nearest);
        prof::time(prof::Part::Overlay, || {
            paint::around_cards(&mut self.scratch, &self.fonts, &name, title, count as usize, nearest, fade);
            if self.machine.hint {
                paint::hint(&mut self.scratch, &self.fonts, "Turn to browse · press to open");
            }
        });
        self.display.show(&self.scratch);
    }

    /// The colour the card at `index` in the deck is on in, for its rim in
    /// the chooser: a room's lights' colour, a thermostat heating or
    /// cooling; None when it isn't on.
    fn accent(&self, index: usize) -> Option<[u8; 3]> {
        let climate = |card: &model::ClimateCard| {
            let (colour, active) = screens::climate_colours(card);
            (active && colour != paint::BLACK).then_some(colour)
        };
        match self.machine.deck().get(index)? {
            Slot::Thermostat => climate(&self.machine.nest.as_ref()?.card),
            Slot::Settings => None,
            &Slot::Card(slot) => match self.machine.cards.cards.get(slot)? {
                Card::Lights(card) => screens::room(&card.lights).map(|(_, colour)| colour),
                Card::Climate(card) => climate(card),
                Card::Media(card) if card.state == "playing" => Some(paint::MEDIA),
                _ => None,
            },
        }
    }

    /// What the card at `index` in the deck is called, above it.
    fn name(&self, index: usize) -> String {
        match self.machine.deck().get(index) {
            // Its name from Home Assistant, as its own screens show it.
            Some(Slot::Thermostat) => self.machine.cards.thermostat_name().unwrap_or("Thermostat").to_owned(),
            Some(Slot::Settings) => "Settings".into(),
            Some(&Slot::Card(slot)) => match self.machine.cards.cards.get(slot) {
                Some(Card::Lights(card)) => card.name.clone(),
                Some(Card::Climate(card)) => card.name.clone(),
                Some(Card::Media(card)) => card.name.clone(),
                Some(Card::Action(card)) => card.name.clone(),
                Some(Card::Camera(card)) => card.name.clone(),
                Some(Card::Options(card)) => card.name.clone(),
                Some(Card::Cover(card)) => card.name.clone(),
                Some(Card::Alarm(card)) => card.name.clone(),
                Some(Card::Unknown) | None => String::new(),
            },
            None => String::new(),
        }
    }

    /// What the card with frame key `index` would be drawn from now; None
    /// for no card there.
    fn source(&self, index: usize) -> Option<Drawn> {
        let count = self.machine.cards.cards.len();
        if index == 0 {
            return None;
        }
        if index > count {
            return Some(Drawn { card: None, done: false, progress: None });
        }
        let slot = index - 1;
        let card = self.machine.cards.cards.get(slot).filter(|card| !matches!(card, Card::Unknown))?;
        let progress = match card {
            Card::Media(card) => card.progress(unix_now()).map(|done| (done * screens::PROGRESS_WIDTH) as u32),
            _ => None,
        };
        Some(Drawn { card: Some(card.clone()), done: self.machine.ran.is_some_and(|(ran, _)| ran == slot), progress })
    }

    /// Checks each card's frame against what it would be drawn from now: a
    /// card that shows something else is out of date (stale), and a frame
    /// of a card no longer there is forgotten. Returns whether any changed.
    fn revalidate(&mut self) -> bool {
        let mut changed = false;
        let indices: Vec<usize> = self.cards.keys().copied().collect();
        for index in indices {
            let source = self.source(index);
            match (self.drawn.get(&index), &source) {
                (Some(drawn), Some(source)) if drawn == source => {}
                (Some(drawn), Some(source)) if drawn.same_card(source) => changed |= self.stale.insert(index),
                _ => {
                    self.forget(index);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Forgets the frames of the card with key `index`.
    fn forget(&mut self, index: usize) {
        self.cards.remove(&index);
        self.shrunk.remove(&index);
        self.drawn.remove(&index);
        self.stale.remove(&index);
    }

    /// Forgets every card's frames.
    fn forget_all(&mut self) {
        self.cards.clear();
        self.shrunk.clear();
        self.drawn.clear();
        self.stale.clear();
    }

    /// Card `at`'s key (wrapping round the deck) in the frames drawn: the
    /// thermostat 0, a card its slot + 1, settings after the cards. They
    /// keep their keys wherever the thermostat goes.
    fn key(&self, at: i64) -> usize {
        let deck = self.machine.deck();
        match deck[at.rem_euclid(deck.len() as i64) as usize] {
            Slot::Thermostat => 0,
            Slot::Card(slot) => slot + 1,
            // Past the cards, with the thermostat's card or without.
            Slot::Settings => self.machine.cards.cards.len() + 1,
        }
    }

    /// Draws card `at` (wrapping round the deck) if it isn't yet.
    fn prepare(&mut self, at: i64) {
        self.prepare_key(self.key(at));
    }

    /// Draws the card with frame key `index` if it isn't yet, or if it is
    /// out of date and the cards are at rest: while they move, the frame it
    /// has shows.
    fn prepare_key(&mut self, index: usize) {
        if index == 0 || self.cards.contains_key(&index) && !(self.stale.contains(&index) && !self.moving()) {
            return;
        }
        let Some(source) = self.source(index) else { return };
        let screen = match &source.card {
            Some(Card::Lights(card)) => Screen::LightsCard { card },
            Some(Card::Climate(card)) => Screen::NestCard { nest: Cow::Owned(Nest::of_card(card)), now: unix_now() },
            Some(Card::Media(card)) => {
                let progress = source.progress.map(|pixels| pixels as f32 / screens::PROGRESS_WIDTH);
                Screen::MediaCard { card, volume: None, progress }
            }
            Some(Card::Action(card)) => Screen::ActionCard { card, done: source.done, opened: false },
            Some(Card::Camera(card)) => Screen::CameraCard { card },
            Some(Card::Options(card)) => Screen::OptionsCard { card },
            Some(Card::Cover(card)) => Screen::CoverCard { card },
            Some(Card::Alarm(card)) => Screen::AlarmCard { card },
            Some(Card::Unknown) => return,
            None => Screen::SettingsCard,
        };
        let mut frame = prof::time(prof::Part::Draw, || device::frame(screens::draw(&self.fonts, &screen, None)));
        prof::time(prof::Part::Round, || zoom::round(&mut frame));
        self.cards.insert(index, frame);
        self.shrunk.remove(&index);
        self.stale.remove(&index);
        self.drawn.insert(index, source);
    }

    /// Card `at` as a frame (nlclient's for the thermostat).
    fn card_frame(&mut self, at: i64) -> device::Frame {
        self.prepare(at);
        let index = self.key(at);
        let frame = if index == 0 { self.home.clone() } else { self.cards.get(&index).cloned() };
        frame.unwrap_or_else(|| vec![0; (paint::SIZE * paint::SIZE * 4) as usize])
    }

    /// How far the ring of the mode's value should show (0..1), if it has one.
    fn value(&self) -> Option<f32> {
        let card = |slot: usize| self.machine.cards.cards.get(slot);
        let mode = self.shown_mode();
        match &mode {
            Mode::LightDim { on: true, value, .. } | Mode::Volume { value, .. } => Some(*value as f32 / 100.0),
            Mode::LightWarmth { slot, light, value } => match card(*slot) {
                Some(Card::Lights(c)) => {
                    let [low, high] = c.lights.get(*light)?.kelvins.unwrap_or([2000, 6500]);
                    Some((*value as f32 - low as f32) / (high as f32 - low as f32).max(1.0))
                }
                _ => None,
            },
            Mode::LightHue { value, .. } => Some(*value as f32 / 360.0),
            Mode::CoverSet { value, .. } => Some(*value as f32 / 100.0),
            Mode::Dial { of, target } => {
                // The mark follows the hand between steps.
                let card = self.machine.climate(*of)?;
                let at = target + self.machine.dial_offset * card.step;
                Some(((at - card.min) / (card.max - card.min).max(0.5)) as f32)
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
                // nlclient draws its dial, and after its menu's Back it is
                // the menu closing: wait for the dial (as the display lights,
                // or the menu goes), else use the dial as it last showed.
                let mut page = self.display.save();
                let waiting = Instant::now();
                while !dial_page(&page) && (blank(&page) || dial_coming()) && waiting.elapsed() < Duration::from_millis(1200) {
                    std::thread::sleep(Duration::from_millis(40));
                    page = self.display.save();
                }
                self.saved = Some(match &self.dial {
                    Some(last) if !dial_page(&page) => last.clone(),
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
                        (kind, _) if kind != Transition::Cut => self.transit(&saved, kind),
                        (_, Some(_)) => self.settle(),
                        _ => {}
                    }
                    self.display.end(&saved);
                }
                self.saved = None;
                self.home = None;
                self.view = None;
                let native = self.machine.mode == Mode::Native;
                device::own(false);
                if native {
                    // nlclient's frame as it was when we took the display: it
                    // drew since, unseen, and draws again only as something
                    // changes. Woken as Home Assistant wakes it (no input), it
                    // draws what it shows now.
                    device::wake_display();
                }
            }
            Effect::Hold(hold) => {
                self.input.grab(hold);
                hold_state(hold);
            }
            Effect::WakeDisplay => device::wake_display(),
            Effect::Click => {
                // A fast turn steps faster than clicks can sound apart: at
                // most one every CLICK_EVERY, as the Nest's ring clicks.
                let now = Instant::now();
                if self.sender.clicks() && now.duration_since(self.last_click) >= CLICK_EVERY {
                    self.last_click = now;
                    self.beeper.click();
                }
            }
            Effect::ClickLow => {
                if self.sender.clicks() {
                    self.last_click = Instant::now();
                    self.beeper.click_low();
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
            Effect::Theme(fledge) => {
                if let Err(e) = std::fs::write(THEME, if fledge { "fledge" } else { "nest" }) {
                    logf!("ui: {THEME}: {e}");
                }
                paint::set_theme(if fledge { paint::Theme::Fledge } else { paint::Theme::Nest });
                logf!("ui: the {} theme", if fledge { "Fledge" } else { "Nest" });
                // Everything drawn so far is in the other theme.
                self.forget_all();
                self.stills.clear();
                self.live = None;
                self.glide = None;
                if self.machine.own_thermostat() && self.home.is_some() {
                    self.draw_home();
                }
                self.force_draw = true;
            }
            Effect::OpenNestSettings => {
                // Its menu, then on to Settings: each step only once the
                // display shows the one before, so a turn only ever moves
                // the menu, never the temperature. Else the menu stays.
                std::thread::sleep(Duration::from_millis(300));
                self.input.press();
                let waiting = Instant::now();
                while !menu_shows(&mut self.display) {
                    if waiting.elapsed() > Duration::from_millis(1500) {
                        logf!("ui: nlclient's menu didn't show: left as it is");
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(40));
                }
                // Once the menu has finished opening (it takes no turn while
                // it animates), Settings is the next item anticlockwise from
                // Back, where it opens heating or cooling; off, it opens on
                // Mode, two items away. Its items are a ring with no ends
                // (at most eight), so one item at a time anticlockwise, at
                // most a lap, until Settings is chosen (only the chosen
                // item's icon lights, the cog white). As nlclient tells it,
                // its menu is checked before every turn, so a turn never
                // reaches its dial. A patch that doesn't tell opened on Back
                // (checked above by its pixels): one item, as before.
                std::thread::sleep(Duration::from_millis(600));
                let told = nlstate::read().is_some();
                let steps: &[i32] = if told { &[40; MENU_ITEMS] } else { &[40] };
                let mut chosen = settings_chosen(&self.display.save());
                for (n, &step) in steps.iter().enumerate() {
                    if chosen {
                        break;
                    }
                    if told && !menu_shows(&mut self.display) {
                        logf!("ui: nlclient's menu went, {n} items turned: left as it is");
                        return;
                    }
                    self.input.hand_turn(step, 13);
                    std::thread::sleep(Duration::from_millis(450));
                    chosen = settings_chosen(&self.display.save());
                }
                if chosen && (!told || menu_shows(&mut self.display)) {
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
            Effect::Send(command) => {
                // Its card shows it at once, not when Home Assistant reports it.
                self.machine.sent(&command, Instant::now());
                self.sender.send(&command)
            }
        }
    }

    /// Takes the thermostat's state as nestlocal has it now; its card is
    /// drawn again when that changed.
    fn refresh_nest(&mut self) {
        let Some(state) = self.sender.nest() else { return };
        let mut nest = Nest::from_state(&state);
        if let Some(name) = self.machine.cards.thermostat_name() {
            nest.card.name = name.to_owned();
        }
        if self.machine.nest.as_ref() == Some(&nest) {
            return;
        }
        // While it's being set, the dial's own target stands.
        let role = self.machine.nest.as_ref().is_some_and(|n| n.acting != nest.acting);
        self.machine.set_nest(nest);
        if role {
            // Its card comes or goes among the others.
            logf!("ui: {}", if self.machine.thermostat_card() { "the thermostat's card is back" } else { "not a thermostat: no card of its own" });
            self.revalidate();
        }
        if self.machine.own_thermostat() && self.home.is_some() {
            self.draw_home();
        }
    }

    /// The thermostat's own card, as the first of the cards.
    fn draw_home(&mut self) {
        let Some(nest) = &self.machine.nest else { return };
        self.home_from = Some(nest.clone());
        let screen = Screen::NestCard { nest: Cow::Borrowed(nest), now: unix_now() };
        let mut frame = prof::time(prof::Part::Draw, || device::frame(screens::draw(&self.fonts, &screen, None)));
        prof::time(prof::Part::Round, || zoom::round(&mut frame));
        self.shrunk.insert(0, prof::time(prof::Part::Shrink, || zoom::shrink(&frame)));
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
                Card::Options(card) => Screen::OptionList { card: Cow::Borrowed(card), selected: *selected },
                _ => return None,
            },
            Mode::DialList { of, list, selected } => match machine.climate(*of) {
                Some(card) => {
                    let (choices, now) = machine.choices(*of, *list);
                    let options = OptionsCard {
                        id: card.id.clone(),
                        name: format!("{} · {}", card.name, list.title()),
                        icon: String::new(),
                        options: choices.iter().map(|c| screens::choice_label(*list, c)).collect(),
                        current: now.map(|now| screens::choice_label(*list, &now)),
                    };
                    Screen::OptionList { card: Cow::Owned(options), selected: *selected }
                }
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
            // Its track at rest, as its card shows it (the volume on the
            // ring); the volume large only while the ring turns.
            Mode::Volume { slot, value } => match card(*slot)? {
                Card::Media(card) if machine.volume_at.is_some_and(|at| at.elapsed() < machine::VOLUME_SHOWN) => {
                    Screen::Volume { card, value: *value }
                }
                Card::Media(card) => Screen::MediaCard { card, volume: Some(*value), progress: card.progress(unix_now()) },
                _ => return None,
            },
            Mode::Camera { slot } => match card(*slot)? {
                Card::Camera(card) => Screen::CameraView { card },
                _ => return None,
            },
            Mode::ActionView { slot } => match card(*slot)? {
                Card::Action(card) => Screen::ActionCard { card, done: machine.ran.is_some_and(|(s, _)| s == *slot),
                                                           opened: true },
                _ => return None,
            },
            Mode::AlarmView { slot } => match card(*slot)? {
                Card::Alarm(card) => Screen::AlarmView { card, now: unix_now(), note: alarm_note(machine) },
                _ => return None,
            },
            Mode::AlarmPad { slot, arm, at, entered } => match card(*slot)? {
                Card::Alarm(card) => {
                    let now = Instant::now();
                    let locked = machine.alarm_locked.filter(|until| *until > now)
                        .map(|until| until.duration_since(now).as_secs_f64().ceil() as u64);
                    Screen::AlarmPad { card, arm: arm.and_then(|i| card.modes.get(i as usize)), at: *at as usize,
                                       entered: entered.0.len(), waiting: machine.alarm_wait.is_some(),
                                       note: alarm_note(machine), locked, now: unix_now() }
                }
                _ => return None,
            },
            Mode::AlarmModes { slot, selected } => match card(*slot)? {
                Card::Alarm(card) => Screen::AlarmModes { card, selected: *selected },
                _ => return None,
            },
            Mode::AlarmOpen { slot, arm } => match card(*slot)? {
                Card::Alarm(card) => Screen::AlarmOpen { card, arm: card.modes.get(*arm as usize)?, sensors: &machine.alarm_open },
                _ => return None,
            },
            Mode::Alert { .. } => Screen::Alert { alert: machine.alert.as_ref()? },
            Mode::Glance => Screen::Glance { at: screens::GlanceIn { glance: machine.glance()?, cards: &machine.cards.cards,
                                                                     nest: machine.nest.as_ref(), now: unix_now() as i64 } },
            Mode::Dial { of, target } => Screen::NestSet {
                target: machine::on_step(*target, machine.climate(*of)?.step),
                nest: Self::dial_nest(machine, *of)?, pulse: None,
                setting: machine.set_at.is_some_and(|at| at.elapsed() < machine::SET_SHOWN), shown: None,
            },
            Mode::Prompt { of, selected, .. } => Screen::ModePrompt { nest: Self::dial_nest(machine, *of)?, selected: *selected },
            Mode::Settings { selected } => Screen::Settings { rows: machine.settings(), nest: machine.nest.as_ref(),
                                                              nest_dial: machine.nest_dial, selected: *selected },
            Mode::SettingSet { setting, value } => Screen::SettingSet { setting: *setting, nest: machine.nest.as_ref()?,
                                                                        value: *value },
            Mode::ThemePick { fledge } => Screen::ThemePick { fledge: *fledge },
            _ => return None,
        })
    }

    /// A device's options (its ring of actions), round the ring, the one
    /// chosen large in the middle in the device's colour.
    fn draw_menu(&mut self, view: &Mode, actions: &[Action], selected: usize) {
        let Some(screen) = Self::screen(&self.machine, view) else { return };
        let title = match &screen {
            Screen::LightDim { light, .. } => light.name.clone(),
            Screen::Volume { card, .. } | Screen::MediaCard { card, .. } => card.name.clone(),
            Screen::CoverSet { card, .. } => card.name.clone(),
            Screen::NestSet { nest, .. } => nest.card.name.clone(),
            Screen::AlarmView { card, .. } => card.name.clone(),
            Screen::ActionCard { card, .. } => card.name.clone(),
            _ => String::new(),
        };
        // What each list is on now, so the menu says before it opens.
        let now = |action: &Action| {
            let (Action::Choose(list), Mode::Dial { of, .. }) = (action, view) else { return None };
            if let (ClimateChoice::HotWater, Some(minutes)) = (list, self.machine.nest.as_ref()?.hot_water_boost) {
                return Some(format!("{minutes} min left"));
            }
            let now = self.machine.choices(*of, *list).1.filter(|now| !now.is_empty())?;
            Some(screens::choice_label(*list, &now))
        };
        let items: Vec<(String, String, String)> = actions.iter().map(|action| menu_item(*action, now(action))).collect();
        let options: Vec<screens::Opt> = items.iter()
            .map(|(icon, name, under)| screens::Opt { icon, name, under }).collect();
        let canvas = screens::options(&self.fonts, &title, &options, selected, screens::accent(&screen));
        let frame = device::frame(canvas);
        // The device's screen redraws whole when the menu closes.
        self.live = None;
        let kind = std::mem::replace(&mut self.transition, Transition::Cut);
        self.transit(&frame, kind);
        self.display.show(&frame);
    }

    fn draw(&mut self) {
        if let Mode::Menu { view, actions, selected } = self.machine.mode.clone() {
            return self.draw_menu(&view, &actions, selected);
        }
        if self.machine.mode == Mode::Home {
            // Held at rest: nothing, as the display goes out by itself.
            if self.black && self.transition == Transition::Cut && !self.display.lost() {
                return;
            }
            self.black = true;
            let black = vec![0; (paint::SIZE * paint::SIZE * 4) as usize];
            let kind = std::mem::replace(&mut self.transition, Transition::Cut);
            self.transit(&black, kind);
            self.live = None;
            self.glide = None;
            self.display.show(&black);
            return;
        }
        let mode = self.shown_mode();
        let Some(mut screen) = Self::screen(&self.machine, &mode) else { return };
        if let Screen::NestSet { pulse, shown, .. } = &mut screen {
            *pulse = self.pulse.map(|at| at.elapsed().as_secs_f32() / PULSE.as_secs_f32());
            let now = self.machine.nest.as_ref().and_then(|n| n.card.current).unwrap_or_default();
            *shown = Some((self.now_shown.0, now - self.now_side));
        }
        let sweep = self.sweep.as_ref().map(|(_, shown)| *shown);
        let kind = match (std::mem::replace(&mut self.transition, Transition::Cut), std::mem::take(&mut self.fade_next)) {
            (Transition::Cut, true) => Transition::Fade,
            (kind, _) => kind,
        };
        let fade = kind != Transition::Cut;
        if let Screen::Glance { at } = &screen {
            if let Some(round) = screens::glance_scroll(&self.fonts, at) {
                // A line too long for its place scrolls: the glance drawn
                // once, then only the line, where it is.
                let key = screens::glance_key(at);
                let fresh = !matches!(&self.ticker, Some(ticker) if ticker.key == key);
                if fresh {
                    let still = screens::glance_still(&self.fonts, at);
                    // The same words go on from where they were, as the minute turns.
                    let since = match &self.ticker {
                        Some(ticker) if ticker.round == round => ticker.since,
                        _ => Instant::now(),
                    };
                    self.ticker = Some(Ticker { key, canvas: still.clone(), still, round, since, shown: -1.0 });
                }
                let ticker = self.ticker.as_mut().expect("just made");
                ticker.shown = ticker_offset(ticker.since.elapsed().as_secs_f32(), round);
                let columns = screens::glance_words(&mut ticker.canvas, &ticker.still, &self.fonts, at, ticker.shown);
                (self.live, self.glide) = (None, None);
                if fresh || fade {
                    let frame = ticker.canvas.pixmap.data().to_vec();
                    self.transit(&frame, kind);
                    self.display.show(&frame);
                } else if let Some((first, last)) = columns {
                    // An upright column is a framebuffer row.
                    let ticker = self.ticker.as_ref().expect("just made");
                    self.display.show_rows(ticker.canvas.pixmap.data(), first as usize, last as usize);
                }
                return;
            }
        }
        self.ticker = None;
        if let Some((selected, first)) = screens::list_place(&screen) {
            let frame = Self::draw_list(&mut self.glide, &self.fonts, &screen, selected - first, fade);
            self.transit(&frame, kind);
            self.live = None;
            self.display.show(&frame);
            return;
        }
        self.glide = None;
        // A value screen draws its still part once, then only what changes.
        if let Some(key) = screens::still_key(&screen) {
            let fresh = !matches!(&self.live, Some((kept, ..)) if *kept == key);
            // The thermostat's dial turning orange (or blue, or black) as the
            // target passes the temperature now: faded over the next frames,
            // as the Nest's, the ring still following the hand.
            let recolour = matches!(&self.live, Some((kept, ..))
                if kept.starts_with("nest ") && key.starts_with("nest ") && kept.split(' ').nth(1) != key.split(' ').nth(1));
            if recolour {
                // From the frame last shown, as we drew it: reading the
                // display back is slow (its memory isn't cached).
                let from = match (&self.recolour, &self.live) {
                    (Some((from, _)), _) => from.clone(),
                    (None, Some((_, canvas, _))) => canvas.pixmap.data().to_vec(),
                    (None, None) => self.display.current(),
                };
                self.recolour = Some((from, Instant::now()));
            }
            if fresh {
                if !key.starts_with("nest ") {
                    self.stills.clear();
                }
                let still = match self.stills.get(&key) {
                    Some(still) => still.clone(),
                    None => {
                        let still = screens::still(&self.fonts, &screen);
                        self.stills.insert(key.clone(), still.clone());
                        still
                    }
                };
                let live = paint::Live::new(&still);
                self.live = Some((key, still, live));
                if self.stills.len() > 8 {
                    let kept = self.live.as_ref().map(|(k, ..)| k.clone()).unwrap_or_default();
                    self.stills.retain(|k, _| *k == kept);
                }
            }
            let (_, canvas, live) = self.live.as_mut().expect("just made");
            live.columns = None;
            screens::moving(canvas, &self.fonts, &screen, sweep, Some(live));
            if let Some((from, at)) = &self.recolour {
                let t = at.elapsed().as_secs_f32() / RECOLOUR.as_secs_f32();
                if t >= 1.0 {
                    self.recolour = None;
                    let frame = canvas.pixmap.data().to_vec();
                    self.display.show(&frame);
                } else {
                    self.display.fade(from, canvas.pixmap.data(), (motion::curve(t) * 256.0) as u16);
                }
                return;
            }
            if fresh || fade {
                let frame = canvas.pixmap.data().to_vec();
                self.transit(&frame, kind);
                self.display.show(&frame);
            } else if let Some((first, last)) = live.columns {
                // An upright column is a framebuffer row.
                self.display.show_rows(canvas.pixmap.data(), first as usize, last as usize);
            }
            return;
        }
        self.live = None;
        let frame = device::frame(screens::draw(&self.fonts, &screen, sweep));
        self.transit(&frame, kind);
        self.display.show(&frame);
    }

    /// A thermostat's dial as the shared card draws it: this one's own, or a
    /// climate card's as one.
    fn dial_nest(machine: &Machine, of: Dial) -> Option<Cow<'_, Nest>> {
        match of {
            Dial::Nest => machine.nest.as_ref().map(Cow::Borrowed),
            Dial::Card(_) => machine.climate(of).map(|card| Cow::Owned(Nest::of_card(card))),
        }
    }

    /// The thermostat's dial in its other colour, with the target on the
    /// other side of the temperature now: its still part and what it's kept
    /// by, unless it's kept already.
    fn other_still(fonts: &paint::Fonts, screen: &Screen, stills: &HashMap<String, paint::Canvas>) -> Option<(String, paint::Canvas)> {
        let Screen::NestSet { nest, target, setting, .. } = screen else { return None };
        let card = &nest.card;
        let now = card.current?;
        let other = if *target > now { now } else { (now + card.step).min(card.max) };
        let other = Screen::NestSet { nest: Cow::Borrowed(nest.as_ref()), target: other, pulse: None, setting: *setting, shown: None };
        let key = screens::still_key(&other)?;
        if stills.contains_key(&key) {
            return None;
        }
        Some((key, screens::still(fonts, &other)))
    }

    /// A list, its highlight on its way to row `slot` of those showing:
    /// gliding there from where it was, or there at once on a new list.
    fn draw_list(glide: &mut Option<Glide>, fonts: &paint::Fonts, screen: &Screen, slot: usize, fresh: bool) -> device::Frame {
        let key = screens::list_key(screen).unwrap_or_default();
        let target = screens::LIST_TOP + slot as f32 * screens::ROW_PITCH;
        // The same list (its kind and its first row), only what it shows changed: kept where it is.
        let same = |g: &Glide| g.key.split(' ').take(2).eq(key.split(' ').take(2));
        let layers = |kept: &mut Vec<(String, paint::Canvas, paint::Canvas)>| match kept.iter().position(|(k, ..)| *k == key) {
            Some(at) => {
                let (_, plain, chosen) = kept.swap_remove(at);
                (plain, chosen)
            }
            None => (screens::list_layer(fonts, screen, screens::Layer::Plain).expect("a list"),
                     screens::list_layer(fonts, screen, screens::Layer::Chosen).expect("a list")),
        };
        match glide {
            Some(g) if g.key == key => {}
            Some(g) if same(g) && !fresh => {
                let (plain, chosen) = layers(&mut g.kept);
                let old = (std::mem::replace(&mut g.key, key.clone()), std::mem::replace(&mut g.plain, plain),
                           std::mem::replace(&mut g.chosen, chosen));
                // One row on or back: the rows slide (D1); further, they're there.
                let first = |key: &str| key.split(' ').nth(1).and_then(|f| f.parse::<i64>().ok());
                g.scroll = match (first(&old.0), first(&key)) {
                    (Some(a), Some(b)) if (b - a).abs() == 1 => Some((old.1.clone(), old.2.clone(), (b - a) as f32, Instant::now())),
                    _ => None,
                };
                // Scrolled: where it was is a window it may go back to.
                g.kept.push(old);
                if g.kept.len() > KEPT_WINDOWS {
                    g.kept.remove(0);
                }
            }
            _ => {
                let mut kept = glide.take().map(|g| g.kept).unwrap_or_default();
                let (plain, chosen) = layers(&mut kept);
                *glide = Some(Glide { plain, chosen, kept, key, y: target, target, drawn: Instant::now(), scroll: None });
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
        let t = g.scroll.as_ref().map_or(1.0, |(.., since)| since.elapsed().as_secs_f32() / motion::STANDARD.as_secs_f32());
        match &g.scroll {
            Some((plain, chosen, way, _)) if t < 1.0 => {
                // The rows between the title and the list's foot slide a row.
                let offset = (1.0 - motion::curve(t)) * screens::ROW_PITCH * way;
                let (top, bottom) = screens::list_band();
                paint::slide_band(&mut canvas, plain, &g.plain, top, bottom, offset);
                let mut lit = g.chosen.clone();
                paint::slide_band(&mut lit, chosen, &g.chosen, top, bottom, offset);
                paint::pill_window(&mut canvas, &lit, x, g.y, w, h);
            }
            _ => {
                g.scroll = None;
                paint::pill_window(&mut canvas, &g.chosen, x, g.y, w, h);
            }
        }
        device::frame(canvas)
    }

    /// Moves the display from what shows to `to` by `kind`, a frame at a
    /// time as the display takes them (blocking: a transition is short).
    fn transit(&mut self, to: &[u8], kind: Transition) {
        if kind == Transition::Cut {
            return;
        }
        let from = self.display.current();
        let mut out = vec![0; from.len()];
        let (started, length) = (Instant::now(), kind.length().as_secs_f32());
        loop {
            let t = started.elapsed().as_secs_f32() / length;
            if t >= 1.0 {
                return;
            }
            motion::frame(kind, &from, to, t, &mut out);
            self.display.show(&out);
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
        Mode::Dial { of: Dial::Card(slot), .. } => Some((2, slot, 0)),
        Mode::Dial { of: Dial::Nest, .. } => Some((6, 0, 0)),
        Mode::SettingSet { setting, .. } => Some((7, setting as usize, 0)),
        _ => None,
    }
}

/// The number a value screen shows, as its mode holds it (a light off: none).
fn number(mode: &Mode) -> Option<f64> {
    Some(match *mode {
        Mode::LightDim { on, value, .. } => if on { value as f64 } else { 0.0 },
        Mode::Volume { value, .. } | Mode::CoverSet { value, .. } => value as f64,
        _ => return None,
    })
}

/// `mode` showing `at` as its number: a light on, for as long as it shows one.
fn with_number(mode: Mode, at: f64) -> Mode {
    let percent = at.round().clamp(0.0, 100.0) as u8;
    match mode {
        Mode::LightDim { slot, light, .. } => Mode::LightDim { slot, light, on: true, value: percent },
        Mode::Volume { slot, .. } => Mode::Volume { slot, value: percent },
        Mode::CoverSet { slot, .. } => Mode::CoverSet { slot, value: percent },
        other => other,
    }
}

/// How the display moves from one screen to the next (motion::Kind).
/// Turning between the cards isn't one: the cards follow the ring
/// (Runtime::follow).
type Transition = motion::Kind;

/// How deep a screen is: the cards 0, what a card opens 1, what that opens
/// 2, and so on; a menu one deeper than its screen.
fn depth(mode: &Mode, machine: &Machine) -> u8 {
    match mode {
        Mode::LightList { .. } | Mode::Dial { .. } | Mode::Volume { .. } | Mode::Camera { .. }
        | Mode::OptionList { .. } | Mode::CoverSet { .. } | Mode::Settings { .. }
        | Mode::AlarmView { .. } | Mode::ActionView { .. } => 1,
        // A lamp's own card opens straight to its dimmer; a room's, from its list.
        Mode::LightDim { slot, .. } => match machine.cards.cards.get(*slot) {
            Some(Card::Lights(card)) if card.lights.len() > 1 => 2,
            _ => 1,
        },
        Mode::Members { .. } | Mode::DialList { .. } | Mode::SettingSet { .. } | Mode::ThemePick { .. } | Mode::Prompt { .. }
        | Mode::AlarmPad { .. } | Mode::AlarmOpen { .. } | Mode::AlarmModes { .. } => 2,
        Mode::LightWarmth { slot, light, .. } | Mode::LightHue { slot, light, .. } => {
            depth(&Mode::LightDim { slot: *slot, light: *light, on: true, value: 0 }, machine) + 1
        }
        Mode::Menu { view, .. } => depth(view, machine) + 1,
        Mode::Alert { back, .. } => depth(back, machine),
        _ => 0,
    }
}

/// How the display goes from `before` to `after` (`turned`: by the ring).
/// The glance gives way by fading through; a notification drops in and
/// lifts away; a menu arrives over its screen and leaves it; deeper in a
/// card pushes, back pops; onto the cards and off them, and the screen
/// going out, fade (the cards zoom as well). Handing nlclient its menus,
/// and anything within one view, cuts.
fn transition(before: &Mode, after: &Mode, turned: bool, machine: &Machine) -> Transition {
    let cards = |mode: &Mode| matches!(mode, Mode::Home | Mode::Chooser(_));
    let same_view = std::mem::discriminant(before) == std::mem::discriminant(after) && !cards(before);
    if before == after || same_view || !(before.owning() || after.owning()) || *after == Mode::Native {
        return Transition::Cut;
    }
    if cards(before) && cards(after) && turned {
        return Transition::Cut;
    }
    match (before, after) {
        (_, Mode::Alert { .. }) => Transition::Drop,
        (Mode::Alert { .. }, _) => Transition::Lift,
        (Mode::Glance, _) => Transition::Through,
        // A camera opens from its card, zoomed in, as a photo does.
        (Mode::Chooser(_), Mode::Camera { .. }) => Transition::Through,
        (_, Mode::Menu { view, .. }) if **view == *before => Transition::MenuIn,
        (Mode::Menu { view, .. }, _) if **view == *after => Transition::MenuOut,
        _ => {
            let (from, to) = (depth(before, machine), depth(after, machine));
            if from == 0 || to == 0 || from == to {
                Transition::Fade
            } else if to > from {
                Transition::Push
            } else {
                Transition::Pop
            }
        }
    }
}

/// The wall clock, in Unix seconds.
fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}


#[cfg(test)]
mod tests {
    use super::*;

    /// A frame as Display::save gives it (BGRX), from a picture of one.
    fn frame_from_png(bytes: &[u8]) -> Vec<u8> {
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
        let mut rgb = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut rgb).unwrap();
        assert_eq!((info.width, info.height, info.color_type), (480, 480, png::ColorType::Rgb));
        rgb.chunks(3).flat_map(|c| [c[2], c[1], c[0], 0]).collect()
    }

    #[test]
    fn nlclient_s_dial_is_known_while_the_thermostat_is_off() {
        // Ben's unit, 2026-11-10: off, 23.5 °C now, Eco's leaf; no ring of ticks.
        let off = frame_from_png(include_bytes!("../testdata/nest-off-dial.png"));
        assert!(dial(&off) && !settings_tile(&off), "Back and Done find it");
        assert!(!dial(&vec![0; off.len()]), "black is not the dial");
        let mut busy = off.clone();
        for px in busy.chunks_mut(4).step_by(97) {
            px[..3].copy_from_slice(&[255, 255, 255]);
        }
        assert!(!off_dial(&busy), "white elsewhere (a menu's icons): not the off dial");
    }

    #[test]
    fn of_nlclient_s_menu_only_settings_lights_its_cog() {
        // Ben's unit, 2026-11-10, heating: the item chosen lights its icon.
        let settings = frame_from_png(include_bytes!("../testdata/nest-menu-settings.png"));
        assert!(settings_chosen(&settings) && !back_chosen(&settings));
        for (name, other) in [
            ("back", &include_bytes!("../testdata/nest-menu-back.png")[..]),
            ("mode", include_bytes!("../testdata/nest-menu-mode.png")),
            ("eco", include_bytes!("../testdata/nest-menu-eco.png")),
            ("hot water", include_bytes!("../testdata/nest-menu-hot-water.png")),
        ] {
            let other = frame_from_png(other);
            assert!(!settings_chosen(&other), "{name} is not Settings");
            assert_eq!(back_chosen(&other), name == "back", "{name}");
            assert!(!dial(&other), "{name}: a menu, not the dial");
        }
    }

    #[test]
    fn a_card_showing_something_else_is_the_same_card_and_another_in_its_place_is_not() {
        let cards = Cards::parse(br#"{"cards":[
            {"kind":"climate","id":"climate.a","name":"Kitchen","icon":"","mode":"heat","action":"idle","target":19,"current":21,"min":5,"max":35,"step":1},
            {"kind":"climate","id":"climate.a","name":"Kitchen","icon":"","mode":"heat","action":"heating","target":22,"current":21,"min":5,"max":35,"step":1},
            {"kind":"climate","id":"climate.b","name":"Utility","icon":"","mode":"heat","action":"idle","target":19,"current":21,"min":5,"max":35,"step":1}]}"#).unwrap();
        let drawn = |i: usize, done| Drawn { card: Some(cards.cards[i].clone()), done, progress: None };
        let settings = Drawn { card: None, done: false, progress: None };
        assert!(drawn(0, false) == drawn(0, false));
        assert!(drawn(0, false) != drawn(1, false) && drawn(0, false).same_card(&drawn(1, false)), "heating now: out of date");
        assert!(drawn(0, false) != drawn(0, true) && drawn(0, false).same_card(&drawn(0, true)), "a scene that ran");
        assert!(!drawn(0, false).same_card(&drawn(2, false)), "another card there");
        assert!(!drawn(0, false).same_card(&settings) && settings.same_card(&settings));
    }

    #[test]
    fn the_glance_s_line_rests_then_scrolls_and_comes_round() {
        let round = 360.0;
        assert_eq!(ticker_offset(0.0, round), 0.0);
        assert_eq!(ticker_offset(TICKER_REST - 0.1, round), 0.0, "resting at its start");
        assert!((ticker_offset(TICKER_REST + 1.0, round) - TICKER_SPEED).abs() < 0.01);
        let period = TICKER_REST + round / TICKER_SPEED;
        assert!(ticker_offset(period - 0.01, round) > round - 1.0, "nearly round");
        assert_eq!(ticker_offset(period + 0.5, round), 0.0, "and at its start again");
    }

    /// A machine with a room of two lamps (slot 0) and a lamp of its own (slot 1).
    fn machine_with_rooms() -> Machine {
        let mut machine = Machine::new(Instant::now());
        let cards = Cards::parse(br#"{"cards":[
            {"kind":"lights","name":"Bedroom","icon":"","lights":[
                {"id":"light.lamp","name":"Lamp","on":true,"dimmable":true,"brightness":60},
                {"id":"light.desk","name":"Desk","on":false,"dimmable":true}]},
            {"kind":"lights","name":"Reading","icon":"","lights":[
                {"id":"light.reading","name":"Reading","on":true,"dimmable":true,"brightness":40}]}]}"#).unwrap();
        machine.set_cards(cards, &mut vec![]);
        machine
    }

    #[test]
    fn going_in_and_out_fades_and_the_rest_cuts() {
        let m = &machine_with_rooms();
        assert_eq!(transition(&Mode::Home, &Mode::Chooser(1), true, m), Transition::Cut, "the cards follow the ring");
        assert_eq!(transition(&Mode::Chooser(4), &Mode::Home, true, m), Transition::Cut);
        assert_eq!(transition(&Mode::Chooser(2), &Mode::Home, false, m), Transition::Fade, "left alone");
        let list = Mode::LightList { slot: 0, selected: 0 };
        assert_eq!(transition(&Mode::Chooser(1), &list, true, m), Transition::Fade, "onto a card's own screens");
        assert_eq!(transition(&list, &Mode::Chooser(1), false, m), Transition::Fade);
        assert_eq!(transition(&Mode::Chooser(4), &Mode::Native, true, m), Transition::Cut, "nlclient's menu");
        assert_eq!(transition(&list, &Mode::LightList { slot: 0, selected: 1 }, true, m), Transition::Cut);
    }

    #[test]
    fn deeper_pushes_back_pops_and_overlays_arrive_over() {
        let m = &machine_with_rooms();
        let list = Mode::LightList { slot: 0, selected: 0 };
        let dim = Mode::LightDim { slot: 0, light: 0, on: true, value: 60 };
        assert_eq!(transition(&list, &dim, false, m), Transition::Push, "a room's lamp, from its list");
        assert_eq!(transition(&dim, &list, false, m), Transition::Pop);
        let lamp = Mode::LightDim { slot: 1, light: 0, on: true, value: 40 };
        assert_eq!(transition(&Mode::Chooser(2), &lamp, false, m), Transition::Fade, "a lamp's own card opens to it");
        let menu = Mode::Menu { view: Box::new(dim.clone()), actions: vec![Action::Back], selected: 0 };
        assert_eq!(transition(&dim, &menu, false, m), Transition::MenuIn);
        assert_eq!(transition(&menu, &dim, false, m), Transition::MenuOut);
        let warmth = Mode::LightWarmth { slot: 0, light: 0, value: 3000 };
        assert_eq!(transition(&menu, &warmth, false, m), Transition::Fade, "from the menu, beside the dimmer");
        assert_eq!(transition(&warmth, &dim, false, m), Transition::Pop, "back to the dimmer");
        assert_eq!(transition(&menu, &list, false, m), Transition::Pop, "the menu's Back");
        assert_eq!(transition(&Mode::Glance, &Mode::Chooser(0), true, m), Transition::Through, "the glance gives way");
        let alert = Mode::Alert { id: "door".into(), back: Box::new(Mode::Chooser(0)) };
        assert_eq!(transition(&Mode::Chooser(0), &alert, false, m), Transition::Drop);
        assert_eq!(transition(&alert, &Mode::Chooser(0), false, m), Transition::Lift);
        assert_eq!(transition(&Mode::Chooser(0), &Mode::Camera { slot: 0 }, false, m), Transition::Through);
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
/// An option of a device's: its icon, its name, and what a press on it
/// does. `now`: what a list is on now, in words.
fn menu_item(action: Action, now: Option<String>) -> (String, String, String) {
    let (icon, name, under) = match action {
        Action::Back => ("mdi:arrow-left", "Back", "To the cards"),
        Action::Switch(true) => ("mdi:power", "Turn on", "Press to turn it on"),
        Action::Switch(false) => ("mdi:power", "Turn off", "Press to turn it off"),
        Action::Play(true) => ("mdi:play", "Play", "Press to play"),
        Action::Play(false) => ("mdi:pause", "Pause", "Press to pause"),
        Action::Warmth => ("mdi:white-balance-sunny", "Warmth", "Press to set its white"),
        Action::Colour => ("mdi:palette", "Colour", "Press to choose a colour"),
        Action::Next => ("mdi:skip-next", "Next", "Press for the next track"),
        Action::Previous => ("mdi:skip-previous", "Previous", "Press for the track before"),
        Action::Members => ("mdi:format-list-bulleted", "Each light", "Press to switch each one"),
        Action::Open => ("mdi:arrow-up-bold", "Open", "Press to open it fully"),
        Action::Close => ("mdi:arrow-down-bold", "Close", "Press to close it"),
        Action::NestDial => ("mdi:thermometer", "Nest's dial", "Press for the Nest's own screens"),
        Action::Run => ("mdi:play", "Run", "Press to run it"),
        Action::Choose(list) => {
            let under = match now {
                Some(now) => format!("{now} · press to change"),
                None => "Press to choose".to_owned(),
            };
            return (list.icon().to_owned(), list.title().to_owned(), under);
        }
    };
    (icon.to_owned(), name.to_owned(), under.to_owned())
}

/// What Home Assistant last said of an alarm command, while it shows.
fn alarm_note(machine: &Machine) -> Option<&str> {
    machine.alarm_note.as_ref().map(|(note, _)| note.as_str())
}

fn mode_state(mode: &Mode) {
    let name = match mode {
        Mode::Chooser(index) => format!("Chooser {index}"),
        Mode::Settings { selected } => format!("Settings {selected}"),
        Mode::Prompt { of: Dial::Nest, selected, .. } => format!("ModePrompt {selected}"),
        Mode::Menu { view, .. } => format!("Menu {}", format!("{view:?}").split([' ', '{', '(']).next().unwrap_or("")),
        other => format!("{other:?}").split([' ', '{', '(']).next().unwrap_or("").to_owned(),
    };
    let _ = crate::settings_ui::publish("ui.mode", name.as_bytes());
}

/// Whether nlclient shows its home dial: as nlclient tells it, or (a screen
/// patch from before it told) as its pixels look.
fn home_dial(display: &mut device::Display) -> bool {
    match nlstate::read() {
        Some(report) => report.shows() == Shows::Dial,
        None => dial(&display.save()),
    }
}

/// Whether a frame of nlclient's is its home dial, drawn: nlclient says it
/// shows its dial as its menu starts closing, so the frame must look it too.
fn dial_page(page: &[u8]) -> bool {
    dial(page) && nlstate::read().is_none_or(|report| report.shows() == Shows::Dial)
}

/// Whether nlclient is on its way to its dial (as it tells it): its frame
/// may still be the menu closing.
fn dial_coming() -> bool {
    nlstate::read().is_some_and(|report| report.shows() == Shows::Dial)
}

/// Whether nlclient shows its menu: as it tells it, or by its pixels with
/// Back chosen (the only menu the pixels know).
fn menu_shows(display: &mut device::Display) -> bool {
    match nlstate::read() {
        Some(report) => report.shows() == Shows::Menu,
        None => back_chosen(&display.save()),
    }
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
    if off_dial(page) {
        return true;
    }
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

/// Whether a frame is nlclient's dial while the thermostat is off: no ring
/// of ticks (only the temperature now's, grey), its large white OFF in the
/// middle, and nothing else white (its menus have white icons round the
/// edge, its Settings tiles are blue).
fn off_dial(page: &[u8]) -> bool {
    let stride = page.len() / 480;
    let white = |x: usize, y: usize| {
        // Upright (x, y) is framebuffer row x, column 479 - y, as BGRX.
        let at = x * stride + (479 - y) * 4;
        page[at..at + 3].iter().all(|&c| c > 200)
    };
    let in_box = |x: usize, y: usize| (170..=310).contains(&x) && (205..=275).contains(&y);
    let mut middle = 0;
    for x in 170..=310 {
        for y in 205..=275 {
            middle += white(x, y) as u32;
        }
    }
    if !(1200..=4500).contains(&middle) {
        return false;
    }
    let mut elsewhere = 0;
    for x in (10..470).step_by(3) {
        for y in (10..470).step_by(3) {
            let (dx, dy) = (x as i32 - 240, y as i32 - 240);
            if !in_box(x, y) && dx * dx + dy * dy < 230 * 230 {
                elsewhere += white(x, y) as u32;
            }
        }
    }
    elsewhere < 40
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
    fn fledge_s_theme_unless_the_nest_s_was_chosen() {
        assert!(fledge_theme(None), "nothing kept: Fledge's, the default");
        assert!(fledge_theme(Some("fledge")) && fledge_theme(Some("")), "as before: kept while Fledge's");
        assert!(!fledge_theme(Some("nest\n")));
    }

    /// Frames from the middle of each transition, from the screens ui_shots
    /// wrote (run it first, into the same UNFR0ZEN_UI_SHOTS): m-<kind>-<t>.png.
    #[test]
    #[ignore]
    fn ui_motion_shots() {
        let out = std::env::var("UNFR0ZEN_UI_SHOTS").expect("set UNFR0ZEN_UI_SHOTS");
        const S: usize = 480;
        let load = |name: &str| -> Vec<u8> {
            let mut reader = png::Decoder::new(std::fs::File::open(format!("{out}/{name}.png")).unwrap()).read_info().unwrap();
            let mut rgba = vec![0; reader.output_buffer_size()];
            reader.next_frame(&mut rgba).unwrap();
            // Upright to the framebuffer's layout (upright x a row, y run back).
            let mut panel = vec![0; S * S * 4];
            for y in 0..S {
                for x in 0..S {
                    let (a, b) = (4 * (x * S + (S - 1 - y)), 4 * (y * S + x));
                    panel[a..a + 4].copy_from_slice(&rgba[b..b + 4]);
                }
            }
            panel
        };
        let save_panel = |frame: &[u8], name: &str| {
            let mut upright = vec![0; S * S * 4];
            for y in 0..S {
                for x in 0..S {
                    let (a, b) = (4 * (x * S + (S - 1 - y)), 4 * (y * S + x));
                    upright[b..b + 4].copy_from_slice(&frame[a..a + 4]);
                    upright[b + 3] = 255;
                }
            }
            save(&paint::Canvas::from_frame(upright), &format!("{out}/{name}.png"));
        };
        let pairs = [
            (motion::Kind::Through, "b6-glance", "t1-nest-idle"),
            (motion::Kind::Push, "5-list", "6-dim"),
            (motion::Kind::Pop, "6-dim", "5-list"),
            (motion::Kind::MenuIn, "6-dim", "9-menu"),
            (motion::Kind::MenuOut, "9-menu", "6-dim"),
            (motion::Kind::Drop, "1-lights", "a4-alert"),
            (motion::Kind::Lift, "a4-alert", "1-lights"),
        ];
        let mut frame = vec![0; S * S * 4];
        for (kind, from, to) in pairs {
            let (from, to) = (load(from), load(to));
            for t in [0.25, 0.5, 0.75] {
                motion::frame(kind, &from, &to, t, &mut frame);
                save_panel(&frame, &format!("m-{kind:?}-{}", (t * 100.0) as u32).to_lowercase());
            }
        }
        let mut list = load("5-list");
        let (x, w, h) = screens::PILL;
        motion::press(&mut list, motion::Pressed::Row(x, screens::LIST_TOP, w, h));
        save_panel(&list, "m-press-row");
        // A dial pressed (its menu next).
        let mut dial = load("9-ac-away");
        motion::press(&mut dial, motion::Pressed::Screen);
        save_panel(&dial, "m-press-dial");
    }

    #[test]
    #[ignore]
    fn ui_shots() {
        let (Ok(fonts), Ok(out)) = (std::env::var("UNFR0ZEN_UI_FONTS"), std::env::var("UNFR0ZEN_UI_SHOTS")) else {
            panic!("set UNFR0ZEN_UI_FONTS and UNFR0ZEN_UI_SHOTS");
        };
        let fonts = paint::Fonts::load(&fonts).unwrap();
        paint::set_theme(if std::env::var("UNFR0ZEN_UI_THEME").is_ok_and(|t| t == "nest") { paint::Theme::Nest } else { paint::Theme::Fledge });
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
            ("2-climate", Screen::NestCard { nest: Cow::Owned(Nest::of_card(climate)), now: 0.0 }),
            ("3-media", Screen::MediaCard { card: media, volume: None, progress: Some(0.4) }),
            ("4-settings", Screen::SettingsCard),
            ("5-list", Screen::LightList { card: lights, selected: 0 }),
            ("6-dim", Screen::LightDim { light: &lights.lights[0], on: true, value: 60 }),
            ("7-climate-set", Screen::NestSet { nest: Cow::Owned(Nest::of_card(climate)), target: 22.0, pulse: None, setting: true, shown: None }),
            ("8-volume", Screen::Volume { card: media, value: 35 }),
        ];
        // An air conditioner: drying (no temperature), and its modes listed.
        let ac = Cards::parse(br#"{"cards":[{"kind":"climate","id":"climate.ac","name":"Office AC","mode":"dry",
            "action":"drying","current":24.5,"presets":["away","boost"],"modes":["off","cool","heat","dry","fan_only","heat_cool"],
            "fan_modes":["auto","low","high"],"fan_mode":"auto"}]}"#).unwrap();
        let [Card::Climate(ac)] = &ac.cards[..] else { panic!() };
        let modes = model::OptionsCard { id: ac.id.clone(), name: format!("{} · Mode", ac.name), icon: String::new(),
                                         options: ac.modes.iter().map(|m| screens::climate_label(m)).collect(),
                                         current: Some(screens::climate_label(&ac.mode)) };
        // Cooling, in its away preset: the preset under its number.
        let away = Cards::parse(br#"{"cards":[{"kind":"climate","id":"climate.ac","name":"Office AC","mode":"cool",
            "action":"cooling","target":21,"current":24.5,"preset":"away","preset_modes":["none","away","boost"]}]}"#).unwrap();
        let [Card::Climate(away)] = &away.cards[..] else { panic!() };
        let shots = shots.into_iter().chain([
            ("9-ac-away", Screen::NestCard { nest: Cow::Owned(Nest::of_card(away)), now: 0.0 }),
            ("9-ac-away-set", Screen::NestSet { nest: Cow::Owned(Nest::of_card(away)), target: 21.0, pulse: None, setting: true, shown: None }),
            ("9-ac-dry", Screen::NestSet { nest: Cow::Owned(Nest::of_card(ac)), target: 22.0, pulse: None, setting: true, shown: None }),
            ("9-ac-modes", Screen::OptionList { card: Cow::Owned(modes), selected: 3 }),
        ]);
        for (name, screen) in shots {
            save(&screens::draw(&fonts, &screen, None), &format!("{out}/{name}.png"));
        }
        let lamp_options = [screens::Opt { icon: "mdi:arrow-left", name: "Back", under: "To the cards" },
                            screens::Opt { icon: "mdi:power", name: "Turn off", under: "Press to turn it off" }];
        save(&screens::options(&fonts, "Reading Lamp", &lamp_options, 0, lights.lights[0].colour()), &format!("{out}/9-menu.png"));
        let lamp = model::LightsCard { name: "Reading Lamp".into(), icon: String::new(), lights: vec![lights.lights[0].clone()], members: vec![] };
        save(&screens::draw(&fonts, &Screen::LightsCard { card: &lamp }, None), &format!("{out}/1b-lamp.png"));
        let scene = model::ActionCard { id: "scene.goodnight".into(), name: "Goodnight".into(), icon: "mdi:weather-night".into() };
        save(&screens::draw(&fonts, &Screen::ActionCard { card: &scene, done: false, opened: false }, None), &format!("{out}/a1-scene.png"));
        save(&screens::draw(&fonts, &Screen::ActionCard { card: &scene, done: false, opened: true }, None), &format!("{out}/a1-scene-opened.png"));
        save(&screens::draw(&fonts, &Screen::ActionCard { card: &scene, done: true, opened: true }, None), &format!("{out}/a2-scene-done.png"));
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
        save(&screens::draw(&fonts, &Screen::OptionList { card: Cow::Borrowed(&boost), selected: 2 }, None), &format!("{out}/b2-option-list.png"));
        let blind = model::CoverCard { id: "cover.blind".into(), name: "Bedroom blind".into(), icon: String::new(),
            state: "open".into(), position: Some(60), positionable: true };
        save(&screens::draw(&fonts, &Screen::CoverCard { card: &blind }, None), &format!("{out}/b3-cover.png"));
        save(&screens::draw(&fonts, &Screen::CoverSet { card: &blind, value: 60 }, None), &format!("{out}/b4-cover-set.png"));
        let group = model::LightsCard { name: "Bedroom Lights".into(), icon: String::new(), lights: vec![lights.lights[2].clone()],
            members: lights.lights.clone() };
        save(&screens::draw(&fonts, &Screen::Members { card: &group, selected: 1 }, None), &format!("{out}/b5-members.png"));
        let glance = model::Glance { offset: 0, weather: Some(model::Weather { condition: "partlycloudy".into(),
            temperature: Some(8.4), unit: "°C".into() }), ..model::Glance::default() };
        let at = screens::GlanceIn { glance: &glance, cards: &cards.cards, nest: None, now: 1_794_172_380 };
        save(&screens::draw(&fonts, &Screen::Glance { at }, None), &format!("{out}/b6-glance.png"));
        let alarm = |state: &str, code: Option<&str>, delay: Option<model::AlarmDelay>| model::AlarmCard {
            id: "alarm_control_panel.house".into(), name: "House alarm".into(), icon: String::new(), state: state.into(),
            modes: vec![model::AlarmMode { mode: "home".into(), name: String::new() },
                        model::AlarmMode { mode: "away".into(), name: "Going out".into() },
                        model::AlarmMode { mode: "night".into(), name: "Bedtime".into() }],
            code: code.map(str::to_owned), code_length: Some(4), delay, sensors: vec!["Front door".into()], can_force: true, takeover: true };
        let now = 1_794_172_380.0;
        let delay = Some(model::AlarmDelay { ends: now + 14.2, seconds: 30.0 });
        let disarmed = alarm("disarmed", None, None);
        let armed = alarm("armed_away", Some("number"), None);
        let arming = alarm("arming", Some("number"), delay);
        let pending = alarm("pending", Some("number"), delay);
        let pending_plain = alarm("pending", Some("number"), None);
        let triggered = alarm("triggered", Some("number"), None);
        let mut any_length = armed.clone();
        any_length.code_length = None;
        let shots = [
            ("c1-alarm-disarmed", Screen::AlarmCard { card: &disarmed }),
            ("c2-alarm-armed", Screen::AlarmCard { card: &armed }),
            ("c3-alarm-arming", Screen::AlarmView { card: &arming, now, note: None }),
            ("c4-alarm-pad", Screen::AlarmPad { card: &armed, arm: None, at: 6, entered: 2, waiting: false, note: None,
                                                locked: None, now }),
            ("c5-alarm-pad-any-length", Screen::AlarmPad { card: &any_length, arm: None, at: 10, entered: 5, waiting: false,
                                                           note: None, locked: None, now }),
            ("c6-alarm-pad-entry", Screen::AlarmPad { card: &pending, arm: None, at: 2, entered: 0, waiting: false,
                                                      note: Some("Not accepted · 2 tries left"), locked: None, now }),
            ("c7-alarm-pad-entry-plain", Screen::AlarmPad { card: &pending_plain, arm: None, at: 4, entered: 1,
                                                            waiting: false, note: None, locked: None, now }),
            ("c8-alarm-triggered", Screen::AlarmView { card: &triggered, now, note: None }),
            ("c9-alarm-arm-pad", Screen::AlarmPad { card: &disarmed, arm: Some(&disarmed.modes[1]), at: 4, entered: 4,
                                                    waiting: true, note: None, locked: None, now }),
            ("ca-alarm-locked", Screen::AlarmPad { card: &armed, arm: None, at: 4, entered: 0, waiting: false, note: None,
                                                   locked: Some(27), now }),
            ("cb-alarm-open", Screen::AlarmOpen { card: &disarmed, arm: &disarmed.modes[1],
                                                  sensors: &["Back door".to_owned()] }),
        ];
        for (name, screen) in &shots {
            save(&screens::draw(&fonts, screen, None), &format!("{out}/{name}.png"));
        }
        // The alarm's modes to arm to, each chosen in turn.
        for selected in 0..=disarmed.modes.len() {
            save(&screens::draw(&fonts, &Screen::AlarmModes { card: &disarmed, selected }, None),
                 &format!("{out}/m{selected}-alarm-modes.png"));
        }
        // An armed alarm's options (Back, Disarm), and arming's (Back, Cancel).
        save(&screens::draw(&fonts, &Screen::AlarmModes { card: &armed, selected: 1 }, None), &format!("{out}/m-armed-disarm.png"));
        save(&screens::draw(&fonts, &Screen::AlarmModes { card: &arming, selected: 1 }, None), &format!("{out}/m-arming-cancel.png"));
        // Each state of an alarm's card among the cards, and opened.
        let states = ["disarmed", "armed_home", "armed_away", "armed_night", "arming", "pending", "triggered", "unavailable"];
        for (i, state) in states.iter().enumerate() {
            let timed = matches!(*state, "arming" | "pending");
            let card = alarm(state, (*state != "disarmed").then_some("number"), if timed { delay } else { None });
            save(&screens::draw(&fonts, &Screen::AlarmCard { card: &card }, None), &format!("{out}/s{i}-card-{state}.png"));
            save(&screens::draw(&fonts, &Screen::AlarmView { card: &card, now, note: None }, None),
                 &format!("{out}/s{i}-open-{state}.png"));
        }
        let mut tunable = lights.lights[0].clone();
        (tunable.kelvin, tunable.kelvins, tunable.hues, tunable.hue) = (Some(2700), Some([2200, 6500]), true, Some(200));
        save(&screens::draw(&fonts, &Screen::LightWarmth { light: &tunable, value: 2700 }, None), &format!("{out}/9b-warmth.png"));
        save(&screens::draw(&fonts, &Screen::LightHue { light: &tunable, value: 200 }, None), &format!("{out}/9c-colour.png"));
        let mut with_art = media.clone();
        // The size the integration sends album art at.
        let size = 168;
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
            "time_to_target": "2026-11-08T21:35:00Z", "hot_water_boost": "60"}));
        let shots = [
            ("t1-nest-idle", state(serde_json::json!({}))),
            ("t2-nest-heating", heating.clone()),
            ("t3-nest-eco", state(serde_json::json!({"preset": "eco"}))),
            ("t4-nest-off", state(serde_json::json!({"mode": "off", "action": "off"}))),
        ];
        for (name, nest) in &shots {
            save(&screens::draw(&fonts, &Screen::NestCard { nest: Cow::Borrowed(nest), now }, None), &format!("{out}/{name}.png"));
        }
        // The glance's layouts (protocol 11), as Home Assistant sends them.
        let glances = [
            ("g1-clock-12h", r#"{"layout":"clock","clock":{"hours":12,"ampm":true},"lines":[{"kind":"date"},{"kind":"weather"},{"kind":"whats_on"}]}"#),
            ("g2-temp", r#"{"layout":"temp","lines":[{"kind":"heating"},{"kind":"weather"}]}"#),
            ("g3-weather", r#"{"layout":"weather","lines":[{"icon":"mdi:thermometer","text":"High 12° · Low 5°"},{"text":"Rain from 15:00"}]}"#),
            ("g4-analog", r#"{"layout":"analog","lines":[{"kind":"date_short"}]}"#),
            ("g5-number", r#"{"layout":"number","big":{"label":"Electricity","value":"24.3","unit":"p/kWh"},"lines":[{"kind":"time"},{"icon":"mdi:car","text":"Car charging"}]}"#),
            ("g6-agenda", r#"{"layout":"agenda","event":{"title":"Dentist","start":1794180600},"lines":[{"kind":"weather"}]}"#),
            ("g7-analog-night", r#"{"layout":"analog","night":true,"dim":true,"lines":[{"kind":"date"}]}"#),
            ("g8-clock-lines", r#"{"layout":"clock","lines":[{"kind":"inside"},{"kind":"heating"},{"icon":"mdi:calendar","text":"16:30 Dentist"}]}"#),
        ];
        for (name, json) in glances {
            let mut glance: model::Glance = serde_json::from_str(json).unwrap();
            glance.weather = Some(model::Weather { condition: "partlycloudy".into(), temperature: Some(8.4), unit: "°C".into() });
            let at = screens::GlanceIn { glance: &glance, cards: &cards.cards, nest: Some(&heating), now: now as i64 };
            save(&screens::draw(&fonts, &Screen::Glance { at }, None), &format!("{out}/{name}.png"));
        }
        save(&screens::draw(&fonts, &Screen::NestSet { nest: Cow::Borrowed(&heating), target: 22.0, pulse: None, setting: true, shown: None }, None), &format!("{out}/t5-nest-set.png"));
        let nest_options: Vec<(String, String, String)> = [Action::Back, Action::Choose(model::ClimateChoice::Mode), Action::Choose(model::ClimateChoice::Preset),
            Action::Choose(model::ClimateChoice::HotWater), Action::NestDial].into_iter().map(|a| menu_item(a, None)).collect();
        let nest_options: Vec<screens::Opt> = nest_options.iter().map(|(icon, name, under)| screens::Opt { icon, name, under }).collect();
        save(&screens::options(&fonts, "Thermostat", &nest_options, 1, paint::HEAT), &format!("{out}/t6-nest-menu.png"));
        // Off: the ring turned asks for a mode.
        let off = &shots[3].1;
        for selected in [machine::PROMPT_ON, machine::PROMPT_CANCEL] {
            save(&screens::draw(&fonts, &Screen::ModePrompt { nest: Cow::Borrowed(off), selected }, None),
                 &format!("{out}/t11-nest-off-prompt-{selected}.png"));
        }
        // Every device's options, one chosen: o<n>-<device>.png.
        use screens::Opt;
        let back = |under| Opt { icon: "mdi:arrow-left", name: "Back", under };
        let lamp_light = &lights.lights[0];
        // A device, its name, its options, its colour, and the one chosen.
        type Menu<'a> = (&'a str, &'a str, Vec<Opt<'a>>, [u8; 3], usize);
        let menus: Vec<Menu> = vec![
            ("lamp", "Reading Lamp",
             vec![back("To the cards"), Opt { icon: "mdi:power", name: "Turn off", under: "Press to turn it off" },
                  Opt { icon: "mdi:white-balance-sunny", name: "Warmth", under: "Press to set its white" },
                  Opt { icon: "mdi:palette", name: "Colour", under: "Press to choose a colour" }],
             lamp_light.colour(), 1),
            ("group", "Bedroom Lights",
             vec![back("To the cards"), Opt { icon: "mdi:power", name: "Turn off", under: "Press to turn them off" },
                  Opt { icon: "mdi:format-list-bulleted", name: "Each light", under: "Press to switch each one" }],
             lights.lights[2].colour(), 2),
            ("speaker", "Bedroom speaker",
             vec![back("To the cards"), Opt { icon: "mdi:pause", name: "Pause", under: "Press to pause" },
                  Opt { icon: "mdi:skip-previous", name: "Previous", under: "Press for the track before" },
                  Opt { icon: "mdi:skip-next", name: "Next", under: "Press for the next track" }],
             paint::MEDIA, 3),
            ("thermostat", "Thermostat",
             vec![back("To the cards"), Opt { icon: "mdi:leaf", name: "Eco", under: "Press to turn Eco on" },
                  Opt { icon: "mdi:power", name: "Turn off", under: "Press to turn the heating off" },
                  Opt { icon: "mdi:water-boiler", name: "Stop boost", under: "Press to stop the hot water" },
                  Opt { icon: "mdi:thermometer", name: "Nest's dial", under: "Press for the Nest's own screens" }],
             paint::HEAT, 1),
            ("ac", "Living room AC",
             vec![back("To the cards"), Opt { icon: "mdi:thermostat", name: "Mode", under: "Cool · press to change" },
                  Opt { icon: "mdi:fan", name: "Fan", under: "Auto · press to change" },
                  Opt { icon: "mdi:tune-variant", name: "Preset", under: "Away · press to change" },
                  Opt { icon: "mdi:arrow-oscillating", name: "Swing", under: "Swing · press to change" },
                  Opt { icon: "mdi:arrow-left-right", name: "Horizontal swing", under: "Off · press to change" }],
             paint::HEAT, 3),
            ("cover", "Bedroom blind",
             vec![back("To the cards"), Opt { icon: "mdi:arrow-up-bold", name: "Open", under: "Press to open it fully" },
                  Opt { icon: "mdi:arrow-down-bold", name: "Close", under: "Press to close it" }],
             [120, 170, 255], 2),
            ("scene", "Goodnight",
             vec![back("To the cards"), Opt { icon: "mdi:play", name: "Run", under: "Press to run it" }],
             [255, 196, 87], 1),
        ];
        for (i, (device, title, new, accent, selected)) in menus.iter().enumerate() {
            save(&screens::options(&fonts, title, new, *selected, *accent), &format!("{out}/o{i}-{device}.png"));
        }
        // A switch's own screen, on and off.
        let fan = model::Light { id: "switch.fan".into(), name: "Fan".into(), on: true, dimmable: false, ..Default::default() };
        save(&screens::draw(&fonts, &Screen::LightDim { light: &fan, on: true, value: 100 }, None), &format!("{out}/s1-switch-on.png"));
        save(&screens::draw(&fonts, &Screen::LightDim { light: &fan, on: false, value: 100 }, None), &format!("{out}/s2-switch-off.png"));
        let idle = state(serde_json::json!({"update": "v0.5.0"}));
        let mut machine = Machine::new(Instant::now());
        machine.set_nest(idle.clone());
        for (name, selected) in [("t7-settings", 0), ("t8-settings-end", 9)] {
            save(&screens::draw(&fonts, &Screen::Settings { rows: machine.settings(), nest: Some(&idle), nest_dial: false, selected }, None),
                 &format!("{out}/{name}.png"));
        }
        save(&screens::draw(&fonts, &Screen::SettingSet { setting: Setting::EcoLow, nest: &idle, value: 12.0 }, None),
             &format!("{out}/t9-eco-set.png"));
        save(&screens::draw(&fonts, &Screen::ThemePick { fledge: true }, None), &format!("{out}/t10-theme.png"));
        let cold = state(serde_json::json!({"action": "heating", "target": 22.0, "current_temperature": 19.5,
            "time_to_target": "2026-11-08T21:38:00Z"}));
        for (name, phase) in [("p1-pulse-start", Some(0.15)), ("p2-pulse-mid", Some(0.4)), ("p3-pulse-fade", Some(0.85)), ("p0-rest", None)] {
            save(&screens::draw(&fonts, &Screen::NestSet { nest: Cow::Borrowed(&cold), target: 22.0, pulse: phase, setting: false, shown: None }, None),
                 &format!("{out}/{name}.png"));
        }
        let scheduled = state(serde_json::json!({"action": "heating", "target": 22.0, "current_temperature": 21.0,
            "time_to_target": "2026-11-08T21:38:00Z", "next_change": "18:00"}));
        save(&screens::draw(&fonts, &Screen::NestCard { nest: Cow::Borrowed(&scheduled), now }, None), &format!("{out}/n1-card.png"));
        save(&screens::draw(&fonts, &Screen::NestSet { nest: Cow::Borrowed(&scheduled), target: 22.5, pulse: None, setting: true, shown: None }, None),
             &format!("{out}/n2-set-up.png"));
        save(&screens::draw(&fonts, &Screen::NestSet { nest: Cow::Borrowed(&scheduled), target: 20.0, pulse: None, setting: true, shown: None }, None),
             &format!("{out}/n3-set-down.png"));
        save(&screens::draw(&fonts, &Screen::NestSet { nest: Cow::Borrowed(&scheduled), target: 22.0, pulse: None, setting: false, shown: None }, None),
             &format!("{out}/n4-open.png"));
        let level = state(serde_json::json!({"action": "idle", "target": 21.0, "current_temperature": 21.0, "next_change": "22:30"}));
        save(&screens::draw(&fonts, &Screen::NestCard { nest: Cow::Borrowed(&level), now }, None), &format!("{out}/n5-at-target.png"));
        let warm = state(serde_json::json!({"action": "idle", "target": 21.0, "current_temperature": 22.5, "next_change": "22:30"}));
        save(&screens::draw(&fonts, &Screen::NestCard { nest: Cow::Borrowed(&warm), now }, None), &format!("{out}/n6-above-target.png"));
        save(&screens::draw(&fonts, &Screen::NestSet { nest: Cow::Borrowed(&warm), target: 22.5, pulse: None, setting: true, shown: Some((0.5, 22.0)) }, None),
             &format!("{out}/n7-fading.png"));
        let mut zoomed = vec![0u8; (paint::SIZE * paint::SIZE * 4) as usize];
        let frames: Vec<device::Frame> = [Screen::NestCard { nest: Cow::Borrowed(&cold), now }, Screen::SettingsCard,
            Screen::ActionCard { card: &scene, done: false, opened: false }].iter().map(|screen| {
                let mut frame = screens::draw(&fonts, screen, None).pixmap.data().to_vec();
                zoom::round(&mut frame);
                zoom::shrink(&frame)
            }).collect();
        zoom::compose(&mut zoomed, 1.0, zoom::zoomed(), zoom::zoomed(), |k| frames.get(k as usize).map(Vec::as_slice));
        let plain = zoomed.clone();
        zoom::focus(&mut zoomed, 1.0, zoom::zoomed(), None);
        save(&paint::Canvas::from_frame(zoomed), &format!("{out}/z1-zoomed.png"));
        // The rim in a card's colour: given as the framebuffer would (BGR),
        // as this preview is upright RGBA.
        let mut lit = plain;
        let [r, g, b] = paint::HEAT;
        zoom::focus(&mut lit, 1.0, zoom::zoomed(), Some([b, g, r]));
        save(&paint::Canvas::from_frame(lit), &format!("{out}/z2-zoomed-on.png"));
    }
}
