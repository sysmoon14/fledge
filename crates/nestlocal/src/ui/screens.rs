//! The UI's screens, each drawn whole as the Nest draws its own: black, a
//! ring of ticks, large numbers, and colour only for what is active.


use super::icons;
use super::machine::Setting;
use super::model::{self, ActionCard, AlarmCard, AlarmMode, Alert, CameraCard, Card, ClimateCard, ClimateChoice, CoverCard, Glance,
                   Layout, Light, LightsCard, Line, MediaCard, Nest, OptionsCard, Weather};
use super::paint::{self, *};

use std::borrow::Cow;

pub enum Screen<'a> {
    LightsCard { card: &'a LightsCard },
    /// `progress`: how far through its track (0..1), for its bar.
    MediaCard { card: &'a MediaCard, volume: Option<u8>, progress: Option<f32> },
    SettingsCard,
    /// A scene, script or button; `done` just after it ran; `opened`, its
    /// own screen (pressed from the cards), saying how to run it.
    ActionCard { card: &'a ActionCard, done: bool, opened: bool },
    CameraCard { card: &'a CameraCard },
    /// A camera opened: its picture full screen, its name over it.
    CameraView { card: &'a CameraCard },
    /// A notification, over whatever showed.
    Alert { alert: &'a Alert },
    /// A select's card, and its options listed.
    OptionsCard { card: &'a OptionsCard },
    /// (A climate's modes and fan speeds are listed as one, made for it.)
    OptionList { card: Cow<'a, OptionsCard>, selected: usize },
    /// A light group's own lights, listed to switch.
    Members { card: &'a LightsCard, selected: usize },
    /// A cover's card, and its position on the ring.
    CoverCard { card: &'a CoverCard },
    CoverSet { card: &'a CoverCard, value: u8 },
    /// An alarm's card among the others: its state, still.
    AlarmCard { card: &'a AlarmCard },
    /// An alarm opened, at `now` (Unix seconds): its countdown, its going
    /// off; `note`, what Home Assistant last said.
    AlarmView { card: &'a AlarmCard, now: f64, note: Option<&'a str> },
    /// Its code pad: arming to `arm`, else disarming; the marker on `at`,
    /// `entered` digits so far; `waiting` for Home Assistant; `locked`, the
    /// seconds the code is locked out for.
    AlarmPad { card: &'a AlarmCard, arm: Option<&'a AlarmMode>, at: usize, entered: usize, waiting: bool,
               note: Option<&'a str>, locked: Option<u64>, now: f64 },
    /// Its modes to arm to round the ring, Back first, `selected` large in
    /// the middle.
    AlarmModes { card: &'a AlarmCard, selected: usize },
    /// Arming refused with `sensors` open: arm to `arm` anyway?
    AlarmOpen { card: &'a AlarmCard, arm: &'a AlarmMode, sensors: &'a [String] },
    /// Home Assistant's glance, as its layout has it.
    Glance { at: GlanceIn<'a> },
    /// The lights card pressed: its lights and Back, `selected` highlighted.
    LightList { card: &'a LightsCard, selected: usize },
    LightDim { light: &'a Light, on: bool, value: u8 },
    /// A light's white (kelvin) and colour (hue, degrees) dials.
    LightWarmth { light: &'a Light, value: u16 },
    LightHue { light: &'a Light, value: u16 },
    Volume { card: &'a MediaCard, value: u8 },
    /// A thermostat: its card (`now`, Unix seconds, for its time to
    /// temperature), and its dial setting `target`. This one's own, or a
    /// Home Assistant climate's as one (Nest::of_card): every thermostat is
    /// the same card.
    NestCard { nest: Cow<'a, Nest>, now: f64 },
    /// `pulse`: how far a pulse from now to the target has run (0..1).
    /// `setting`: the ring just turned, so it says what it's set to.
    /// `shown`: how far the temperature now's number has faded in (0..1),
    /// as the target leaves it or comes to it; None, as it stands.
    /// With it, a target on the side it shows (it stays where it was as it
    /// fades, rather than jumping across as the target passes).
    NestSet { nest: Cow<'a, Nest>, target: f64, pulse: Option<f32>, setting: bool, shown: Option<(f32, f64)> },
    /// Its Settings, `selected` highlighted; `nest_dial`, Nest's own dial
    /// in place of its card.
    Settings { rows: Vec<Setting>, nest: Option<&'a Nest>, nest_dial: bool, selected: usize },
    /// A setting's temperature, on its own dial.
    SettingSet { setting: Setting, nest: &'a Nest, value: f64 },
    /// Settings' Theme: the Nest's and Fledge's side by side, each a small
    /// dial in its own look, `fledge` chosen.
    ThemePick { fledge: bool },
    /// The thermostat off and its ring turned: the Nest's question, and Heat
    /// (Cool), Off and Cancel listed as Settings are, `selected` from 1.
    ModePrompt { nest: Cow<'a, Nest>, selected: usize },
}

const HALF: f32 = 240.0;
/// Where a dial's number sits, and the words above and under it, as on the
/// Nest's own dial.
const NUMBER_BASELINE: f32 = 299.0;
const ABOVE_BASELINE: f32 = 161.0;
const UNDER_BASELINE: f32 = 343.0;

/// Words above or under a dial's number, in the Nest's capitals.
fn dial_words(pixmap: &mut Canvas, fonts: &Fonts, y: f32, text: &str, c: [u8; 3], icon_name: Option<&str>) {
    if paint::fledge() {
        // Fledge's: what it's doing in a pill above the number, plain words
        // under it.
        if y < NUMBER_BASELINE {
            pill_words(pixmap, fonts, y - 9.0, text, c, icon_name);
        } else {
            paint::text(pixmap, fonts.regular(), 24.0, HALF, y, text, 0.0, [163, 173, 184]);
        }
        return;
    }
    let text = paint::fit(fonts.medium(), 25.0, &text.to_uppercase(), 1.5, 300.0);
    paint::text(pixmap, fonts.medium(), 25.0, HALF, y, &text, 1.5, c);
}

/// Words in a soft pill centred on `y`, tinted by `c`, an icon before them.
fn pill_words(pixmap: &mut Canvas, fonts: &Fonts, y: f32, text: &str, c: [u8; 3], icon_name: Option<&str>) {
    pill_words_at(pixmap, fonts, y, text, c, icon_name, 22.0, 280.0);
}

/// pill_words at `size`, its words at most `room` wide.
#[allow(clippy::too_many_arguments)]
fn pill_words_at(pixmap: &mut Canvas, fonts: &Fonts, y: f32, text: &str, c: [u8; 3], icon_name: Option<&str>, size: f32,
                 room: f32) {
    let text = paint::fit(fonts.medium(), size, text, 0.0, room);
    let words = paint::measure(fonts.medium(), size, &text, 0.0);
    let icon_room = if icon_name.is_some() { size * 1.45 } else { 0.0 };
    let (pad, height) = (size * 0.95, size * 2.1);
    let width = words + icon_room + 2.0 * pad;
    let left = HALF - width / 2.0;
    paint::pill(pixmap, left, y - height / 2.0, width, height, alpha(c, 0.18));
    if let Some(name) = icon_name {
        icon(pixmap, name, "state", left + pad + size * 0.55, y, size * 1.1, c);
    }
    paint::text_at(pixmap, fonts.medium(), size, left + pad + icon_room, y + size * 0.36, &text, 0.0, c, 1.0);
}

/// Draws `screen`; `sweep`, where it has a value on its ring, is how far the
/// ring shows (0..1): the value itself, or on its way there.
pub fn draw(fonts: &Fonts, screen: &Screen, sweep: Option<f32>) -> Canvas {
    match screen {
        Screen::LightsCard { card } => lights_card(fonts, card),
        Screen::MediaCard { card, volume, progress } => media_card(fonts, card, volume.or(card.volume), *progress),
        Screen::SettingsCard => settings_card(fonts),
        Screen::ActionCard { card, done, opened } => action_card(fonts, card, *done, *opened),
        Screen::CameraCard { card } => camera_card(fonts, card),
        Screen::CameraView { card } => camera_view(fonts, card),
        Screen::Alert { alert } => alert_screen(fonts, alert),
        Screen::OptionsCard { card } => options_card(fonts, card),
        Screen::OptionList { card, selected } => option_list(fonts, card, *selected, Layer::Whole),
        Screen::Members { card, selected } => lights_listed(fonts, &card.name, &card.members, *selected, Layer::Whole),
        Screen::CoverCard { card } => cover_card(fonts, card),
        Screen::AlarmCard { card } => alarm_screen(fonts, card, None, None),
        Screen::AlarmView { card, now, note } => alarm_screen(fonts, card, Some(*now), *note),
        Screen::AlarmPad { card, arm, at, entered, waiting, note, locked, now } => {
            alarm_pad(fonts, card, *arm, *at, *entered, *waiting, *note, *locked, *now)
        }
        Screen::AlarmOpen { card, arm, sensors } => alarm_open(fonts, card, arm, sensors),
        Screen::AlarmModes { card, selected } => alarm_modes(fonts, card, *selected),
        Screen::Glance { at } => glance_screen(fonts, at),
        Screen::LightList { card, selected } => light_list(fonts, card, *selected),
        Screen::NestCard { nest, now } => nest_card(fonts, nest, *now),
        Screen::ThemePick { fledge } => theme_pick(fonts, *fledge),
        Screen::Settings { rows, nest, nest_dial, selected } => settings_list(fonts, rows, *nest, *nest_dial, *selected, Layer::Whole),
        Screen::ModePrompt { nest, selected } => mode_prompt(fonts, nest, *selected, Layer::Whole),
        _ => {
            let mut p = still(fonts, screen);
            moving(&mut p, fonts, screen, sweep, None);
            p
        }
    }
}

/// A screen with a value on its ring, without the ring and the value: what
/// stays put while the ring turns, so a frame only draws what moves. Any
/// other screen whole.
pub fn still(fonts: &Fonts, screen: &Screen) -> Canvas {
    match screen {
        Screen::LightDim { light, on, .. } => light_dim_still(fonts, light, *on, true, parked(screen)),
        Screen::LightWarmth { light, .. } => light_dial_still(fonts, light, "Warmth", parked(screen)),
        Screen::LightHue { light, .. } => light_dial_still(fonts, light, "Colour", None),
        Screen::Volume { card, .. } => volume_still(fonts, card, parked(screen)),
        Screen::CoverSet { card, .. } => cover_still(fonts, card, true, parked(screen)),
        Screen::NestSet { nest, target, setting, .. } => {
            nest_still(fonts, &setting_to(nest, *target, *setting), nest.card.target.map(|_| *target), *setting, unix_now(),
                       parked(screen))
        }
        Screen::SettingSet { setting, .. } => setting_still(fonts, *setting),
        _ => draw(fonts, screen, None),
    }
}

/// What still() left out: the ring, `sweep` of the way round, and the value.
/// On a Live screen (`live`), only what changed since its last frame.
pub fn moving(p: &mut Canvas, fonts: &Fonts, screen: &Screen, sweep: Option<f32>, mut live: Option<&mut paint::Live>) {
    match screen {
        Screen::LightDim { light, on, value } => light_dim_moving(p, live.as_deref_mut(), fonts, light, *on, *value, sweep),
        Screen::LightWarmth { light, value } => warmth_moving(p, live.as_deref_mut(), fonts, light, *value, sweep),
        Screen::LightHue { value, .. } => hue_moving(p, live.as_deref_mut(), fonts, *value, sweep),
        Screen::Volume { card, value } => volume_moving(p, live.as_deref_mut(), fonts, card, *value, sweep),
        Screen::CoverSet { card, value } => cover_moving(p, live.as_deref_mut(), fonts, card, *value, sweep),
        Screen::NestSet { nest, target, pulse, shown, setting } => {
            // A mode with no temperature (dry, fan only): nothing to set.
            let target = nest.card.target.map(|_| *target);
            climate_moving(p, live.as_deref_mut(), fonts, &setting_to(nest, target.unwrap_or(0.0), *setting).card, target, sweep, *pulse, *shown)
        }
        Screen::SettingSet { setting, nest, value } => setting_moving(p, live.as_deref_mut(), fonts, *setting, nest, *value, sweep),
        _ => {}
    }
    if let Some(live) = live {
        paint::settle(live);
    }
}

/// What still() draws a screen from, to know when it can be kept.
/// Only what the still part shows: Home Assistant reporting a value back
/// as the ring turns (a light's brightness, a speaker's volume) mustn't
/// draw the whole screen again mid-turn.
pub fn still_key(screen: &Screen) -> Option<String> {
    // Past its ring's limits, it says so under its number.
    still_part_key(screen).map(|key| match parked(screen) {
        Some(edge) => format!("{key} {edge:?}"),
        None => key,
    })
}

fn still_part_key(screen: &Screen) -> Option<String> {
    match screen {
        Screen::LightDim { light, on, .. } => {
            Some(format!("dim {} {} {} {:?} {} {on}", light.id, light.name, light.icon, light.rgb, light.dimmable))
        }
        Screen::LightWarmth { light, .. } => Some(format!("warmth {}", light.id)),
        Screen::LightHue { light, .. } => Some(format!("hue {}", light.id)),
        Screen::Volume { card, .. } => Some(format!("volume {} {}", card.name, card.state)),
        Screen::CoverSet { card, .. } => Some(format!("cover {} {} {} {} {}", card.id, card.name, card.icon, card.state,
                                                     card.positionable)),
        Screen::NestSet { nest, target, setting, .. } => {
            let nest = setting_to(nest, *target, *setting);
            // Turning only changes it in Eco (its leaf gives way).
            let setting = *setting && nest.eco;
            // (The temperature now's number is drawn with the ring, so which
            // side of its tick it's on doesn't change this.)
            Some(format!("nest {} {:?} {} {:?} {:?} {:?} {} {:?} {} {} {setting} {:?}", nest.card.id, nest.card.name,
                nest.card.mode, nest.card.target.is_some(), nest.card.action, nest.card.current, nest.eco,
                nest.card.active_preset(), nest.hot_water_boost.unwrap_or(0),
                (nest.time_to_target.unwrap_or(0.0) - unix_now()).max(0.0) as i64 / 60, nest.next_change))
        }
        Screen::SettingSet { setting, .. } => Some(format!("setting {setting:?}")),
        _ => None,
    }
}

fn label(pixmap: &mut Canvas, fonts: &Fonts, y: f32, text: &str, c: [u8; 3]) {
    if paint::fledge() {
        // Fledge names things as they're written.
        let text = paint::fit(fonts.medium(), 24.0, text, 0.0, 310.0);
        paint::text(pixmap, fonts.medium(), 24.0, HALF, y, &text, 0.0, c);
        return;
    }
    let text = paint::fit(fonts.medium(), 22.0, &text.to_uppercase(), 1.5, 300.0);
    paint::text(pixmap, fonts.medium(), 22.0, HALF, y, &text, 1.5, c);
}

fn line(pixmap: &mut Canvas, fonts: &Fonts, y: f32, text: &str, c: [u8; 3]) {
    let text = paint::fit(fonts.regular(), 24.0, text, 0.0, 330.0);
    paint::text(pixmap, fonts.regular(), 24.0, HALF, y, &text, 0.0, c);
}

/// A value at or past one of its ring's limits.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// Which limit: -1 its lowest, 1 its highest.
    pub side: i8,
    /// That limit in words, its unit after it.
    pub limit: String,
    /// Past it (Home Assistant set it there, or it was pushed through),
    /// not just at it.
    pub past: bool,
    /// Whether that limit gives way to being turned on past.
    pub pushes: bool,
    /// The clicks pushing on it so far.
    pub pressed: u8,
}

pub type Parked = Option<Edge>;

/// The push on a limit under way (Machine::pushing), for the screens drawn
/// next: the arrows past that end light one a click.
static PUSHING: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(0);

pub fn set_pushing(pushing: i8) {
    PUSHING.store(pushing, std::sync::atomic::Ordering::Relaxed);
}

fn edge_of(value: i32, ring: model::Ring, words: impl Fn(i32) -> String) -> Parked {
    let side = ring.edge(value);
    let limit = ring.limit(side)?;
    let past = ring.past(value) != 0;
    let pushing = PUSHING.load(std::sync::atomic::Ordering::Relaxed);
    let pressed = if !past && pushing.signum() == side { pushing.unsigned_abs() } else { 0 };
    Some(Edge { side, limit: words(limit), past, pushes: ring.pushes(side), pressed })
}

/// Whether a value screen's value is at or past one of its ring's limits.
/// At one, it says so. Past one that holds, the ring waits at that end,
/// arrows beyond it pointing on out, its number grey, and under it which
/// limit and which way to turn to it; past one that gives way, the arrows
/// and the number are amber and it says which limit it's over.
pub fn parked(screen: &Screen) -> Parked {
    match screen {
        Screen::LightDim { light, on: true, value } if light.dimmable => {
            edge_of(*value as i32, light.brightness_ring(), |v| format!("{v}%"))
        }
        Screen::LightWarmth { light, value } => edge_of(*value as i32, light.kelvin_ring(), |v| format!("{v}K")),
        Screen::Volume { card, value } => edge_of(*value as i32, card.volume_ring(), |v| v.to_string()),
        Screen::CoverSet { card, value } if card.positionable => {
            edge_of(*value as i32, card.position_ring(), |v| format!("{v}%"))
        }
        Screen::NestSet { nest, target, .. } if nest.card.target.is_some() && nest.card.mode != "off" => {
            edge_of((*target * 10.0).round() as i32, nest.card.ring(), |v| degrees(v as f64 / 10.0))
        }
        _ => None,
    }
}

/// A value over a limit that gives way: its number and words, its arrows.
const OVER: [u8; 3] = [250, 199, 117];
const OVER_ARROW: [u8; 3] = [239, 159, 39];

/// Three small arrows beyond the end of the ring (`side`: -1 its start, 1
/// its end), in the ring's gap, pointing on out: `lit` of them in `tint`,
/// brightest nearest the ring, the rest faint where `faint`.
fn past_arrows(p: &mut Canvas, side: i8, lit: u8, tint: [u8; 3], faint: bool) {
    let way = side as f32;
    let end = way * paint::tick_angle(paint::TICKS - 1, paint::TICKS);
    let radius = if paint::fledge() { 212.0 } else { 206.0 };
    let at = |degrees: f32, r: f32| {
        let a = degrees.to_radians();
        (HALF + r * a.sin(), HALF - r * a.cos())
    };
    for (k, shade) in [0.9, 0.6, 0.35].into_iter().enumerate() {
        let colour = match (k < lit as usize, faint) {
            (true, _) => alpha(tint, shade),
            (false, true) => alpha(GREY, 0.3),
            (false, false) => continue,
        };
        let centre = end + way * (8.0 + 5.0 * k as f32);
        let tip = at(centre + way * 1.6, radius);
        for arm in [-11.0, 11.0] {
            let arm = at(centre - way * 1.6, radius + arm);
            paint::line(p, arm.0, arm.1, tip.0, tip.1, 3.0, colour);
        }
    }
}

/// Where the words under a value at or past a limit go: high enough that
/// the second line clears the long mark waiting at the ring's end.
const PARKED_WORDS: f32 = 331.0;

/// Under the number of a value at or past one of its ring's limits, from
/// `y`, and the arrows past that end. Whether it took both lines: at a
/// limit and not pushing on it, the screen's own hint keeps its place.
fn parked_words(p: &mut Canvas, fonts: &Fonts, edge: &Edge, y: f32, soft: [u8; 3]) -> bool {
    let limit = &edge.limit;
    match (edge.past, edge.pushes) {
        (true, true) => {
            let over = if edge.side > 0 { "Over" } else { "Under" };
            // On a coloured face (heating, cooling), amber wouldn't read.
            let plain = soft == GREY;
            line(p, fonts, y, &format!("{over} your limit of {limit}"), if plain { OVER } else { WHITE });
            past_arrows(p, edge.side, 3, if plain { OVER_ARROW } else { WHITE }, false);
        }
        (true, false) => {
            let (over, way) = if edge.side > 0 { ("Above", "left") } else { ("Below", "right") };
            line(p, fonts, y, &format!("{over} your limit of {limit}"), soft);
            line(p, fonts, y + 28.0, &format!("Turn {way} to set {limit}"), WHITE);
            past_arrows(p, edge.side, 3, WHITE, false);
        }
        (false, pushes) => {
            line(p, fonts, y, "At your limit", soft);
            if pushes {
                past_arrows(p, edge.side, edge.pressed, WHITE, true);
            }
            if edge.pressed == 0 {
                return false;
            }
            line(p, fonts, y + 28.0, &format!("Keep turning to go past {limit}"), WHITE);
        }
    }
    true
}

/// A value's number: white; past a limit that holds, grey; over one that
/// gives way, amber.
fn number_tint(ring: model::Ring, value: i32) -> [u8; 3] {
    match ring.past(value) {
        0 => WHITE,
        over if ring.pushes(over) => OVER,
        _ => GREY,
    }
}

/// Where `value` sits along a ring from `low` to `high` (0..1, clamped
/// when drawn).
fn along(value: f32, [low, high]: [f32; 2]) -> f32 {
    (value - low) / (high - low).max(1e-3)
}

/// A large number as the Nest's dial shows a temperature: 21, or 21 with a
/// small raised 5 for the half.
/// big_number, on a Live screen when there is one.
#[allow(clippy::too_many_arguments)]
fn big_number_on(pixmap: &mut Canvas, live: Option<&mut paint::Live>, fonts: &Fonts, value: f64, half: bool, unit: &str,
                 c: [u8; 3]) {
    // As the Nest's dial: 119 px digits, centred on the display.
    let size = 170.0;
    let whole = if half { value.floor() as i64 } else { value.round() as i64 };
    let digits = whole.to_string();
    // The half: a raised 5 on the Nest's dial, a smaller .5 on Fledge's.
    let half_mark = if paint::fledge() { ".5" } else { "5" };
    let small = if half && value.fract() >= 0.25 { half_mark } else { unit };
    let digits_font = fonts.digits();
    let width = paint::measure(digits_font, size, &digits, -4.0);
    let small_width = if small.is_empty() { 0.0 } else { paint::measure(digits_font, 52.0, small, 0.0) + 6.0 };
    let x = HALF - (width + small_width) / 2.0;
    // Top-aligned with the digits, as the Nest's dial sets its half degree.
    let small_x = x + width + 4.0;
    let mut parts = vec![(digits_font, size, x, NUMBER_BASELINE, digits.as_str(), -4.0)];
    if !small.is_empty() {
        parts.push((digits_font, 52.0, small_x, NUMBER_BASELINE - 82.0, small, 0.0));
    }
    match live {
        Some(live) => paint::number_live(pixmap, live, &parts, c),
        None => {
            for (font, size, x, y, text, spacing) in parts {
                paint::text_at(pixmap, font, size, x, y, text, spacing, c, 1.0);
            }
        }
    }
}

/// paint::ring, on a Live screen when there is one.
fn ring_on(p: &mut Canvas, live: Option<&mut paint::Live>, of: impl Fn(usize) -> tiny_skia::Color, marks: &[usize]) {
    match live {
        Some(live) => paint::ring_live(p, live, of, marks),
        None => paint::ring(p, of, marks),
    }
}

fn icon(pixmap: &mut Canvas, name: &str, entity: &str, cx: f32, cy: f32, size: f32, c: [u8; 3]) {
    if let Some(found) = icons::lookup(name, entity) {
        paint::icon(pixmap, found, cx, cy, size, colour(c));
    }
}

/// What a room's lights come to: the average brightness of those on, as
/// Home Assistant averages a light group (on/off lights count as full), and
/// their colours mixed in the same proportions.
pub fn room(lights: &[Light]) -> Option<(u8, [u8; 3])> {
    let on: Vec<(u32, [u8; 3])> = lights.iter().filter(|l| l.on)
        .map(|l| (l.brightness.unwrap_or(100).max(1) as u32, l.colour())).collect();
    if on.is_empty() {
        return None;
    }
    let weight: u32 = on.iter().map(|(b, _)| b).sum();
    let mix = |channel: usize| (on.iter().map(|(b, c)| b * c[channel] as u32).sum::<u32>() / weight) as u8;
    let average = (weight as f32 / on.len() as f32).round() as u8;
    Some((average, [mix(0), mix(1), mix(2)]))
}

fn lights_card(fonts: &Fonts, card: &LightsCard) -> Canvas {
    if let [light] = &card.lights[..] {
        return light_card(fonts, card, light);
    }
    let mut p = paint::face(BLACK);
    let lights = &card.lights;
    let room = room(lights);
    // One sweep, as a dimmer's: how bright the room is, in its lights' colour.
    let top = room.map(|(level, _)| paint::tick_at(level as f32 / 100.0));
    paint::ring(&mut p, |i| match (room, top) {
        (Some((_, mixed)), Some(top)) if i <= top => colour(mixed),
        _ => colour(DIM),
    }, &[]);
    let accent = room.map_or(GREY, |(_, mixed)| mixed);
    icon(&mut p, &card.icon, "lights", HALF, 132.0, 72.0, accent);
    label(&mut p, fonts, 214.0, &card.name, accent);
    match room {
        None => {
            paint::text(&mut p, fonts.regular(), 96.0, HALF, 320.0, "Off", 0.0, WHITE);
            line(&mut p, fonts, 366.0, &format!("{} lights", lights.len()), GREY);
        }
        Some((level, _)) => {
            let on = lights.iter().filter(|l| l.on).count();
            paint::text(&mut p, fonts.regular(), 112.0, HALF, 322.0, &on.to_string(), 0.0, WHITE);
            line(&mut p, fonts, 366.0, &format!("of {} on · {level}%", lights.len()), GREY);
        }
    }
    p
}

/// A card of one light: as its own dimmer shows it, so opening it changes
/// nothing on the display but what the ring then does.
fn light_card(fonts: &Fonts, card: &LightsCard, light: &Light) -> Canvas {
    let mut light = light.clone();
    if light.icon.is_empty() {
        light.icon = card.icon.clone();
    }
    let value = light.brightness.unwrap_or(100);
    // Among the cards, no hold hint: a hold there puts the screen away.
    let mut p = light_dim_still(fonts, &light, light.on, false, None);
    if light.dimmable {
        light_dim_moving(&mut p, None, fonts, &light, light.on, value, None);
    } else {
        // On or off: the ring full in its colour, or dim.
        paint::ring(&mut p, |_| if light.on { colour(light.colour()) } else { colour(DIM) }, &[]);
    }
    p
}

pub fn climate_colours(card: &ClimateCard) -> ([u8; 3], bool) {
    if card.mode == "off" {
        // Off is off, whatever its action last said (some integrations
        // leave it at heating).
        return (BLACK, false);
    }
    match card.action.as_str() {
        "heating" | "preheating" => (HEAT, true),
        "cooling" => (COOL, true),
        _ => (BLACK, false),
    }
}

/// Where the temperature now's number sits on the ring: its middle, and
/// the ticks under it, as the Nest sets it in the ring beside its tick, on
/// the side away from the target.
fn now_place(card: &ClimateCard, target: Option<f64>) -> Option<((f32, f32), std::ops::RangeInclusive<usize>)> {
    let now = card.current?;
    let tick = paint::tick_at(((now - card.min) / (card.max - card.min).max(0.5)) as f32);
    let away = if target.is_some_and(|t| t < now) { 1.0 } else { -1.0 };
    let angle = (paint::tick_angle(tick, paint::TICKS) + away * 9.0).to_radians();
    let centre = tick as f32 + away * 4.5;
    let (low, high) = ((centre - 5.0).max(0.0).round() as usize, (centre + 5.0).round().min((paint::TICKS - 1) as f32) as usize);
    // In the Nest's ticks; just inside Fledge's arc.
    let radius = if paint::fledge() { 178.0 } else { 212.0 };
    Some(((HALF + radius * angle.sin(), HALF - radius * angle.cos()), low..=high))
}

/// Whether a dial shows the temperature now by its tick: only while the
/// target is another, as the Nest's does.
fn now_differs(card: &ClimateCard, target: Option<f64>) -> bool {
    matches!((card.current, target), (Some(now), Some(t)) if (now - t).abs() >= card.step / 2.0)
}

/// The temperature now, small in the ring over its ticks, beside its tick,
/// `shown` (0..1) of the way faded in. On a Live screen, climate_moving
/// puts back what was under it first, as it's drawn again each frame.
fn now_label(p: &mut Canvas, fonts: &Fonts, card: &ClimateCard, target: Option<f64>, shown: f32) {
    let (Some(now), Some(((x, y), _))) = (card.current, now_place(card, target)) else { return };
    if shown <= 0.01 {
        return;
    }
    let half = card.step < 1.0 && (now.fract() - 0.5).abs() < 0.25;
    let whole = if half { now.floor() } else { now.round() } as i64;
    let digits = whole.to_string();
    let width = paint::measure(fonts.medium(), 34.0, &digits, 0.0);
    let small = if half { paint::measure(fonts.medium(), 20.0, "5", 0.0) + 2.0 } else { 0.0 };
    let left = x - (width + small) / 2.0;
    // White over the ticks, to read on the black dial as on the orange.
    let tint = WHITE;
    paint::text_at(p, fonts.medium(), 34.0, left, y + 12.0, &digits, 0.0, tint, shown);
    if half {
        paint::text_at(p, fonts.medium(), 20.0, left + width + 2.0, y - 3.0, "5", 0.0, tint, shown);
    }
}

/// One of a thermostat's lists' options in words: a hot water boost's
/// minutes as a time ("1 hour"), else as climate_label says.
pub fn choice_label(list: ClimateChoice, option: &str) -> String {
    match (list, option.parse::<u32>()) {
        (ClimateChoice::HotWater, Ok(60)) => "1 hour".to_owned(),
        (ClimateChoice::HotWater, Ok(m)) if m % 60 == 0 => format!("{} hours", m / 60),
        (ClimateChoice::HotWater, Ok(m)) => format!("{m} min"),
        _ => climate_label(option),
    }
}

/// A climate mode or fan speed as Home Assistant names it, in words:
/// heat_cool "Heat/cool", fan_only "Fan only".
pub fn climate_label(name: &str) -> String {
    match name {
        "heat_cool" => "Heat/cool".to_owned(),
        name => capitalise(&name.replace('_', " ")),
    }
}


/// The ring's ticks on the heating orange, as the Nest's (on cooling blue,
/// a lighter blue).
const AMBER: [u8; 3] = [246, 136, 0];

/// A climate dial's plain ticks: amber on heating orange, light blue on
/// cooling blue, dim on black.
fn tick_base(card: &ClimateCard) -> [u8; 3] {
    if paint::fledge() {
        return DIM; // Fledge's track
    }
    match climate_colours(card) {
        (_, false) => DIM,
        _ if card.action == "cooling" => [96, 160, 240],
        _ => AMBER,
    }
}

/// The temperature now's tick while heating or cooling: a warm cream.
const NOW: [u8; 3] = [255, 246, 214];

/// How lit a tick `along` (0 at the temperature now, 1 at the target) is
/// as a pulse `phase` (0..1) runs: filling from now to the target, then
/// fading back.
fn pulse_light(phase: f32, along: f32) -> f32 {
    let filled = phase / 0.6;
    let front = 1.0 - (1.0 - filled.min(1.0)).powi(3);
    let fade = ((phase - 0.6) / 0.4).clamp(0.0, 1.0);
    // The front's edge soft over a few ticks.
    let reached = ((front - along) / 0.12 + 0.5).clamp(0.0, 1.0);
    reached * (1.0 - fade)
}

/// The Nest's dial: its ticks soft, the temperature now a wide cream tick,
/// the target long and white, and while heating or cooling the ticks
/// between bright, `pulse` (0..1, as it runs) filling them from now to the
/// target.
#[allow(clippy::too_many_arguments)]
fn climate_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, card: &ClimateCard,
                  target: Option<f64>, sweep: Option<f32>, pulse: Option<f32>, shown: Option<(f32, f64)>) {
    // The temperature now's number goes over the ticks: first, put back
    // what was under it, so its ticks are drawn again.
    let (shown, place) = match shown {
        Some((shown, place)) => (shown, Some(place)),
        None => (if now_differs(card, target) { 1.0 } else { 0.0 }, target),
    };
    if let Some(live) = live.as_deref_mut() {
        if let Some(now) = card.current {
            for side in [now - 1.0, now + 1.0] {
                if let Some(((cx, cy), under)) = now_place(card, Some(side)) {
                    live.clear(p, (cx as i32 - 34, cy as i32 - 24, 68, 42), under);
                }
            }
        }
    }
    let (_, active) = climate_colours(card);
    let fraction = |t: f64| ((t - card.min) / (card.max - card.min).max(0.5)) as f32;
    // Off: no target to mark, only the temperature now.
    let mark = target.filter(|_| card.mode != "off").map(|t| paint::tick_at(sweep.unwrap_or(fraction(t))));
    let now = card.current.map(|t| paint::tick_at(fraction(t)));
    let range = match (now, mark) {
        (Some(a), Some(b)) if active && a != b => Some((a.min(b), a.max(b), a)),
        _ => None,
    };
    let base = colour(tick_base(card));
    let of = |i: usize| {
        if Some(i) == mark {
            (colour(WHITE), paint::Shape::Long)
        } else if Some(i) == now {
            (colour(if active { NOW } else { GREY }), paint::Shape::Wide)
        } else {
            match range {
                Some((low, high, from)) if (low..=high).contains(&i) => {
                    let along = (i as f32 - from as f32).abs() / (high - low) as f32;
                    let lit = pulse.map_or(0.0, |phase| pulse_light(phase, along));
                    if paint::fledge() {
                        // Fledge's arc glows from the temperature now to the
                        // target, amber (blue cooling), the pulse brightening it.
                        let (from, to) = if card.action == "cooling" {
                            ([60, 140, 240], [150, 205, 255])
                        } else {
                            ([255, 106, 43], [255, 179, 92])
                        };
                        let t = 0.35 * (1.0 - along) + 0.65 * lit;
                        let mix = |k: usize| (from[k] as f32 + (to[k] as f32 - from[k] as f32) * t) as u8;
                        (colour([mix(0), mix(1), mix(2)]), paint::Shape::Short)
                    } else {
                        (alpha(WHITE, 0.62 + 0.38 * lit), paint::Shape::Short)
                    }
                }
                _ => (base, paint::Shape::Short),
            }
        }
    };
    match live.as_deref_mut() {
        Some(live) => paint::ring_shaped_live(p, live, of),
        None => paint::ring_shaped(p, of),
    }
    if card.mode != "off" {
        now_label(p, fonts, card, place, shown);
    } else {
        // Off, as the Nest's: the temperature now stays by its tick.
        now_label(p, fonts, card, None, 1.0);
    }
    if let Some(t) = target.filter(|_| card.mode != "off") {
        // Past its limits: softer, until it's turned back to them.
        let tint = match number_tint(card.ring(), (t * 10.0).round() as i32) {
            WHITE => WHITE,
            _ if active => [255, 225, 205],
            tint => tint,
        };
        big_number_on(p, live, fonts, t, card.step < 1.0, "", tint);
    }
}

/// How wide a media card's progress bar is.
pub const PROGRESS_WIDTH: f32 = 180.0;

fn media_card(fonts: &Fonts, card: &MediaCard, volume: Option<u8>, progress: Option<f32>) -> Canvas {
    let mut p = paint::face(BLACK);
    let playing = card.state == "playing";
    let lit = volume.map(|v| paint::tick_at(v as f32 / 100.0));
    paint::ring(&mut p, |i| match lit {
        Some(top) if i <= top && playing => colour(MEDIA),
        Some(top) if i <= top => alpha(MEDIA, 0.45),
        _ => colour(DIM),
    }, &[]);
    let accent = if playing { MEDIA } else { GREY };
    // With album art, the art in place of the name and icon.
    let top = match &card.art {
        Some(art) if paint::fledge() => {
            // Fledge's: smaller, rounder, inside the arc.
            let side = (art.size as f32).min(176.0);
            paint::picture_scaled(&mut p, &art.rgb, art.size, side as u32, HALF, 186.0, 24.0);
            186.0 + side / 2.0
        }
        Some(art) => {
            paint::picture(&mut p, &art.rgb, art.size, HALF, 194.0, 14.0);
            194.0 + art.size as f32 / 2.0
        }
        None => {
            label(&mut p, fonts, 124.0, &card.name, accent);
            icon(&mut p, &card.icon, &card.id, HALF, 210.0, 92.0, accent);
            270.0
        }
    };
    let title = if card.title.is_empty() { capitalise(&card.state) } else { card.title.clone() };
    let title = paint::fit(fonts.medium(), 28.0, &title, 0.0, 300.0);
    paint::text(&mut p, fonts.medium(), 28.0, HALF, top + 42.0, &title, 0.0, WHITE);
    // How far through the track, under its title.
    let mut below = top + 76.0;
    if let Some(done) = progress {
        let (x, y) = (HALF - PROGRESS_WIDTH / 2.0, top + 57.0);
        paint::pill(&mut p, x, y, PROGRESS_WIDTH, 8.0, colour(DIM));
        let width = (PROGRESS_WIDTH * done).max(8.0);
        paint::pill(&mut p, x, y, width, 8.0, colour(if playing { MEDIA } else { GREY }));
        below += 14.0;
    }
    let under = if card.artist.is_empty() { String::new() } else { card.artist.clone() };
    line(&mut p, fonts, below, &under, GREY);
    p
}

/// Scenes' and scripts' accent: a warm gold.
const ACTION: [u8; 3] = [255, 196, 87];

/// No ring: there's nothing to turn. Done, its icon and words turn gold.
/// Among the cards it only says what it is; opened, how to run it.
fn action_card(fonts: &Fonts, card: &ActionCard, done: bool, opened: bool) -> Canvas {
    let mut p = paint::face(BLACK);
    // Its own icon if the thermostat has it, else a play button.
    let found = icons::lookup(&card.icon, &card.id).or_else(|| icons::lookup("mdi:play-circle-outline", ""));
    if let Some(found) = found {
        paint::icon(&mut p, found, HALF, 178.0, 104.0, colour(if done { ACTION } else { WHITE }));
    }
    let name = paint::fit(fonts.regular(), 40.0, &card.name, 0.0, 330.0);
    paint::text(&mut p, fonts.regular(), 40.0, HALF, 288.0, &name, 0.0, WHITE);
    let hint = if done { "Done" } else if opened { "Hold to run" } else { "" };
    line(&mut p, fonts, 330.0, hint, if done { ACTION } else { GREY });
    p
}

/// A camera's latest still, the whole display; before the first, its icon
/// and name.
fn camera_card(fonts: &Fonts, card: &CameraCard) -> Canvas {
    let mut p = paint::face(BLACK);
    match &card.art {
        // Its name shows above the cards: the picture alone.
        Some(art) => paint::picture(&mut p, &art.rgb, art.size, HALF, HALF, 0.0),
        None => {
            paint::ring(&mut p, |_| colour(DIM), &[]);
            let icon_name = if card.icon.is_empty() { "mdi:cctv" } else { &card.icon };
            icon(&mut p, icon_name, &card.id, HALF, 178.0, 96.0, WHITE);
            let name = paint::fit(fonts.regular(), 36.0, &card.name, 0.0, 330.0);
            paint::text(&mut p, fonts.regular(), 36.0, HALF, 286.0, &name, 0.0, WHITE);
            line(&mut p, fonts, 328.0, "Press for a picture", GREY);
        }
    }
    p
}

/// A camera opened: its picture, or its card until the first comes, with
/// its name over it.
fn camera_view(fonts: &Fonts, card: &CameraCard) -> Canvas {
    let mut p = camera_card(fonts, card);
    if card.art.is_some() {
        let name = paint::fit(fonts.medium(), 24.0, &card.name, 0.0, 280.0);
        let width = paint::measure(fonts.medium(), 24.0, &name, 0.0) + 36.0;
        paint::pill(&mut p, HALF - width / 2.0, 386.0, width, 40.0, alpha(BLACK, 0.6));
        paint::text(&mut p, fonts.medium(), 24.0, HALF, 414.0, &name, 0.0, WHITE);
    }
    p
}

/// `text` in lines no wider than `width`, at most `lines` of them, the
/// last cut short if it must be.
fn wrap(font: &ab_glyph::FontArc, size: f32, text: &str, width: f32, lines: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let tried = if current.is_empty() { word.to_owned() } else { format!("{current} {word}") };
        if paint::measure(font, size, &tried, 0.0) <= width || current.is_empty() {
            current = tried;
        } else {
            out.push(std::mem::take(&mut current));
            current = word.to_owned();
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    if out.len() > lines {
        let rest = out[lines - 1..].join(" ");
        out.truncate(lines - 1);
        out.push(rest);
    }
    out.into_iter().map(|line| paint::fit(font, size, &line, 0.0, width)).collect()
}

/// A notification: with a picture (a camera's), the picture and its title
/// over it; else its icon, title and message on black. Press to answer it,
/// hold to put it away.
fn alert_screen(fonts: &Fonts, alert: &Alert) -> Canvas {
    let mut p = paint::face(BLACK);
    let hint = "Hold to dismiss";
    match &alert.art {
        Some(art) => {
            paint::picture(&mut p, &art.rgb, art.size, HALF, HALF, 0.0);
            // The title over the picture, on a band, in up to two lines.
            let title = if alert.title.is_empty() { &alert.message } else { &alert.title };
            let lines = wrap(fonts.medium(), 26.0, title, 300.0, 2);
            let width = lines.iter().map(|l| paint::measure(fonts.medium(), 26.0, l, 0.0)).fold(0.0, f32::max) + 44.0;
            let height = 18.0 + 32.0 * lines.len() as f32;
            let top = 412.0 - height;
            paint::pill(&mut p, HALF - width / 2.0, top, width, height, alpha(BLACK, 0.65));
            for (i, text) in lines.iter().enumerate() {
                paint::text(&mut p, fonts.medium(), 26.0, HALF, top + 34.0 + 32.0 * i as f32, text, 0.0, WHITE);
            }
        }
        None => {
            let icon_name = if alert.icon.is_empty() { "mdi:bell-ring" } else { &alert.icon };
            icon(&mut p, icon_name, "alert", HALF, 132.0, 80.0, ACTION);
            let mut y = 222.0;
            if !alert.title.is_empty() {
                let title = paint::fit(fonts.medium(), 34.0, &alert.title, 0.0, 340.0);
                paint::text(&mut p, fonts.medium(), 34.0, HALF, y, &title, 0.0, WHITE);
                y += 44.0;
            }
            for text in wrap(fonts.regular(), 24.0, &alert.message, 330.0, 3) {
                paint::text(&mut p, fonts.regular(), 24.0, HALF, y, &text, 0.0, [220, 223, 228]);
                y += 32.0;
            }
            line(&mut p, fonts, 372.0, hint, GREY);
        }
    }
    p
}

/// No ring: there's nothing to turn.
fn settings_card(fonts: &Fonts) -> Canvas {
    let mut p = paint::face(BLACK);
    icon(&mut p, "mdi:cog", "settings", HALF, 168.0, 96.0, WHITE);
    paint::text(&mut p, fonts.regular(), 40.0, HALF, 278.0, "Settings", 0.0, WHITE);
    line(&mut p, fonts, 316.0, "Eco · Home/Away · Fledge", GREY);
    p
}

/// A heating thermostat as `theme` draws it, small: a theme's preview.
fn theme_preview(fonts: &Fonts, theme: paint::Theme) -> (Vec<u8>, u32) {
    let shown = if paint::fledge() { paint::Theme::Fledge } else { paint::Theme::Nest };
    paint::set_theme(theme);
    let card = ClimateCard { id: "preview".into(), name: String::new(), icon: String::new(), mode: "heat".into(),
                             action: "heating".into(), target: Some(22.0), current: Some(20.5), min: 9.0, max: 32.0,
                             step: 0.5, ..ClimateCard::default() };
    let mut p = paint::face(climate_colours(&card).0);
    climate_moving(&mut p, None, fonts, &card, card.target, None, None, Some((0.0, 22.0)));
    let rgb = p.rgb_upright();
    paint::set_theme(shown);
    // The whole dial, its ring round the edge.
    (rgb, paint::SIZE)
}

/// Settings' Theme, as its mockup: the two looks side by side, each a small
/// live dial, the chosen one on the highlight; turn to choose, press to keep.
fn theme_pick(fonts: &Fonts, fledge: bool) -> Canvas {
    let mut p = paint::face(BLACK);
    label(&mut p, fonts, 128.0, "Theme", GREY);
    let side = 112.0;
    for (i, (theme, name)) in [(paint::Theme::Nest, "Nest"), (paint::Theme::Fledge, "Fledge")].into_iter().enumerate() {
        let chosen = (i == 1) == fledge;
        let cx = HALF + if i == 0 { -84.0 } else { 84.0 };
        if chosen {
            paint::rounded(&mut p, cx - 76.0, 160.0, 152.0, 196.0, 30.0, colour(paint::select()));
        }
        let (rgb, size) = theme_preview(fonts, theme);
        paint::picture_scaled(&mut p, &rgb, size, side as u32, cx, 240.0, side / 2.0);
        let tint = if chosen { WHITE } else { GREY };
        let font = if chosen { fonts.medium() } else { fonts.regular() };
        paint::text(&mut p, font, 22.0, cx, 332.0, name, 0.0, tint);
    }
    line(&mut p, fonts, 392.0, "Turn to choose · press to keep", GREY);
    p
}

/// Eco's leaf green, as the Nest shows it.
const ECO: [u8; 3] = [76, 175, 80];
/// A setting that's on, as the Nest's own Settings show it.
const ON: [u8; 3] = [74, 144, 226];

/// A temperature as the thermostat shows it: 20.5° (or 69°F's 69°).
fn degrees(t: f64) -> String {
    if t.fract().abs() < 0.05 { format!("{t:.0}°") } else { format!("{t:.1}°") }
}

/// What the thermostat is doing, above its number, as the Nest's dial says
/// it: when it'll reach its target, else heating or cooling.
fn nest_doing(nest: &Nest) -> &'static str {
    match nest.card.action.as_str() {
        "heating" => "Heating",
        "cooling" => "Cooling",
        _ => "",
    }
}

/// "In 20 min" while it heats or cools towards its target and knows when
/// it'll get there.
fn nest_eta(nest: &Nest, now: f64) -> Option<String> {
    let at = nest.time_to_target.filter(|at| *at > now)?;
    if !matches!(nest.card.action.as_str(), "heating" | "cooling") {
        return None;
    }
    let minutes = ((at - now) / 60.0).ceil().max(1.0) as i64;
    Some(match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("In {minutes} min"),
        (hours, 0) => format!("In {hours} h"),
        (hours, minutes) => format!("In {hours} h {minutes} min"),
    })
}

/// The thermostat as its target set to `target` makes it: heating as
/// soon as the target is above the temperature now (cooling below, when it
/// cools), idle else, as the Nest's dial turns orange, blue or black as the
/// ring passes it, before nlclient has said.
/// A thermostat as its dial shows it set to `target`: predicted while the
/// ring sets it (`setting`, or a target it hasn't taken yet), so its colour
/// follows the hand; else as it reports, so the dial glows as its card does.
fn setting_to(nest: &Nest, target: f64, setting: bool) -> Cow<'_, Nest> {
    match nest.card.target {
        Some(t) if !setting && (t - target).abs() < 1e-6 => Cow::Borrowed(nest),
        _ => Cow::Owned(predicted(nest, target)),
    }
}

pub fn predicted(nest: &Nest, target: f64) -> Nest {
    let mut nest = nest.clone();
    let card = &mut nest.card;
    if let (Some(now), false) = (card.current, card.mode == "off") {
        let heats = matches!(card.mode.as_str(), "heat" | "heat_cool") && target > now;
        let cools = matches!(card.mode.as_str(), "cool" | "heat_cool") && target < now;
        card.action = if heats { "heating" } else if cools { "cooling" } else { "idle" }.into();
    }
    nest
}

/// The thermostat without its ring and number: heating orange, cooling
/// blue, else black, as the Nest's own. Above the number what it's doing
/// (set to, while the ring turns; when it'll get there; heating), under it
/// until when the schedule keeps it there, and the temperature now by its
/// tick.
fn nest_still(fonts: &Fonts, nest: &Nest, target: Option<f64>, setting: bool, now: f64, parked: Parked) -> Canvas {
    let card = &nest.card;
    let (background, active) = climate_colours(card);
    let mut p = paint::face(background);
    let soft = if active { [255, 225, 205] } else { GREY };
    let parked = parked.filter(|_| card.mode != "off" && target.is_some());
    // At or past a limit: it says so where the preset goes, and where the
    // thermostat's name goes too if it has more to say.
    if !parked.as_ref().is_some_and(|edge| parked_words(&mut p, fonts, edge, PARKED_WORDS, soft)) {
        // At the foot, in the ring's gap: a hot water boost running, else
        // which thermostat this is.
        match nest.hot_water_boost {
            Some(minutes) => line(&mut p, fonts, 392.0, &format!("Hot water boost · {minutes} min"), soft),
            None => line(&mut p, fonts, 392.0, &card.name, soft),
        }
    }
    if card.mode == "off" {
        paint::text(&mut p, fonts.digits(), 96.0, HALF, NUMBER_BASELINE - 18.0, "OFF", 0.0, WHITE);
        return p;
    }
    if target.is_none() {
        // A mode with no temperature to set (dry, fan only): its name in
        // the number's place.
        let mode = paint::fit(fonts.digits(), 64.0, &climate_label(&card.mode), 0.0, 300.0);
        paint::text(&mut p, fonts.digits(), 64.0, HALF, NUMBER_BASELINE - 30.0, &mode, 0.0, WHITE);
    }
    if nest.eco && !setting {
        icon(&mut p, "mdi:leaf", "eco", HALF, 148.0, 40.0, if active { WHITE } else { ECO });
    } else {
        // As the Nest's dial: heating (or cooling) towards it, when it'll
        // get there, else Heating; short of it, what it's set to. Fledge's
        // pill says both, a flame (snowflake) before them.
        let above = match card.action.as_str() {
            "heating" | "cooling" if paint::fledge() => Some(match nest_eta(nest, now) {
                Some(eta) => format!("{} · {}", nest_doing(nest), eta.trim_start_matches("In ")),
                None => nest_doing(nest).to_owned(),
            }),
            "heating" | "cooling" => nest_eta(nest, now).or_else(|| Some(nest_doing(nest).to_owned())),
            _ if target.is_none() => None,
            _ => Some(match card.mode.as_str() {
                "cool" => "Cool set to",
                "heat" => "Heat set to",
                _ => "Set to",
            }.to_owned()),
        };
        let state_icon = match card.action.as_str() {
            "heating" => Some("mdi:fire"),
            "cooling" => Some("mdi:snowflake"),
            _ => None,
        };
        if let Some(above) = above {
            let tint = match (active, paint::fledge()) {
                (true, true) if card.action == "cooling" => [170, 205, 255],
                (true, true) => [255, 201, 166],
                (true, false) => WHITE,
                (false, _) => GREY,
            };
            dial_words(&mut p, fonts, ABOVE_BASELINE, &above, tint, state_icon);
        }
        // Under it, the preset it's in (away, boost, ...) and until when
        // the schedule keeps it there.
        let under: Vec<String> = [
            card.active_preset().map(climate_label),
            nest.next_change.as_ref().map(|until| format!("Until {until}")),
        ].into_iter().flatten().collect();
        if !under.is_empty() && parked.is_none() {
            let icon_name = card.active_preset().map(preset_icon);
            dial_words(&mut p, fonts, UNDER_BASELINE, &under.join(" · "), soft, icon_name);
        }
    }
    p
}

/// The thermostat's card: as its dial shows it, or Eco and its leaf.
fn nest_card(fonts: &Fonts, nest: &Nest, now: f64) -> Canvas {
    let card = &nest.card;
    // Off: the ring without a target on it.
    let target = card.target.filter(|_| card.mode != "off");
    let mut p = nest_still(fonts, nest, target, false, now, None);
    if nest.eco && card.mode != "off" {
        let (_, active) = climate_colours(card);
        let tick = card.current.map(|t| paint::tick_at(((t - card.min) / (card.max - card.min).max(0.5)) as f32));
        paint::ring_shaped(&mut p, |i| if Some(i) == tick {
            (colour(if active { NOW } else { GREY }), paint::Shape::Wide)
        } else {
            (colour(tick_base(card)), paint::Shape::Short)
        });
        now_label(&mut p, fonts, card, target, if now_differs(card, target) { 1.0 } else { 0.0 });
        paint::text(&mut p, fonts.digits(), 120.0, HALF, NUMBER_BASELINE - 8.0, if paint::fledge() { "Eco" } else { "ECO" }, 0.0, WHITE);
        return p;
    }
    climate_moving(&mut p, None, fonts, card, target, None, None, None);
    p
}

/// The wall clock, in Unix seconds.
fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// A row of Settings: its icon, its name, and what it's set to.
fn setting_row(setting: Setting, nest: Option<&Nest>, nest_dial: bool) -> (&'static str, &'static str, String) {
    let switch = |on: bool| if on { "On" } else { "Off" }.to_owned();
    let get = |f: fn(&Nest) -> bool| nest.map(f).map(switch).unwrap_or_default();
    let temperature = |t: Option<f64>| t.map(degrees).unwrap_or_default();
    let cooling = nest.is_some_and(|n| n.can_cool);
    match setting {
        Setting::EcoLow => ("mdi:leaf", if cooling { "Eco, lowest" } else { "Eco temperature" },
                            temperature(nest.and_then(|n| n.eco_low))),
        Setting::EcoHigh => ("mdi:leaf", "Eco, highest", temperature(nest.and_then(|n| n.eco_high))),
        Setting::HomeAway => ("mdi:home-account", "Home/Away Assist", get(|n| n.home_away)),
        Setting::Learning => ("mdi:calendar-clock", "Auto-Schedule", get(|n| n.learning)),
        Setting::EarlyOn => ("mdi:clock-fast", "Early-On", get(|n| n.early_on)),
        Setting::Sunblock => ("mdi:white-balance-sunny", "Sunblock", get(|n| n.sunblock)),
        Setting::HotWaterSchedule => ("mdi:water-boiler", "Hot water schedule", get(|n| n.hot_water_schedule)),
        Setting::Scale => ("mdi:temperature-celsius", "Units",
                           nest.map(|n| if n.fahrenheit { "°F" } else { "°C" }.to_owned()).unwrap_or_default()),
        Setting::NestDial => ("mdi:thermometer", "Nest's own dial", switch(nest_dial)),
        Setting::Theme => ("mdi:palette", "Theme", if paint::fledge() { "Fledge" } else { "Nest" }.to_owned()),
        Setting::Software => ("mdi:update", "Fledge", match nest {
            Some(Nest { update: Some(tag), .. }) => format!("Install {tag}"),
            // v0.4.9, not a development build's v0.4.9-1-g42707b2-dirty.
            Some(n) => n.version.split('-').next().unwrap_or_default().to_owned(),
            None => String::new(),
        }),
        Setting::More => ("mdi:dots-horizontal", "More Nest settings", String::new()),
        Setting::Done => ("mdi:check", "Done", String::new()),
    }
}

/// Settings, listed as a device's lights are, with what each is set to.
fn settings_list(fonts: &Fonts, rows: &[Setting], nest: Option<&Nest>, nest_dial: bool, selected: usize,
                 layer: Layer) -> Canvas {
    let first = first_row(selected, rows.len());
    let mut p = list_backdrop(layer, selected.checked_sub(first));
    label(&mut p, fonts, LIST_TITLE, "Settings", GREY);
    for (slot, row) in (first..rows.len().min(first + ROWS)).enumerate() {
        let y = LIST_TOP + slot as f32 * ROW_PITCH;
        let (x, w, h) = PILL;
        let chosen = layer.chosen(row == selected);
        let (icon_name, name, value) = setting_row(rows[row], nest, nest_dial);
        let update = rows[row] == Setting::Software && nest.is_some_and(|n| n.update.is_some());
        let value_tint = if chosen { WHITE } else if update || value == "On" { ON } else { GREY };
        icon(&mut p, icon_name, "setting", x + 34.0, y + h / 2.0, 28.0, if chosen { WHITE } else { GREY });
        let value_width = paint::measure(fonts.regular(), 24.0, &value, 0.0);
        let name = paint::fit(fonts.regular(), 26.0, name, 0.0, w - 90.0 - value_width - 16.0);
        paint::text_at(&mut p, fonts.regular(), 26.0, x + 62.0, y + 37.0, &name, 0.0, WHITE, 1.0);
        paint::text_at(&mut p, fonts.regular(), 24.0, x + w - 24.0 - value_width, y + 36.0, &value, 0.0, value_tint, 1.0);
    }
    p
}

/// A setting's temperature dial, without its ring and number.
fn setting_still(fonts: &Fonts, setting: Setting) -> Canvas {
    let mut p = paint::face(BLACK);
    icon(&mut p, "mdi:leaf", "eco", HALF, 98.0, 40.0, ECO);
    let (_, name, _) = setting_row(setting, None, false);
    label(&mut p, fonts, 150.0, name, ECO);
    line(&mut p, fonts, 366.0, "Press when done", GREY);
    p
}

#[allow(clippy::too_many_arguments)]
fn setting_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, setting: Setting, nest: &Nest,
                  value: f64, sweep: Option<f32>) {
    let Some((low, high, step)) = setting.range(nest.fahrenheit) else { return };
    let mark = paint::tick_at(sweep.unwrap_or(((value - low) / (high - low).max(0.5)) as f32));
    ring_on(p, live.as_deref_mut(), |i| if i == mark { colour(ECO) } else { colour(DIM) }, &[mark]);
    big_number_on(p, live, fonts, value, step < 1.0, "", WHITE);
}

const ROWS: usize = 5;
/// A list's rows: where the first's top is, how far apart, and the
/// highlight's place and size on a row.
///
/// Five rows of the highlight must all lie on the round display: the last
/// one's lower corners are the nearest anything on a list comes to its edge
/// (list_rows_lie_on_the_round_display).
pub const LIST_TOP: f32 = 122.0;
pub const ROW_PITCH: f32 = 58.0;
pub const PILL: (f32, f32, f32) = (62.0, 356.0, 54.0);
/// A list's title, over its rows.
const LIST_TITLE: f32 = 96.0;

/// Where a list's rows are, top and bottom as it stands: what slides as
/// it scrolls a row, under its title.
pub fn list_band() -> (f32, f32) {
    (LIST_TOP - (ROW_PITCH - PILL.2) / 2.0 - 2.0, LIST_TOP + ROWS as f32 * ROW_PITCH)
}

/// How a list is drawn: whole, its highlight on the chosen row; or in two
/// layers for the highlight to glide between rows (Runtime): every row as
/// it shows unchosen, and every row as it shows under the highlight, on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layer {
    Whole,
    Plain,
    Chosen,
}

impl Layer {
    fn chosen(self, chosen: bool) -> bool {
        match self {
            Layer::Whole => chosen,
            Layer::Plain => false,
            Layer::Chosen => true,
        }
    }
}

/// The first row a list of `rows` shows with `selected` chosen.
fn first_row(selected: usize, rows: usize) -> usize {
    selected.saturating_sub(ROWS - 1).min(rows.saturating_sub(ROWS))
}

/// What a list's rows are drawn on: the face with the highlight on the
/// chosen row (Whole) or none (Plain), or the highlight's colour across it
/// all (Chosen), for its pill to be cut from as it glides.
fn list_backdrop(layer: Layer, slot: Option<usize>) -> Canvas {
    let (x, w, h) = PILL;
    match (layer, slot) {
        // Only its pill is cut from it: no face under it to make.
        (Layer::Chosen, _) => paint::canvas(paint::select()),
        (Layer::Whole, Some(slot)) => {
            let mut p = paint::face(BLACK);
            paint::pill(&mut p, x, LIST_TOP + slot as f32 * ROW_PITCH, w, h, colour(paint::select()));
            p
        }
        _ => paint::face(BLACK),
    }
}

/// A list screen's chosen row and the first it shows, for its highlight.
pub fn list_place(screen: &Screen) -> Option<(usize, usize)> {
    let (selected, rows) = match screen {
        Screen::LightList { card, selected } => (*selected, card.lights.len() + 1),
        Screen::Members { card, selected } => (*selected, card.members.len() + 1),
        Screen::OptionList { card, selected } => (*selected, card.options.len() + 1),
        Screen::Settings { rows, selected, .. } => (*selected, rows.len()),
        Screen::ModePrompt { selected, .. } => (*selected, PROMPT_ROWS),
        _ => return None,
    };
    Some((selected, first_row(selected, rows)))
}

/// A list screen drawn as `layer`, and what it shows apart from its
/// highlight (to know when its layers can be kept).
pub fn list_layer(fonts: &Fonts, screen: &Screen, layer: Layer) -> Option<Canvas> {
    Some(match screen {
        Screen::LightList { card, selected } => lights_listed(fonts, &card.name, &card.lights, *selected, layer),
        Screen::Members { card, selected } => lights_listed(fonts, &card.name, &card.members, *selected, layer),
        Screen::OptionList { card, selected } => option_list(fonts, card, *selected, layer),
        Screen::Settings { rows, nest, nest_dial, selected } => settings_list(fonts, rows, *nest, *nest_dial, *selected, layer),
        Screen::ModePrompt { nest, selected } => mode_prompt(fonts, nest, *selected, layer),
        _ => return None,
    })
}

/// The windows a list scrolls to next from where it is: its first row one
/// or two on, or back. As the same list with a row chosen that shows them.
pub fn list_neighbours<'a>(screen: &Screen<'a>) -> Vec<Screen<'a>> {
    let Some((_, first)) = list_place(screen) else { return Vec::new() };
    let rows = match screen {
        Screen::LightList { card, .. } => card.lights.len() + 1,
        Screen::Members { card, .. } => card.members.len() + 1,
        Screen::OptionList { card, .. } => card.options.len() + 1,
        Screen::Settings { rows, .. } => rows.len(),
        _ => return Vec::new(),
    };
    // The last row of the window one on, two on; the first of one back, two back.
    let chosen = [Some(first + ROWS), Some(first + ROWS + 1), first.checked_sub(1), first.checked_sub(2)];
    chosen.into_iter().flatten().filter(|&row| row < rows).filter_map(|row| list_with(screen, row)).collect()
}

/// The same list with `selected` chosen.
fn list_with<'a>(screen: &Screen<'a>, selected: usize) -> Option<Screen<'a>> {
    Some(match screen {
        Screen::LightList { card, .. } => Screen::LightList { card, selected },
        Screen::Members { card, .. } => Screen::Members { card, selected },
        Screen::OptionList { card, .. } => Screen::OptionList { card: card.clone(), selected },
        Screen::Settings { rows, nest, nest_dial, .. } => Screen::Settings { rows: rows.clone(), nest: *nest, nest_dial: *nest_dial, selected },
        _ => return None,
    })
}

pub fn list_key(screen: &Screen) -> Option<String> {
    let (_, first) = list_place(screen)?;
    Some(match screen {
        Screen::LightList { card, .. } => format!("lights {first} {card:?}"),
        Screen::Members { card, .. } => format!("members {first} {card:?}"),
        Screen::OptionList { card, .. } => format!("options {first} {card:?}"),
        Screen::Settings { rows, nest, nest_dial, .. } => format!("settings {first} {rows:?} {nest:?} {nest_dial}"),
        Screen::ModePrompt { nest, .. } => format!("prompt {first} {}", nest.on_mode()),
        _ => return None,
    })
}

fn light_list(fonts: &Fonts, card: &LightsCard, selected: usize) -> Canvas {
    lights_listed(fonts, &card.name, &card.lights, selected, Layer::Whole)
}

/// Lights listed under `name`, each with its state, then Back.
fn lights_listed(fonts: &Fonts, name: &str, lights: &[Light], selected: usize, layer: Layer) -> Canvas {
    let rows = lights.len() + 1;
    let first = first_row(selected, rows);
    let mut p = list_backdrop(layer, selected.checked_sub(first));
    let accent = lights.iter().find(|l| l.on).map_or(GREY, |l| l.colour());
    label(&mut p, fonts, LIST_TITLE, name, accent);
    for (slot, row) in (first..rows.min(first + ROWS)).enumerate() {
        let y = LIST_TOP + slot as f32 * ROW_PITCH;
        let (x, w, h) = PILL;
        let chosen = layer.chosen(row == selected);
        let (name, value, tint, value_tint, icon_name, entity) = match lights.get(row) {
            Some(light) => {
                let value = match (light.on, light.brightness) {
                    (true, Some(b)) if light.dimmable => format!("{b}%"),
                    (true, _) => "On".to_owned(),
                    (false, _) => "Off".to_owned(),
                };
                let tint = if light.on { light.colour() } else { DIM };
                (light.name.as_str(), value, tint, if light.on { light.colour() } else { GREY },
                 light.icon.as_str(), light.id.as_str())
            }
            None => ("Back", String::new(), GREY, GREY, "mdi:arrow-left", "back"),
        };
        let (tint, value_tint, text_tint) = if chosen { (WHITE, WHITE, WHITE) } else { (tint, value_tint, WHITE) };
        icon(&mut p, icon_name, entity, x + 34.0, y + h / 2.0, 28.0, tint);
        let value_width = paint::measure(fonts.regular(), 24.0, &value, 0.0);
        let name = paint::fit(fonts.regular(), 26.0, name, 0.0, w - 90.0 - value_width - 16.0);
        paint::text_at(&mut p, fonts.regular(), 26.0, x + 62.0, y + 37.0, &name, 0.0, text_tint, 1.0);
        paint::text_at(&mut p, fonts.regular(), 24.0, x + w - 24.0 - value_width, y + 36.0, &value, 0.0, value_tint, 1.0);
    }
    p
}

/// A light's dimmer (and its card) without the ring and number: its icon
/// and name, Off, and what holding (a press, for one that only switches)
/// does.
fn light_dim_still(fonts: &Fonts, light: &Light, on: bool, opened: bool, parked: Parked) -> Canvas {
    let mut p = paint::face(BLACK);
    let accent = if on { light.colour() } else { GREY };
    icon(&mut p, &light.icon, &light.id, HALF, 98.0, 40.0, accent);
    label(&mut p, fonts, 150.0, &light.name, accent);
    if !on || !light.dimmable {
        let words = if on { "On" } else { "Off" };
        paint::text(&mut p, fonts.regular(), 96.0, HALF, NUMBER_BASELINE - 8.0, words, 0.0, WHITE);
    }
    let hint = match (light.dimmable, on) {
        (true, true) => "Hold to turn off",
        (true, false) => "Hold to turn on",
        (false, true) => "Hold to turn off",
        (false, false) => "Hold to turn on",
    };
    if !parked.as_ref().is_some_and(|edge| parked_words(&mut p, fonts, edge, PARKED_WORDS, GREY)) && opened {
        line(&mut p, fonts, 366.0, hint, GREY);
    }
    p
}

#[allow(clippy::too_many_arguments)]
fn light_dim_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, light: &Light, on: bool,
                    value: u8, sweep: Option<f32>) {
    let accent = if on { light.colour() } else { GREY };
    if !light.dimmable {
        // A switch: its ring whole, lit while it's on.
        ring_on(p, live.as_deref_mut(), |_| if on { colour(accent) } else { colour(DIM) }, &[]);
        return;
    }
    let ends = light.brightness_ends().map(f32::from);
    let top = paint::tick_at(sweep.unwrap_or(along(value as f32, ends)));
    ring_on(p, live.as_deref_mut(), |i| if on && i <= top { colour(accent) } else { colour(DIM) },
            &if on { vec![top] } else { vec![] });
    if on {
        big_number_on(p, live, fonts, value as f64, false, "%", number_tint(light.brightness_ring(), value as i32));
    }
}

/// A light's warmth or colour dial without its ring and value: its name and
/// what the dial sets.
fn light_dial_still(fonts: &Fonts, light: &Light, what: &str, parked: Parked) -> Canvas {
    let mut p = paint::face(BLACK);
    icon(&mut p, &light.icon, &light.id, HALF, 98.0, 40.0, GREY);
    label(&mut p, fonts, 150.0, &light.name, GREY);
    if !parked.as_ref().is_some_and(|edge| parked_words(&mut p, fonts, edge, PARKED_WORDS, GREY)) {
        line(&mut p, fonts, 366.0, &format!("{what} · press when done"), GREY);
    }
    p
}

/// A white light's colour at `kelvin`: candle orange to daylight blue.
pub fn kelvin_rgb(kelvin: f32) -> [u8; 3] {
    const STOPS: [(f32, [f32; 3]); 4] =
        [(2000.0, [255.0, 138.0, 18.0]), (3000.0, [255.0, 180.0, 107.0]), (4500.0, [255.0, 228.0, 206.0]),
         (6500.0, [201.0, 218.0, 255.0])];
    let k = kelvin.clamp(STOPS[0].0, STOPS[3].0);
    let i = STOPS.windows(2).position(|w| k <= w[1].0).unwrap_or(2);
    let ((k0, a), (k1, b)) = (STOPS[i], STOPS[i + 1]);
    let t = (k - k0) / (k1 - k0);
    [0, 1, 2].map(|c| (a[c] + (b[c] - a[c]) * t).round() as u8)
}

/// The fully saturated colour at `hue` degrees.
pub fn hue_rgb(hue: f32) -> [u8; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, g, b) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    [r, g, b].map(|c: f32| (c * 255.0).round() as u8)
}

/// The warmth dial: the ring warm to cool across the light's range, the
/// mark at its white, the kelvin in the middle.
fn warmth_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, light: &Light, value: u16,
                 sweep: Option<f32>) {
    let [low, high] = light.kelvin_ends();
    let span = (high as f32 - low as f32).max(1.0);
    let fraction = sweep.unwrap_or((value as f32 - low as f32) / span);
    let mark = paint::tick_at(fraction);
    let last = (paint::TICKS - 1) as f32;
    ring_on(p, live.as_deref_mut(), |i| colour(kelvin_rgb(low as f32 + span * i as f32 / last)), &[mark]);
    big_text_on(p, live, fonts, &value.to_string(), "K", 120.0, number_tint(light.kelvin_ring(), value as i32));
}

/// The colour dial: the ring round the colour wheel, the mark at the
/// light's hue, a swatch of it in the middle.
fn hue_moving(p: &mut Canvas, live: Option<&mut paint::Live>, _fonts: &Fonts, value: u16, sweep: Option<f32>) {
    let mark = paint::tick_at(sweep.unwrap_or(value as f32 / 360.0));
    let last = (paint::TICKS - 1) as f32;
    ring_on(p, live, |i| colour(hue_rgb(360.0 * i as f32 / last)), &[mark]);
    // Opaque over the last one, the edge on black, so nothing builds up.
    paint::circle(p, HALF, 258.0, 76.0, colour(BLACK));
    paint::circle(p, HALF, 258.0, 72.0, colour(hue_rgb(value as f32)));
}

/// Text as large as the dial's number, `unit` raised beside it: for values
/// too long for big_number's size.
#[allow(clippy::too_many_arguments)]
fn big_text_on(pixmap: &mut Canvas, live: Option<&mut paint::Live>, fonts: &Fonts, digits: &str, unit: &str, size: f32,
               c: [u8; 3]) {
    let font = fonts.digits();
    let width = paint::measure(font, size, digits, -3.0);
    let small = size * 0.31;
    let small_width = if unit.is_empty() { 0.0 } else { paint::measure(font, small, unit, 0.0) + 6.0 };
    let x = HALF - (width + small_width) / 2.0;
    let mut parts = vec![(font, size, x, 300.0, digits, -3.0)];
    if !unit.is_empty() {
        parts.push((font, small, x + width + 4.0, 300.0 - size * 0.7 + small, unit, 0.0));
    }
    match live {
        Some(live) => paint::number_live(pixmap, live, &parts, c),
        None => {
            for (font, size, x, y, text, spacing) in parts {
                paint::text_at(pixmap, font, size, x, y, text, spacing, c, 1.0);
            }
        }
    }
}

fn volume_still(fonts: &Fonts, card: &MediaCard, parked: Parked) -> Canvas {
    let mut p = paint::face(BLACK);
    label(&mut p, fonts, 150.0, &card.name, MEDIA);
    if !parked.as_ref().is_some_and(|edge| parked_words(&mut p, fonts, edge, PARKED_WORDS, GREY)) {
        let action = if card.state == "playing" { "Hold to pause" } else { "Hold to play" };
        line(&mut p, fonts, 366.0, action, GREY);
    }
    p
}

fn volume_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, card: &MediaCard, value: u8,
                 sweep: Option<f32>) {
    let top = paint::tick_at(sweep.unwrap_or(along(value as f32, card.volume_ends().map(f32::from))));
    ring_on(p, live.as_deref_mut(), |i| if i <= top { colour(MEDIA) } else { colour(DIM) }, &[top]);
    big_number_on(p, live, fonts, value as f64, false, "", number_tint(card.volume_ring(), value as i32));
}

/// What a glance draws from: Home Assistant's glance, the cards (what's
/// on), this thermostat (its temperature, what it's doing) and the time
/// (`now`, Unix seconds).
#[derive(Clone, Copy)]
pub struct GlanceIn<'a> {
    pub glance: &'a Glance,
    pub cards: &'a [Card],
    pub nest: Option<&'a Nest>,
    pub now: i64,
}

impl GlanceIn<'_> {
    /// Minutes since midnight, Home Assistant's time.
    fn minutes(&self) -> i64 {
        (self.now + self.glance.offset).rem_euclid(86_400) / 60
    }

    /// Days since 1970, Home Assistant's time.
    fn days(&self) -> i64 {
        (self.now + self.glance.offset).div_euclid(86_400)
    }
}

/// The glance: something large and lines under it, as its layout has them.
fn glance_screen(fonts: &Fonts, at: &GlanceIn) -> Canvas {
    let mut p = glance_still(fonts, at);
    let still = p.clone();
    glance_words(&mut p, &still, fonts, at, 0.0);
    p
}

/// The colour of Fledge's line of what's on.
const FLEDGE_ON: [u8; 3] = [255, 211, 154];
/// The glance's quieter words (the date).
const GLANCE_QUIET: [u8; 3] = [196, 203, 211];

/// How a glance line is drawn in its place.
#[derive(Clone, Copy, PartialEq)]
enum Style {
    /// Smaller, grey, no icon (the date under the time).
    Quiet,
    /// White with its icon (the weather).
    Main,
    /// In a soft pill (what's on); it scrolls where it's too long.
    Pill,
}

/// A line's place: its baseline (a pill's middle), how, and how wide.
#[derive(Clone, Copy)]
struct Slot {
    y: f32,
    style: Style,
    room: f32,
}

const fn slot(y: f32, style: Style, room: f32) -> Slot {
    Slot { y, style, room }
}

const CLOCK_SLOTS: [Slot; 3] = [slot(290.0, Style::Quiet, 380.0), slot(352.0, Style::Main, 330.0), slot(404.0, Style::Pill, 290.0)];
const LARGE_SLOTS: [Slot; 2] = [slot(352.0, Style::Main, 330.0), slot(402.0, Style::Quiet, 300.0)];
const ANALOG_SLOTS: [Slot; 1] = [slot(350.0, Style::Quiet, 320.0)];
const NUMBER_SLOTS: [Slot; 2] = [slot(370.0, Style::Quiet, 320.0), slot(418.0, Style::Pill, 230.0)];
const AGENDA_SLOTS: [Slot; 1] = [slot(370.0, Style::Main, 320.0)];
const BATTERY_SLOTS: [Slot; 1] = [slot(398.0, Style::Quiet, 280.0)];

/// Where each layout puts its lines, top to bottom.
fn slots(layout: Layout) -> &'static [Slot] {
    match layout {
        Layout::Clock | Layout::Unknown => &CLOCK_SLOTS,
        Layout::Temp | Layout::Weather => &LARGE_SLOTS,
        Layout::Analog => &ANALOG_SLOTS,
        Layout::Number => &NUMBER_SLOTS,
        Layout::Agenda => &AGENDA_SLOTS,
        Layout::Battery => &BATTERY_SLOTS,
    }
}

/// A line worked out: its words, icon and colour (None: the style's).
#[derive(Debug, PartialEq)]
struct Said {
    text: String,
    icon: Option<String>,
    colour: Option<[u8; 3]>,
}

fn said(text: String, icon: Option<&str>, colour: Option<[u8; 3]>) -> Option<Said> {
    (!text.trim().is_empty()).then(|| Said { text, icon: icon.map(str::to_owned), colour })
}

/// What a line says: the Nest's own kinds worked out here, Home
/// Assistant's words as they came; None for nothing to say.
fn line_said(line: &Line, at: &GlanceIn) -> Option<Said> {
    let Some(kind) = line.kind.as_deref() else {
        return said(line.text.clone().unwrap_or_default(), line.icon.as_deref().filter(|i| !i.is_empty()), None);
    };
    let clock = &at.glance.clock;
    match kind {
        "date" => said(glance_date(at.glance, at.days(), false), None, None),
        "date_short" => said(glance_date(at.glance, at.days(), true), None, None),
        "time" => said(clock.time(at.minutes(), true), Some("mdi:clock-outline"), None),
        "whats_on" => said(whats_on(at.cards), None, None),
        "weather" => {
            let weather = at.glance.weather.as_ref()?;
            said(weather_words(weather), Some(&format!("mdi:{}", weather_icon(&weather.condition))), None)
        }
        "inside" => {
            let nest = at.nest?;
            let now = degrees(nest.card.current?);
            let text = match nest.humidity {
                Some(h) => format!("Inside {now} · {h:.0}%"),
                None => format!("Inside {now}"),
            };
            said(text, Some("mdi:thermometer"), None)
        }
        "heating" => {
            let nest = at.nest?;
            let card = &nest.card;
            let eta = nest_eta(nest, at.now as f64).map(|eta| eta.trim_start_matches("In ").to_owned());
            let to = card.target.map(degrees).unwrap_or_default();
            let (text, icon, colour) = match card.action.as_str() {
                "heating" | "preheating" => (format!("Heating to {to}"), "mdi:fire", Some([255, 154, 92])),
                "cooling" => (format!("Cooling to {to}"), "mdi:snowflake", Some([130, 178, 255])),
                _ if card.mode == "off" || !nest.acting => ("Heating off".to_owned(), "mdi:thermostat", None),
                _ if nest.eco => ("Eco".to_owned(), "mdi:leaf", Some([120, 200, 124])),
                _ => (format!("Set to {to}"), "mdi:thermostat", None),
            };
            let text = match eta {
                Some(eta) if icon != "mdi:thermostat" => format!("{text} · {eta}"),
                _ => text,
            };
            said(text, Some(icon), colour)
        }
        _ => None,
    }
}

/// The weather in words: "9° · Partly cloudy".
fn weather_words(weather: &Weather) -> String {
    let temperature = weather.temperature.map(|t| format!("{t:.0}{}", if weather.unit.contains('F') { "°F" } else { "°" }));
    [temperature.unwrap_or_default(), condition_name(&weather.condition)]
        .into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
}

/// Each of the glance's lines worked out, in their places.
fn glance_lines(at: &GlanceIn) -> Vec<(Slot, Option<Said>)> {
    // A line with nothing to say takes no place: those after it close up.
    let said: Vec<Said> = at.glance.lines().iter().filter_map(|line| line_said(line, at)).collect();
    slots(at.glance.layout).iter().zip(said.into_iter().map(Some).chain(std::iter::repeat_with(|| None)))
        .map(|(slot, said)| {
            let mut slot = *slot;
            if at.glance.layout == Layout::Analog {
                slot.y = analog_line(at.minutes());
            }
            (slot, said)
        }).collect()
}

/// Where the analog clock's line goes at `minutes` past midnight: under
/// the middle, or above it while a hand points down through its place
/// and none points up.
fn analog_line(minutes: i64) -> f32 {
    let hands = [(minutes.rem_euclid(720)) as f32 / 2.0, minutes.rem_euclid(60) as f32 * 6.0];
    let down = hands.iter().any(|a| (130.0..=230.0).contains(a));
    let up = hands.iter().any(|a| *a >= 310.0 || *a <= 50.0);
    if down && !up { 152.0 } else { ANALOG_SLOTS[0].y }
}

/// The glance's line of what's on: how large, how much room it has, and
/// how wide its words are.
struct GlanceLine {
    size: f32,
    room: f32,
    words: f32,
}

impl GlanceLine {
    fn new(fonts: &Fonts, on: &str, room: f32) -> GlanceLine {
        let (size, room) = if paint::fledge() { (23.0, room) } else { (24.0, room + 10.0) };
        GlanceLine { size, room, words: paint::measure(fonts.medium(), size, on, 0.0) }
    }

    /// Too long for its place: it scrolls through it.
    fn scrolls(&self) -> bool {
        self.words > self.room
    }
}

/// The space between the end of the glance's scrolling line and its start
/// coming round again.
const GLANCE_GAP: f32 = 72.0;

/// The glance's pill line, if it has one with something to say.
fn pill_line(at: &GlanceIn) -> Option<(Slot, Said)> {
    glance_lines(at).into_iter().find_map(|(slot, said)| (slot.style == Style::Pill).then_some(said).flatten().map(|s| (slot, s)))
}

/// How far the glance's pill line scrolls before it is back where it
/// started, in pixels; None where it fits and stays put.
pub fn glance_scroll(fonts: &Fonts, at: &GlanceIn) -> Option<f32> {
    let (slot, said) = pill_line(at)?;
    let line = GlanceLine::new(fonts, &said.text, slot.room);
    line.scrolls().then_some(line.words + GLANCE_GAP)
}

/// Everything the glance shows: another key, another glance to draw.
pub fn glance_key(at: &GlanceIn) -> String {
    let lines: Vec<String> = glance_lines(at).into_iter().map(|(_, said)| format!("{said:?}")).collect();
    let nest = at.nest.map(|n| (n.card.current, n.card.action.as_str(), n.card.mode.as_str()));
    format!("{:?} {} {} {nest:?} {}", at.glance, (at.now + at.glance.offset).div_euclid(60), lines.join("|"), paint::fledge())
}

/// Colour `c` as the glance draws it: dimmer and warmer at night.
fn glance_colour(at: &GlanceIn, c: [u8; 3]) -> [u8; 3] {
    if at.glance.dim { paint::warmed(c, at.glance.dim_level.unwrap_or(paint::DIM_LEVEL)) } else { c }
}

/// The glance's pill line, `offset` pixels along, over glance_still
/// (`still`), where it scrolls: a title on the speaker, a film, a book, too
/// long to show whole. Returns the upright columns drawn in, if any were.
pub fn glance_words(p: &mut Canvas, still: &Canvas, fonts: &Fonts, at: &GlanceIn, offset: f32) -> Option<(u32, u32)> {
    let (slot, said) = pill_line(at)?;
    let line = GlanceLine::new(fonts, &said.text, slot.room);
    if !line.scrolls() {
        return None;
    }
    let (baseline, colour) = if paint::fledge() {
        (slot.y + line.size * 0.36, said.colour.unwrap_or(FLEDGE_ON))
    } else {
        (slot.y - 2.0, said.colour.unwrap_or(ACTION))
    };
    let ticker = paint::Ticker { font: fonts.medium(), size: line.size, text: &said.text, words: line.words,
        left: HALF - line.room / 2.0, width: line.room, baseline, colour: glance_colour(at, colour), gap: GLANCE_GAP };
    Some(paint::ticker(p, still, &ticker, offset))
}

/// The glance without the words of a pill line that scrolls (glance_words).
pub fn glance_still(fonts: &Fonts, at: &GlanceIn) -> Canvas {
    let glance = at.glance;
    let background = match (glance.layout, at.nest) {
        (Layout::Temp, Some(nest)) if glance.colour.unwrap_or(true) => match nest.card.action.as_str() {
            "heating" | "preheating" => HEAT,
            "cooling" => COOL,
            _ => BLACK,
        },
        // Charging fills the Nest's face, as heating does.
        (Layout::Battery, _) if !paint::fledge() && glance.battery.as_ref().is_some_and(|b| b.mood() == model::Mood::Charging) => {
            BATTERY_CHARGING
        }
        _ => BLACK,
    };
    let coloured = background != BLACK;
    let mut p = paint::face(background);
    match glance.layout {
        Layout::Clock | Layout::Unknown => {
            // Big enough to read across a room: the time across most of the
            // face, the rest under it filling the round display's width.
            let size = if paint::fledge() { 196.0 } else { 160.0 };
            big_time(&mut p, fonts, &glance.clock, at.minutes(), size, 236.0, 430.0);
        }
        Layout::Temp => {
            let label = if paint::fledge() { "Inside" } else { "INSIDE" };
            paint::text(&mut p, fonts.medium(), 22.0, HALF, 150.0, label, if paint::fledge() { 1.0 } else { 2.0 },
                        if coloured { [255, 232, 218] } else { GLANCE_QUIET });
            match at.nest.and_then(|n| n.card.current.map(|t| (t, n.fahrenheit))) {
                Some((t, fahrenheit)) => {
                    let value = if fahrenheit { t.round() } else { (t * 2.0).round() / 2.0 };
                    big_number_on(&mut p, None, fonts, value, !fahrenheit, "", WHITE);
                }
                None => paint::text(&mut p, fonts.digits(), 170.0, HALF, NUMBER_BASELINE, "–", 0.0, WHITE),
            }
        }
        Layout::Weather => match &glance.weather {
            Some(weather) => {
                icon(&mut p, &format!("mdi:{}", weather_icon(&weather.condition)), "weather", HALF, 112.0, 64.0, WHITE);
                let words = paint::fit(fonts.regular(), 26.0, &condition_name(&weather.condition), 0.0, 300.0);
                paint::text(&mut p, fonts.regular(), 26.0, HALF, 178.0, &words, 0.0, GLANCE_QUIET);
                let t = weather.temperature.map(|t| format!("{t:.0}°")).unwrap_or_else(|| "–".into());
                hero(&mut p, fonts, &t, if paint::fledge() { 150.0 } else { 128.0 }, 300.0, 400.0, WHITE);
            }
            None => {
                paint::text(&mut p, fonts.regular(), 26.0, HALF, 178.0, "No weather chosen", 0.0, GLANCE_QUIET);
                hero(&mut p, fonts, "–", if paint::fledge() { 150.0 } else { 128.0 }, 300.0, 400.0, WHITE);
            }
        },
        Layout::Analog => analog_ticks(&mut p),
        Layout::Number => {
            let big = glance.big.clone().unwrap_or_default();
            glance_label(&mut p, fonts, 132.0, &big.label);
            let value = if big.value.trim().is_empty() { "–" } else { big.value.trim() };
            hero(&mut p, fonts, value, if paint::fledge() { 170.0 } else { 140.0 }, 270.0, 400.0, WHITE);
            let unit = paint::fit(fonts.regular(), 28.0, &big.unit, 0.0, 280.0);
            paint::text(&mut p, fonts.regular(), 28.0, HALF, 316.0, &unit, 0.0, FLEDGE_ON);
        }
        Layout::Battery => battery_face(&mut p, fonts, glance.battery.as_ref(), coloured),
        Layout::Agenda => {
            let time = glance.clock.time(at.minutes(), true);
            paint::text(&mut p, fonts.clock(), 56.0, HALF, 122.0, &time, -1.0, WHITE);
            glance_label(&mut p, fonts, 178.0, "Up next");
            match glance.event.as_ref().filter(|e| !e.title.trim().is_empty()) {
                Some(event) => {
                    fitted(&mut p, fonts.medium(), 64.0, 40.0, HALF, 250.0, event.title.trim(), 380.0, WHITE);
                    let when = event_when(event, at);
                    let when = paint::fit(fonts.regular(), 28.0, &when, 0.0, 340.0);
                    paint::text(&mut p, fonts.regular(), 28.0, HALF, 300.0, &when, 0.0, FLEDGE_ON);
                }
                None => paint::text(&mut p, fonts.medium(), 40.0, HALF, 250.0, "Nothing coming up", 0.0, WHITE),
            }
        }
    }
    if glance.layout == Layout::Analog {
        analog_hands(&mut p, at.minutes());
    }
    for (slot, said) in glance_lines(at) {
        let Some(said) = said else { continue };
        if glance.layout == Layout::Analog {
            // Over the hands on a plate of the face's own colour, as a
            // watch's date window: read whichever way they point.
            let size = 30.0;
            let words = paint::measure(fonts.regular(), size, &said.text, 0.0).min(slot.room);
            let (width, height) = (words + 36.0, 46.0);
            let face = if paint::fledge() { paint::FACE } else { BLACK };
            paint::pill(&mut p, HALF - width / 2.0, slot.y - size * 0.36 - height / 2.0, width, height, alpha(face, 0.82));
        }
        glance_line(&mut p, fonts, slot, &said, coloured);
    }
    if glance.dim {
        paint::warm_dim(&mut p, glance.dim_level.unwrap_or(paint::DIM_LEVEL));
    }
    p
}

/// The Nest's face while a battery charges, and a battery's colours:
/// charging, very low, low.
const BATTERY_CHARGING: [u8; 3] = [14, 140, 69];
const BATTERY_RING: [[u8; 3]; 3] = [[72, 214, 130], [255, 59, 59], [246, 136, 0]];
const BATTERY_WORDS: [[u8; 3]; 3] = [[123, 227, 164], [255, 122, 122], [255, 176, 82]];

/// The battery face without its line: the level round the ring in the
/// colour of how it's doing (green charging; amber, then red, as it runs
/// low), a notch where it stops charging, its picture and name, the level
/// large, and under it what it's doing. `filled`: on the Nest's charging
/// green, where everything is white.
fn battery_face(p: &mut Canvas, fonts: &Fonts, battery: Option<&model::Battery>, filled: bool) {
    let Some(battery) = battery else {
        glance_label(p, fonts, 150.0, "Battery");
        paint::text(p, fonts.digits(), 170.0, HALF, NUMBER_BASELINE, "–", 0.0, WHITE);
        paint::text(p, fonts.regular(), 26.0, HALF, 352.0, "No battery chosen", 0.0, GLANCE_QUIET);
        return;
    };
    let mood = battery.mood();
    let tone = match mood {
        model::Mood::Charging => Some(0),
        model::Mood::Critical => Some(1),
        model::Mood::Low => Some(2),
        model::Mood::Calm => None,
    };
    let calm = if paint::fledge() { GLANCE_QUIET } else { WHITE };
    let ring = if filled { WHITE } else { tone.map_or(calm, |i| BATTERY_RING[i]) };
    let words = if filled { WHITE } else { tone.map_or(WHITE, |i| BATTERY_WORDS[i]) };
    if paint::fledge() {
        match mood {
            model::Mood::Charging => paint::glow(p, BATTERY_RING[0], HALF, 298.0, 280.0, 0.42),
            model::Mood::Critical => paint::glow(p, BATTERY_RING[1], HALF, 298.0, 280.0, 0.26),
            _ => {}
        }
    }
    let level = battery.level.map(|l| l.clamp(0.0, 100.0));
    let top = level.filter(|l| *l > 0.0).map(|l| paint::tick_at(l / 100.0));
    let limit = battery.limit.filter(|l| *l > 0.0 && *l < 100.0).map(|l| paint::tick_at(l / 100.0));
    let unlit = if filled { alpha(WHITE, 0.38) } else { colour(DIM) };
    // The Nest's look marks the limit with a long tick; Fledge's with a notch across its arc.
    let marks: Vec<usize> = limit.filter(|_| !paint::fledge()).into_iter().collect();
    ring_on(p, None, |i| if top.is_some_and(|t| i <= t) { colour(ring) } else { unlit }, &marks);
    if let Some(tick) = limit.filter(|_| paint::fledge()) {
        let a = paint::tick_angle(tick, paint::TICKS).to_radians();
        let at = |r: f32| (HALF + r * a.sin(), HALF - r * a.cos());
        let (inner, outer) = (at(199.0), at(225.0));
        paint::line(p, inner.0, inner.1, outer.0, outer.1, 3.0, alpha(WHITE, 0.85));
    }
    let picture = if battery.icon.is_empty() { "mdi:battery" } else { &battery.icon };
    icon(p, picture, "", HALF, 96.0, 52.0, if filled || tone.is_none() { WHITE } else { ring });
    glance_label(p, fonts, 156.0, &battery.label);
    match level {
        Some(level) => big_number_on(p, None, fonts, level.round() as f64, false, "%", WHITE),
        None => paint::text(p, fonts.digits(), 170.0, HALF, NUMBER_BASELINE, "–", 0.0, WHITE),
    }
    let doing = match battery.status.as_str() {
        "charging" => "mdi:flash",
        "plugged" => "mdi:power-plug",
        "discharging" => "mdi:lightning-bolt",
        _ => "mdi:battery",
    };
    if let Some(said) = said(battery.text.clone(), Some(doing), Some(words)) {
        glance_line(p, fonts, slot(350.0, Style::Main, 300.0), &said, filled);
    }
}

/// A small label over something large: the Nest's capitals, or as written
/// in Fledge's look.
fn glance_label(p: &mut Canvas, fonts: &Fonts, y: f32, text: &str) {
    if paint::fledge() {
        let text = paint::fit(fonts.medium(), 22.0, text, 1.0, 300.0);
        paint::text(p, fonts.medium(), 22.0, HALF, y, &text, 1.0, GLANCE_QUIET);
    } else {
        let text = paint::fit(fonts.medium(), 20.0, &text.to_uppercase(), 3.0, 300.0);
        paint::text(p, fonts.medium(), 20.0, HALF, y, &text, 3.0, GLANCE_QUIET);
    }
}

/// One line of the glance in its place.
fn glance_line(p: &mut Canvas, fonts: &Fonts, slot: Slot, said: &Said, coloured: bool) {
    match slot.style {
        Style::Quiet => {
            // On heating's or cooling's colour, a line's own colour would vanish.
            let c = if coloured { [255, 232, 218] } else { said.colour.unwrap_or(GLANCE_QUIET) };
            fitted(p, fonts.regular(), 30.0, 26.0, HALF, slot.y, &said.text, slot.room, c);
        }
        Style::Main => {
            let c = if coloured { WHITE } else { said.colour.unwrap_or(WHITE) };
            let icon_room = if said.icon.as_deref().is_some_and(|name| icons::lookup(name, "").is_some()) { 56.0 } else { 0.0 };
            // Smaller first (down to 24 px), then shortened.
            let room = slot.room - icon_room;
            let full = paint::measure(fonts.regular(), 34.0, &said.text, 0.0);
            let size = if full > room { (34.0 * room / full).max(24.0) } else { 34.0 };
            let text = paint::fit(fonts.regular(), size, &said.text, 0.0, room);
            let width = paint::measure(fonts.regular(), size, &text, 0.0);
            let left = HALF - (width + icon_room) / 2.0;
            if let (Some(name), true) = (&said.icon, icon_room > 0.0) {
                icon(p, name, "", left + 21.0, slot.y - 12.0, 44.0, c);
            }
            paint::text_at(p, fonts.regular(), size, left + icon_room, slot.y, &text, 0.0, c, 1.0);
        }
        Style::Pill => {
            let line = GlanceLine::new(fonts, &said.text, slot.room);
            match (line.scrolls(), paint::fledge()) {
                (false, true) => pill_words_at(p, fonts, slot.y, &said.text, said.colour.unwrap_or(FLEDGE_ON),
                                               said.icon.as_deref().filter(|name| icons::lookup(name, "").is_some()),
                                               line.size, line.room),
                (false, false) => paint::text(p, fonts.medium(), line.size, HALF, slot.y - 2.0, &said.text, 0.0,
                                              said.colour.unwrap_or(ACTION)),
                (true, true) => {
                    // The words scroll over it (glance_words).
                    let (pad, height) = (line.size * 0.95, line.size * 2.1);
                    paint::pill(p, HALF - line.room / 2.0 - pad, slot.y - height / 2.0, line.room + 2.0 * pad, height,
                                alpha(said.colour.unwrap_or(FLEDGE_ON), 0.18));
                }
                (true, false) => {}
            }
        }
    }
}

/// The time large, centred on `baseline`, a small AM / PM beside a
/// 12-hour time; smaller where it would be wider than `room`.
fn big_time(p: &mut Canvas, fonts: &Fonts, clock: &model::Clock, minutes: i64, size: f32, baseline: f32, room: f32) {
    let time = clock.time(minutes, false);
    let half = (clock.twelve() && clock.ampm).then(|| model::Clock::half(minutes.rem_euclid(1440) / 60));
    let measure = |size: f32| {
        let words = paint::measure(fonts.clock(), size, &time, -3.0);
        let small = half.map_or(0.0, |h| 10.0 + paint::measure(fonts.medium(), size * 0.2, h, 1.0));
        (words, small)
    };
    let (words, small) = measure(size);
    let size = if words + small > room { size * room / (words + small) } else { size };
    let (words, small) = measure(size);
    let left = HALF - (words + small) / 2.0;
    paint::text_at(p, fonts.clock(), size, left, baseline, &time, -3.0, WHITE, 1.0);
    if let Some(h) = half {
        paint::text_at(p, fonts.medium(), size * 0.2, left + words + 10.0, baseline, h, 1.0, GLANCE_QUIET, 1.0);
    }
}

/// A large value centred on the display, smaller where it would be wider
/// than `room`.
fn hero(p: &mut Canvas, fonts: &Fonts, value: &str, size: f32, baseline: f32, room: f32, c: [u8; 3]) {
    let font = fonts.digits();
    let words = paint::measure(font, size, value, -3.0);
    let size = if words > room { size * room / words } else { size };
    let words = paint::measure(font, size, value, -3.0);
    paint::text_at(p, font, size, HALF - words / 2.0, baseline, value, -3.0, c, 1.0);
}

/// `text` centred at `size`, smaller (down to `least`) and then shortened
/// to fit `room`.
#[allow(clippy::too_many_arguments)]
fn fitted(p: &mut Canvas, font: &ab_glyph::FontArc, size: f32, least: f32, cx: f32, y: f32, text: &str, room: f32, c: [u8; 3]) {
    let width = paint::measure(font, size, text, 0.0);
    let size = if width > room { (size * room / width).max(least) } else { size };
    let text = paint::fit(font, size, text, 0.0, room);
    paint::text(p, font, size, cx, y, &text, 0.0, c);
}

/// When the next event is: "16:30 · in 2 h", "All day · tomorrow".
fn event_when(event: &model::Event, at: &GlanceIn) -> String {
    let local = event.start + at.glance.offset;
    let days = local.div_euclid(86_400) - at.days();
    let day = match days {
        ..=0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        2..=6 => weekday(at.glance, local.div_euclid(86_400)),
        _ => glance_date(at.glance, local.div_euclid(86_400), true),
    };
    if event.all_day {
        return format!("All day · {day}");
    }
    let time = at.glance.clock.time(local.rem_euclid(86_400) / 60, true);
    let away = (event.start - at.now) as f64 / 60.0;
    let soon = match away.ceil() as i64 {
        ..=0 => "now".to_owned(),
        minutes @ 1..=59 => format!("in {minutes} min"),
        minutes if days == 0 => format!("in {} h", (minutes as f64 / 60.0).round() as i64),
        _ => day,
    };
    format!("{time} · {soon}")
}

/// The analog clock's minute ticks round the ring, the hours longer.
fn analog_ticks(p: &mut Canvas) {
    for i in 0..60 {
        let angle = i as f32 * std::f32::consts::PI / 30.0;
        let hour = i % 5 == 0;
        let (inner, outer) = (if hour { 194.0 } else { 210.0 }, 226.0);
        let (s, c) = (angle.sin(), angle.cos());
        paint::line(p, HALF + inner * s, HALF - inner * c, HALF + outer * s, HALF - outer * c,
                    if hour { 5.0 } else { 2.5 }, colour(if hour { WHITE } else { DIM }));
    }
}

/// The analog clock's hands at `minutes` past midnight.
fn analog_hands(p: &mut Canvas, minutes: i64) {
    let (h, m) = ((minutes.rem_euclid(720)) as f32 / 60.0, minutes.rem_euclid(60) as f32);
    let hand = |p: &mut Canvas, angle: f32, length: f32, width: f32| {
        let (s, c) = (angle.sin(), angle.cos());
        paint::line(p, HALF - 18.0 * s, HALF + 18.0 * c, HALF + length * s, HALF - length * c, width, colour(WHITE));
    };
    hand(p, h * std::f32::consts::PI / 6.0, 112.0, 11.0);
    hand(p, m * std::f32::consts::PI / 30.0, 170.0, 6.0);
    paint::circle(p, HALF, HALF, 9.0, colour(FLEDGE_ON));
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September",
                            "October", "November", "December"];

/// The day of the week (0 Sunday), the day of the month and the month
/// (0 January) of `days` since 1970.
fn civil(days: i64) -> (usize, i64, usize) {
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    ((days + 4).rem_euclid(7) as usize, day, (month - 1) as usize)
}

/// The day of the week in the glance's words.
fn weekday(glance: &Glance, days: i64) -> String {
    let (weekday, _, _) = civil(days);
    glance.words().map_or(DAYS[weekday].to_owned(), |words| words.days[weekday].clone())
}

/// The date as the glance says it: "Sunday 8 November" ("Sun 8 Nov"
/// `short`), the month first where Home Assistant asks ("Sunday, November
/// 8"), in its words where it sent them.
fn glance_date(glance: &Glance, days: i64, short: bool) -> String {
    let (weekday, day, month) = civil(days);
    let (day_name, month_name) = match (glance.words(), short) {
        (Some(words), false) => (words.days[weekday].clone(), words.months[month].clone()),
        (Some(words), true) => (words.days_short[weekday].clone(), words.months_short[month].clone()),
        (None, false) => (DAYS[weekday].to_owned(), MONTHS[month].to_owned()),
        (None, true) => (DAYS[weekday][..3].to_owned(), MONTHS[month][..3].to_owned()),
    };
    if glance.month_first() {
        format!("{day_name}, {month_name} {day}")
    } else {
        format!("{day_name} {day} {month_name}")
    }
}

/// An MDI weather icon for Home Assistant's condition.
fn weather_icon(condition: &str) -> &'static str {
    match condition {
        "sunny" => "weather-sunny",
        "clear-night" => "weather-night",
        "partlycloudy" => "weather-partly-cloudy",
        "rainy" => "weather-rainy",
        "pouring" => "weather-pouring",
        "snowy" | "snowy-rainy" => "weather-snowy",
        "fog" => "weather-fog",
        "windy" | "windy-variant" => "weather-windy",
        "lightning" | "lightning-rainy" => "weather-lightning",
        "hail" => "weather-hail",
        _ => "weather-cloudy",
    }
}

/// Home Assistant's weather condition in words.
fn condition_name(condition: &str) -> String {
    match condition {
        "partlycloudy" => "Partly cloudy".to_owned(),
        "clear-night" => "Clear".to_owned(),
        "snowy-rainy" => "Sleet".to_owned(),
        "lightning-rainy" => "Thunderstorms".to_owned(),
        "windy-variant" => "Windy".to_owned(),
        "exceptional" => String::new(),
        other => capitalise(&other.replace('-', " ")),
    }
}

/// What's on among the cards: lights, switches and fans, each counted by
/// what Home Assistant calls them (a smart plug is a switch, not a light),
/// and what's playing by its title.
fn whats_on(cards: &[Card]) -> String {
    // (one, many, how many on), in the order they're said.
    let mut kinds = [("light", "lights", 0usize), ("switch", "switches", 0), ("fan", "fans", 0)];
    for card in cards {
        let Card::Lights(card) = card else { continue };
        for light in card.lights.iter().filter(|l| l.on) {
            let kind = match light.id.split_once('.').map_or("", |(domain, _)| domain) {
                "light" => 0,
                "fan" => 2,
                _ => 1, // switch, input_boolean and the like
            };
            kinds[kind].2 += 1;
        }
    }
    let playing = cards.iter().find_map(|c| match c {
        Card::Media(m) if m.state == "playing" => Some(if m.title.is_empty() { m.name.clone() } else { m.title.clone() }),
        _ => None,
    });
    let mut parts: Vec<String> = kinds.iter().filter(|(_, _, n)| *n > 0)
        .map(|(one, many, n)| format!("{n} {} on", if *n == 1 { one } else { many })).collect();
    if let Some(title) = playing {
        parts.push(format!("playing {title}"));
    }
    if parts.is_empty() { "All quiet".to_owned() } else { parts.join(" · ") }
}

/// A thermostat preset's icon in its menu.
pub fn preset_icon(preset: &str) -> &'static str {
    match preset {
        "eco" => "mdi:leaf",
        "away" | "vacation" => "mdi:airplane",
        "home" => "mdi:home",
        "sleep" | "night" => "mdi:sleep",
        "boost" | "comfort" => "mdi:fire",
        _ => "mdi:thermometer",
    }
}

/// Covers' accent: a cool blue.
const COVER: [u8; 3] = [120, 170, 255];

fn options_card(fonts: &Fonts, card: &OptionsCard) -> Canvas {
    let mut p = paint::face(BLACK);
    let found = icons::lookup(&card.icon, &card.id).or_else(|| icons::lookup("mdi:format-list-bulleted", ""));
    if let Some(found) = found {
        paint::icon(&mut p, found, HALF, 150.0, 84.0, colour(WHITE));
    }
    label(&mut p, fonts, 232.0, &card.name, GREY);
    let current = paint::fit(fonts.regular(), 40.0, card.current.as_deref().unwrap_or("—"), 0.0, 330.0);
    paint::text(&mut p, fonts.regular(), 40.0, HALF, 292.0, &current, 0.0, WHITE);
    p
}

/// A select's options, the current one marked, then Back.
/// The off thermostat's prompt: its question where a list's first row
/// would be, and its choices under it.
const PROMPT_ROWS: usize = 4;

fn mode_prompt(fonts: &Fonts, nest: &Nest, selected: usize, layer: Layer) -> Canvas {
    let mut p = list_backdrop(layer, Some(selected));
    for (i, line) in crate::ui::machine::MODE_PROMPT.split('\n').enumerate() {
        let line = paint::fit(fonts.regular(), 22.0, line, 0.0, 330.0);
        paint::text(&mut p, fonts.regular(), 22.0, HALF, 118.0 + 30.0 * i as f32, &line, 0.0, WHITE);
    }
    let cools = nest.on_mode() == "cool";
    let rows = [
        (if cools { "mdi:snowflake" } else { "mdi:fire" }, if cools { "Cool" } else { "Heat" }),
        ("mdi:power", "Off"),
        ("mdi:window-close", "Cancel"),
    ];
    for (i, (icon_name, name)) in rows.iter().enumerate() {
        let row = i + 1;
        let y = LIST_TOP + row as f32 * ROW_PITCH;
        let (x, _, h) = PILL;
        let chosen = layer.chosen(row == selected);
        icon(&mut p, icon_name, "prompt", x + 34.0, y + h / 2.0, 28.0, if chosen { WHITE } else { GREY });
        paint::text_at(&mut p, fonts.regular(), 26.0, x + 62.0, y + 37.0, name, 0.0, WHITE, 1.0);
    }
    p
}

fn option_list(fonts: &Fonts, card: &OptionsCard, selected: usize, layer: Layer) -> Canvas {
    let rows = card.options.len() + 1;
    let first = first_row(selected, rows);
    let mut p = list_backdrop(layer, selected.checked_sub(first));
    label(&mut p, fonts, LIST_TITLE, &card.name, GREY);
    for (slot, row) in (first..rows.min(first + ROWS)).enumerate() {
        let y = LIST_TOP + slot as f32 * ROW_PITCH;
        let (x, w, h) = PILL;
        let chosen = layer.chosen(row == selected);
        let (name, now) = match card.options.get(row) {
            Some(option) => (option.as_str(), card.current.as_deref() == Some(option.as_str())),
            None => ("Back", false),
        };
        if row == card.options.len() {
            icon(&mut p, "mdi:arrow-left", "back", x + 34.0, y + h / 2.0, 28.0, if chosen { WHITE } else { GREY });
        }
        let value = if now { "Now" } else { "" };
        let value_width = paint::measure(fonts.regular(), 24.0, value, 0.0);
        let name = paint::fit(fonts.regular(), 26.0, name, 0.0, w - 90.0 - value_width - 16.0);
        paint::text_at(&mut p, fonts.regular(), 26.0, x + 62.0, y + 37.0, &name, 0.0, WHITE, 1.0);
        if now {
            paint::text_at(&mut p, fonts.regular(), 24.0, x + w - 22.0 - value_width, y + 36.0, value, 0.0,
                           if chosen { WHITE } else { ACTION }, 1.0);
        }
    }
    p
}

/// A cover: as its own screen shows it, how open on the ring if it knows,
/// open or closed in words if it doesn't, so opening it moves nothing.
fn cover_card(fonts: &Fonts, card: &CoverCard) -> Canvas {
    let open = card.state == "open" || card.state == "opening";
    let mut p = cover_still(fonts, card, false, None);
    let value = card.position.unwrap_or(if open { 100 } else { 0 });
    cover_moving(&mut p, None, fonts, card, value, None);
    p
}

/// `opened`: its own screen, which says what a hold does; among the cards,
/// no hint.
fn cover_still(fonts: &Fonts, card: &CoverCard, opened: bool, parked: Parked) -> Canvas {
    let mut p = paint::face(BLACK);
    let open = card.state == "open" || card.state == "opening";
    let name = if open { "mdi:window-shutter-open" } else { "mdi:window-shutter" };
    let found = icons::lookup(&card.icon, &card.id).or_else(|| icons::lookup(name, ""));
    if let Some(found) = found {
        paint::icon(&mut p, found, HALF, 98.0, 40.0, colour(COVER));
    }
    label(&mut p, fonts, 150.0, &card.name, COVER);
    if !card.positionable {
        let words = match card.state.as_str() {
            "opening" => "Opening".to_owned(),
            "closing" => "Closing".to_owned(),
            state => capitalise(state),
        };
        paint::text(&mut p, fonts.regular(), 80.0, HALF, NUMBER_BASELINE - 12.0, &words, 0.0, WHITE);
    }
    if parked.as_ref().is_some_and(|edge| parked_words(&mut p, fonts, edge, PARKED_WORDS, GREY)) {
    } else if opened {
        let open = card.state == "open" || card.state == "opening";
        line(&mut p, fonts, 366.0, if open { "Hold to close" } else { "Hold to open" }, GREY);
    }
    p
}

fn cover_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, card: &CoverCard, value: u8,
                sweep: Option<f32>) {
    let top = paint::tick_at(sweep.unwrap_or(along(value as f32, card.position_ends().map(f32::from))));
    ring_on(p, live.as_deref_mut(), |i| if i <= top && value > 0 { colour(COVER) } else { colour(DIM) }, &[top]);
    if card.positionable {
        big_number_on(p, live, fonts, value as f64, false, "%", number_tint(card.position_ring(), value as i32));
    }
}

pub fn capitalise(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// An alarm's colours: disarmed, Fledge's eco green; a delay running,
/// amber; armed, a violet clear of cooling's blue; going off, red.
const ALARM_ARMED: [u8; 3] = [154, 134, 255];
const ALARM_RED: [u8; 3] = [255, 59, 59];

/// An armed mode's icon.
pub fn alarm_mode_icon(mode: &str) -> &'static str {
    match mode {
        "home" => "mdi:shield-home",
        "night" => "mdi:shield-moon",
        "vacation" => "mdi:airplane",
        "custom_bypass" => "mdi:shield-star",
        _ => "mdi:shield-lock",
    }
}

/// An alarm screen's face: Fledge's glowing with `c`; the Nest's black,
/// but solid red for an alarm going off, its ring and words white on it.
fn alarm_face(c: [u8; 3]) -> Canvas {
    paint::face(if paint::fledge() || c == ALARM_RED { c } else { BLACK })
}

/// How bright something going off is at `now` (0..1), pulsing.
fn alarm_pulse(now: f64) -> f32 {
    0.5 + 0.5 * (now * std::f64::consts::TAU / 1.2).sin() as f32
}

/// A delay running on the ring: with its length, what's left of it lit
/// amber; without, a sweep going round (at `now`), or all of it dimly on
/// a card at rest. `mark`: a white tick too.
fn alarm_delay_ring(p: &mut Canvas, card: &AlarmCard, now: Option<f64>, mark: Option<usize>) {
    let marks: Vec<usize> = mark.into_iter().collect();
    match (now, now.and_then(|now| card.left(now))) {
        (_, Some((left, _))) => {
            let lit = left * (TICKS - 1) as f32;
            paint::ring(p, |i| if i as f32 <= lit { colour(AMBER) } else { colour(DIM) }, &marks);
        }
        (Some(now), None) => {
            let head = (now / 1.6).fract() as f32 * (TICKS + 30) as f32 - 15.0;
            paint::ring(p, |i| {
                let behind = head - i as f32;
                if (0.0..30.0).contains(&behind) { alpha(AMBER, 1.0 - behind / 30.0 * 0.8) } else { alpha(AMBER, 0.2) }
            }, &marks);
        }
        (None, None) => paint::ring(p, |_| colour(AMBER), &marks),
    }
}

/// The note under an alarm's screen, if there's one.
fn alarm_note(p: &mut Canvas, fonts: &Fonts, note: Option<&str>, y: f32) -> bool {
    let Some(note) = note else { return false };
    let note = paint::fit(fonts.medium(), 21.0, note, 0.0, 320.0);
    paint::text(p, fonts.medium(), 21.0, HALF, y, &note, 0.0, ALARM_RED);
    true
}

/// An alarm's state at a glance: its colour (the whole ring lit in it, and
/// Fledge's face glowing with it), its icon, the word for it, and what's
/// under the word.
fn alarm_state(card: &AlarmCard) -> ([u8; 3], &'static str, &'static str, String) {
    match (card.state.as_str(), card.armed()) {
        (_, Some(mode)) => (ALARM_ARMED, alarm_mode_icon(mode), "Armed", card.mode_name(mode)),
        ("disarmed", _) => (ECO, "mdi:shield-lock-open-outline", "Disarmed", String::new()),
        ("arming", _) => (AMBER, "mdi:home-export-outline", "Arming", "Leave now".into()),
        ("pending", _) => (AMBER, "mdi:door-open", "Entry delay",
                           if card.wants_digits() { "Enter the code".into() } else { String::new() }),
        ("triggered", _) => (ALARM_RED, "mdi:alarm-light", "Alarm", card.sensors.first().cloned().unwrap_or_default()),
        ("disarming", _) => (AMBER, "mdi:shield-lock-open-outline", "Disarming", String::new()),
        _ => (DIM, "mdi:shield-lock-open-outline", "Unavailable", String::new()),
    }
}

/// An alarm as its card shows it, or opened (`now`: its countdown and its
/// going off move). Its state is the large word, in its colour, with the
/// whole ring lit in the same; the alarm's name small above it. Opened
/// while a delay runs, the seconds left take the word's place.
fn alarm_screen(fonts: &Fonts, card: &AlarmCard, now: Option<f64>, note: Option<&str>) -> Canvas {
    let name = if card.name.is_empty() { "Alarm" } else { card.name.as_str() };
    let (accent, icon_name, word, under) = alarm_state(card);
    let triggered = card.state == "triggered";
    let mut p = alarm_face(if accent == DIM { BLACK } else { accent });
    let timed = matches!(card.state.as_str(), "arming" | "pending");
    if timed {
        alarm_delay_ring(&mut p, card, now, None);
    } else if triggered {
        let glow = now.map_or(1.0, alarm_pulse);
        let c = if paint::fledge() { ALARM_RED } else { WHITE };
        paint::ring(&mut p, |_| alpha(c, 0.45 + 0.55 * glow), &[]);
    } else {
        paint::ring(&mut p, |_| colour(accent), &[]);
    }
    // On the Nest's solid red, everything white.
    let (accent, words) = if triggered && !paint::fledge() { (WHITE, WHITE) } else { (accent, GREY) };
    let hint = match card.state.as_str() {
        "arming" => "Press to cancel",
        "pending" | "triggered" if card.wants_digits() => "Press to enter the code",
        "pending" | "triggered" => "Press to disarm",
        "disarmed" if !card.modes.is_empty() => "Press to arm",
        _ if card.armed().is_some() => "Press to disarm",
        _ => "",
    };
    let seconds = now.and_then(|now| card.left(now)).filter(|_| timed).map(|(_, s)| s);
    match seconds {
        Some(seconds) => {
            pill_words(&mut p, fonts, 150.0, word, accent, Some(icon_name));
            paint::text(&mut p, fonts.digits(), 140.0, HALF, 300.0, &seconds.to_string(), -4.0, WHITE);
            if !alarm_note(&mut p, fonts, note, 346.0) {
                line(&mut p, fonts, 346.0, hint, WHITE);
            }
        }
        None => {
            icon(&mut p, icon_name, &card.id, HALF, 136.0, 76.0, if triggered { WHITE } else { accent });
            let label = paint::fit(fonts.medium(), 24.0, name, 0.0, 300.0);
            paint::text(&mut p, fonts.medium(), 24.0, HALF, 216.0, &label, 0.0, words);
            let big = paint::fit(fonts.medium(), 52.0, word, 0.0, 340.0);
            paint::text(&mut p, fonts.medium(), 52.0, HALF, 284.0, &big, 0.0, if triggered { WHITE } else { accent });
            if !under.is_empty() {
                let under = paint::fit(fonts.regular(), 24.0, &under, 0.0, 320.0);
                paint::text(&mut p, fonts.regular(), 24.0, HALF, 326.0, &under, 0.0, WHITE);
            }
            // How to use it once it's open; among the cards, what it is.
            if !alarm_note(&mut p, fonts, note, 372.0) && now.is_some() {
                line(&mut p, fonts, 372.0, hint, words);
            }
        }
    }
    p
}

/// Where the code pad's place `k` of `n` sits on the ring: its tick, and
/// the point its digit is drawn at.
fn pad_place(k: usize, n: usize) -> (usize, f32, f32) {
    let (first, last) = (8.0, (TICKS - 9) as f32);
    let tick = (first + (last - first) * k as f32 / (n - 1).max(1) as f32).round() as usize;
    let angle = paint::tick_angle(tick, TICKS).to_radians();
    // Inside Fledge's arc, or the Nest's ticks.
    let radius = if paint::fledge() { 172.0 } else { 150.0 };
    (tick, HALF + radius * angle.sin(), HALF - radius * angle.cos())
}

/// An alarm's code pad: the digits round the ring, the one under the
/// marker large in the middle, the digits entered as dots.
#[allow(clippy::too_many_arguments)]
fn alarm_pad(fonts: &Fonts, card: &AlarmCard, arm: Option<&AlarmMode>, at: usize, entered: usize, waiting: bool,
             note: Option<&str>, locked: Option<u64>, now: f64) -> Canvas {
    let pending = card.state == "pending";
    let triggered = card.state == "triggered";
    let accent = if triggered { ALARM_RED } else if pending { AMBER } else { ALARM_ARMED };
    let mut p = alarm_face(if triggered { ALARM_RED } else if pending { AMBER } else { BLACK });
    let n = super::machine::DIGITS.len() + usize::from(card.code_length.is_none());
    let at = at.min(n - 1);
    if let Some(seconds) = locked {
        paint::ring(&mut p, |_| colour(DIM), &[]);
        icon(&mut p, "mdi:lock", "lock", HALF, 200.0, 78.0, GREY);
        paint::text(&mut p, fonts.medium(), 26.0, HALF, 290.0, &format!("Try again in {seconds} s"), 0.0, WHITE);
        line(&mut p, fonts, 326.0, "Too many tries", GREY);
        return p;
    }
    let (mark, _, _) = pad_place(at, n);
    if pending {
        alarm_delay_ring(&mut p, card, Some(now), Some(mark));
    } else {
        let base = if triggered { alpha(ALARM_RED, 0.3 + 0.4 * alarm_pulse(now)) } else { colour(DIM) };
        paint::ring(&mut p, |i| if i.abs_diff(mark) <= 3 { colour(accent) } else { base }, &[mark]);
    }
    for k in 0..n {
        let (_, x, y) = pad_place(k, n);
        let chosen = k == at;
        match super::machine::DIGITS.get(k) {
            Some(digit) => {
                let (font, size) = if chosen { (fonts.medium(), 34.0) } else { (fonts.regular(), 26.0) };
                paint::text(&mut p, font, size, x, y + size * 0.36, &digit.to_string(), 0.0, if chosen { WHITE } else { GREY });
            }
            None => icon(&mut p, "mdi:check", "check", x, y, if chosen { 36.0 } else { 28.0 }, if chosen { WHITE } else { GREY }),
        }
    }
    let title = match (arm, pending) {
        (Some(mode), _) => card.mode_name(&mode.mode),
        (None, true) => match card.left(now) {
            Some((_, seconds)) => format!("Disarm · {seconds} s"),
            None => "Disarm now".to_owned(),
        },
        (None, false) => "Disarm".to_owned(),
    };
    let title_icon = arm.map_or("mdi:shield-lock-open-outline", |mode| alarm_mode_icon(&mode.mode));
    pill_words(&mut p, fonts, 150.0, &title, accent, Some(title_icon));
    match super::machine::DIGITS.get(at) {
        Some(digit) if !waiting => {
            paint::text(&mut p, fonts.digits(), 120.0, HALF, 288.0, &digit.to_string(), 0.0, WHITE);
        }
        Some(_) => {}
        None => icon(&mut p, "mdi:check", "check", HALF, 250.0, 86.0, accent),
    }
    // The digits entered: as many dots as the code has, if Fledge knows,
    // else one more than entered.
    let shown = match card.code_length {
        Some(length) => (length as usize).max(entered),
        None => entered + usize::from(!waiting),
    };
    if shown == 0 || (entered == 0 && card.code_length.is_none() && !waiting) {
        line(&mut p, fonts, 336.0, "Enter the code", GREY);
    } else {
        let gap = if shown > 6 { 20.0 } else { 26.0 };
        let left = HALF - (shown - 1) as f32 * gap / 2.0;
        for k in 0..shown {
            let c = if k < entered { colour(WHITE) } else { alpha(GREY, 0.35) };
            paint::circle(&mut p, left + k as f32 * gap, 330.0, 7.0, c);
        }
    }
    if waiting {
        line(&mut p, fonts, 404.0, "Checking…", GREY);
    } else if !alarm_note(&mut p, fonts, note, 404.0) {
        let hint = match (super::machine::DIGITS.get(at), entered) {
            (None, _) => if arm.is_some() { "Press to arm" } else { "Press to disarm" },
            (Some(_), 0) => "Press to enter · hold to go back",
            (Some(_), _) => "Hold to rub out",
        };
        line(&mut p, fonts, 404.0, hint, GREY);
    }
    p
}

/// Where an alarm's option `k` of `n` (Back, then its modes) sits: round
/// the top of the ring, centred, 20 ticks apart.
fn mode_place(k: usize, n: usize) -> (usize, f32, f32) {
    let middle = (TICKS - 1) as f32 / 2.0;
    let tick = (middle + (k as f32 - (n - 1) as f32 / 2.0) * 20.0).round().clamp(4.0, (TICKS - 5) as f32) as usize;
    let angle = paint::tick_angle(tick, TICKS).to_radians();
    let radius = if paint::fledge() { 172.0 } else { 150.0 };
    (tick, HALF + radius * angle.sin(), HALF - radius * angle.cos())
}

/// One option of a device's: its icon, its name, and what a press on it does.
pub struct Opt<'a> {
    pub icon: &'a str,
    pub name: &'a str,
    pub under: &'a str,
}

/// A device's options as the code pad's digits are: round the top of the
/// ring, the marker under the one chosen, and it large in the middle in
/// the device's colour (`accent`), with what a press does. Back (an option
/// like any other) is white. The device's name sits low, in the ring's gap.
/// Nothing shows through from behind.
pub fn options(fonts: &Fonts, title: &str, items: &[Opt], selected: usize, accent: [u8; 3]) -> Canvas {
    let n = items.len().max(1);
    let selected = selected.min(n - 1);
    let mut p = paint::face(BLACK);
    let Some(chosen) = items.get(selected) else { return p };
    let back = chosen.icon == "mdi:arrow-left";
    let tint = if back { WHITE } else { accent };
    let (mark, _, _) = mode_place(selected, n);
    paint::ring(&mut p, |i| if i.abs_diff(mark) <= 3 { colour(if back { GREY } else { accent }) } else { colour(DIM) }, &[mark]);
    for (k, item) in items.iter().enumerate() {
        let (_, x, y) = mode_place(k, n);
        let on = k == selected;
        icon(&mut p, item.icon, "option", x, y, if on { 40.0 } else { 32.0 }, if on { WHITE } else { GREY });
    }
    icon(&mut p, chosen.icon, "option", HALF, 212.0, 80.0, tint);
    let name = paint::fit(fonts.medium(), 42.0, chosen.name, 0.0, 320.0);
    paint::text(&mut p, fonts.medium(), 42.0, HALF, 300.0, &name, 0.0, WHITE);
    line(&mut p, fonts, 340.0, chosen.under, GREY);
    let label = paint::fit(fonts.medium(), 20.0, title, 0.0, 210.0);
    paint::text(&mut p, fonts.medium(), 20.0, HALF, 404.0, &label, 0.0, GREY);
    p
}

/// A device's colour in its options: a light's own, a speaker's teal,
/// heating's orange (cooling's blue), a cover's blue, a scene's gold.
pub fn accent(screen: &Screen) -> [u8; 3] {
    match screen {
        Screen::LightDim { light, .. } => light.colour(),
        Screen::Volume { .. } | Screen::MediaCard { .. } => MEDIA,
        Screen::NestSet { nest, .. } if nest.card.mode == "cool" => COOL,
        Screen::NestSet { .. } => HEAT,
        Screen::CoverSet { .. } => COVER,
        Screen::ActionCard { .. } => ACTION,
        Screen::AlarmView { .. } => ALARM_ARMED,
        _ => WHITE,
    }
}

/// An alarm's options, as its state offers them: Back, then its modes to
/// arm to (violet), Disarm (green) or Cancel (amber).
fn alarm_modes(fonts: &Fonts, card: &AlarmCard, selected: usize) -> Canvas {
    use super::machine::{alarm_options, AlarmOption};
    let choices = alarm_options(card);
    let names: Vec<String> = choices.iter().map(|o| match o {
        AlarmOption::Arm(i) => card.modes.get(*i as usize).map_or(String::new(), |m| card.mode_name(&m.mode)),
        _ => String::new(),
    }).collect();
    let code = card.wants_digits();
    let items: Vec<Opt> = choices.iter().zip(&names).map(|(option, name)| match option {
        AlarmOption::Back => Opt { icon: "mdi:arrow-left", name: "Back", under: "To the cards" },
        AlarmOption::Arm(i) => Opt {
            icon: card.modes.get(*i as usize).map_or("mdi:shield-lock", |m| alarm_mode_icon(&m.mode)), name,
            under: if code { "Press, then the code" } else { "Press to arm" },
        },
        AlarmOption::Disarm => Opt { icon: "mdi:shield-lock-open", name: "Disarm",
                                     under: if code { "Press, then the code" } else { "Press to disarm" } },
        AlarmOption::Cancel => Opt { icon: "mdi:window-close", name: "Cancel", under: "Press to stop it arming" },
    }).collect();
    let accent = match choices.get(selected.min(choices.len() - 1)) {
        Some(AlarmOption::Disarm) => ECO,
        Some(AlarmOption::Cancel) => AMBER,
        _ => ALARM_ARMED,
    };
    let title = if card.name.is_empty() { "Alarm" } else { card.name.as_str() };
    options(fonts, title, &items, selected, accent)
}

/// Arming refused with a door or window open: what's open, and arming
/// anyway on a press.
fn alarm_open(fonts: &Fonts, card: &AlarmCard, arm: &AlarmMode, sensors: &[String]) -> Canvas {
    let mut p = alarm_face(AMBER);
    paint::ring(&mut p, |_| alpha(AMBER, 0.4), &[]);
    pill_words(&mut p, fonts, 104.0, &card.mode_name(&arm.mode), AMBER, Some(alarm_mode_icon(&arm.mode)));
    icon(&mut p, "mdi:door-open", &card.id, HALF, 190.0, 76.0, WHITE);
    let what = match sensors {
        [one] => format!("{one} is open"),
        [] => "Something is open".to_owned(),
        many => format!("{} are open", many.len()),
    };
    let what = paint::fit(fonts.medium(), 30.0, &what, 0.0, 340.0);
    paint::text(&mut p, fonts.medium(), 30.0, HALF, 272.0, &what, 0.0, WHITE);
    line(&mut p, fonts, 314.0, "Press to arm anyway", WHITE);
    line(&mut p, fonts, 348.0, "Hold to leave it disarmed", GREY);
    p
}

#[cfg(test)]
mod tests {
    use super::super::model::{Cards, Event, Glance, Line, Nest};
    use super::{event_when, glance_date, line_said, GlanceIn, Said};

    /// Sunday 8 November 2026, 21:13 UTC.
    const NOW: i64 = 1_794_172_380;

    fn glance(json: &str) -> Glance {
        serde_json::from_str(json).unwrap()
    }

    fn nest(extra: serde_json::Value) -> Nest {
        let mut s = serde_json::json!({"mode": "heat", "action": "idle", "target": 21.0, "current_temperature": 19.5,
            "current_humidity": 48, "preset": "none", "temperature_scale": "C", "can_heat": true});
        for (k, v) in extra.as_object().unwrap() {
            s[k] = v.clone();
        }
        Nest::from_state(s.as_object().unwrap())
    }

    fn says(line: Line, glance: &Glance, nest: Option<&Nest>) -> Option<String> {
        line_said(&line, &GlanceIn { glance, cards: &[], nest, now: NOW }).map(|Said { text, .. }| text)
    }

    #[test]
    fn the_glance_s_own_lines_say_the_date_time_and_thermostat() {
        let twelve = glance(r#"{"clock":{"hours":12}}"#);
        assert_eq!(says(Line::kind("date"), &twelve, None).as_deref(), Some("Sunday 8 November"));
        assert_eq!(says(Line::kind("date_short"), &twelve, None).as_deref(), Some("Sun 8 Nov"));
        assert_eq!(says(Line::kind("time"), &twelve, None).as_deref(), Some("9:13 PM"));
        assert_eq!(says(Line::kind("time"), &glance("{}"), None).as_deref(), Some("21:13"));
        let idle = nest(serde_json::json!({}));
        assert_eq!(says(Line::kind("inside"), &twelve, Some(&idle)).as_deref(), Some("Inside 19.5° · 48%"));
        assert_eq!(says(Line::kind("heating"), &twelve, Some(&idle)).as_deref(), Some("Set to 21°"));
        let heating = nest(serde_json::json!({"action": "heating", "target": 21.5, "time_to_target": "2026-11-08T21:35:00Z"}));
        assert_eq!(says(Line::kind("heating"), &twelve, Some(&heating)).as_deref(), Some("Heating to 21.5° · 22 min"));
        assert_eq!(says(Line::kind("inside"), &twelve, None), None, "no thermostat: nothing to say");
        assert_eq!(says(Line::kind("weather"), &twelve, None), None, "no weather chosen");
        assert_eq!(says(Line::kind("no_such_kind"), &twelve, None), None, "a newer kind: left empty");
    }

    #[test]
    fn home_assistant_s_lines_come_as_they_are() {
        let line = Line { icon: Some("mdi:calendar".into()), text: Some("16:30 Dentist".into()), kind: None };
        let g = glance("{}");
        let said = line_said(&line, &GlanceIn { glance: &g, cards: &[], nest: None, now: NOW }).unwrap();
        assert_eq!((said.text.as_str(), said.icon.as_deref()), ("16:30 Dentist", Some("mdi:calendar")));
        assert_eq!(says(Line { text: Some("  ".into()), ..Line::default() }, &g, None), None, "blank: nothing");
    }

    #[test]
    fn the_next_event_says_when() {
        let g = glance(r#"{"offset":3600}"#);
        let at = GlanceIn { glance: &g, cards: &[], nest: None, now: NOW };
        let event = |start: i64, all_day: bool| event_when(&Event { title: "x".into(), start, all_day }, &at);
        assert_eq!(event(NOW + 25 * 60, false), "22:38 · in 25 min");
        assert_eq!(event(NOW + 90 * 60, false), "23:43 · in 2 h");
        assert_eq!(event(NOW + 2 * 3600, false), "00:13 · tomorrow", "past midnight");
        assert_eq!(event(NOW - 60, false), "22:12 · now");
        assert_eq!(event(NOW + 12 * 3600, false), "10:13 · tomorrow");
        assert_eq!(event(NOW + 3 * 86_400, true), "All day · Wednesday");
        assert_eq!(glance_date(&glance("{}"), 20_400, true), "Sat 8 Nov");
    }

    #[test]
    fn the_date_in_home_assistant_s_words_and_order() {
        let french = glance(r#"{"words":{"days":["dimanche","lundi","mardi","mercredi","jeudi","vendredi","samedi"],
            "days_short":["dim.","lun.","mar.","mer.","jeu.","ven.","sam."],
            "months":["janvier","février","mars","avril","mai","juin","juillet","août","septembre","octobre","novembre","décembre"],
            "months_short":["janv.","févr.","mars","avr.","mai","juin","juil.","août","sept.","oct.","nov.","déc."]}}"#);
        assert_eq!(glance_date(&french, 20_765, false), "dimanche 8 novembre");
        assert_eq!(glance_date(&french, 20_765, true), "dim. 8 nov.");
        let american = glance(r#"{"date_order":"mdy"}"#);
        assert_eq!(glance_date(&american, 20_765, false), "Sunday, November 8");
        assert_eq!(glance_date(&american, 20_765, true), "Sun, Nov 8");
        let short = glance(r#"{"words":{"days":["a"],"days_short":[],"months":[],"months_short":[]}}"#);
        assert_eq!(glance_date(&short, 20_765, false), "Sunday 8 November", "not all there: English");
    }

    #[test]
    fn a_line_with_nothing_to_say_takes_no_place() {
        let g = glance(r#"{"lines":[{"text":""},{"kind":"weather"},{"kind":"date"},{"text":"Bins tomorrow"}]}"#);
        let at = GlanceIn { glance: &g, cards: &[], nest: None, now: NOW };
        let said: Vec<Option<String>> = super::glance_lines(&at).into_iter().map(|(_, said)| said.map(|s| s.text)).collect();
        assert_eq!(said, [Some("Sunday 8 November".to_owned()), None, None],
                   "the empty line and the weather nobody chose gone, the date first; three places only");
    }

    #[test]
    fn the_analog_clock_s_line_keeps_clear_of_the_hands() {
        use super::analog_line;
        assert_eq!(analog_line(10 * 60 + 10), 350.0, "ten past ten: under the middle");
        assert_eq!(analog_line(6 * 60 + 35), 152.0, "both hands down: above it");
        assert_eq!(analog_line(6 * 60), 350.0, "one up, one down: where it always is");
        assert_eq!(analog_line(3 * 60 + 30), 152.0, "the minute hand down");
    }

    #[test]
    fn the_glance_without_lines_is_the_glance_as_it_was() {
        let cards = Cards::parse(br#"{"cards":[],"glance":{"offset":0}}"#).unwrap();
        let lines = cards.glance.unwrap().lines();
        assert_eq!(lines, ["date", "weather", "whats_on"].map(Line::kind).to_vec());
        let analog = glance(r#"{"layout":"analog","lines":[{"kind":"date"},{"kind":"time"}]}"#);
        assert_eq!(analog.lines().len(), 1, "as many as its layout has room for");
        assert_eq!(glance(r#"{"layout":"hologram"}"#).layout.slots(), 3, "a layout from a newer Fledge: the clock");
    }

    #[test]
    fn a_dial_predicts_only_while_its_target_is_being_set() {
        let card = super::ClimateCard { mode: "heat".into(), action: "idle".into(), target: Some(21.0), current: Some(20.8),
                                        ..Default::default() };
        let nest = super::Nest::of_card(&card);
        assert_eq!(super::setting_to(&nest, 21.0, false).card.action, "idle", "at rest: as it reports, as its card shows");
        assert_eq!(super::setting_to(&nest, 21.0, true).card.action, "heating", "the ring just turned: predicted");
        assert_eq!(super::setting_to(&nest, 22.0, false).card.action, "heating", "a target it hasn't taken yet");
    }

    #[test]
    fn an_off_thermostat_never_glows_whatever_its_action_says() {
        let card = |mode: &str| super::ClimateCard { mode: mode.into(), action: "heating".into(), ..Default::default() };
        assert_eq!(super::climate_colours(&card("heat")), (super::HEAT, true));
        assert_eq!(super::climate_colours(&card("off")), (super::BLACK, false));
    }

    #[test]
    fn a_hot_water_boost_s_minutes_read_as_a_time() {
        use super::super::model::ClimateChoice::{HotWater, Mode};
        let label = |list, o: &str| super::choice_label(list, o);
        assert_eq!([label(HotWater, "off"), label(HotWater, "30"), label(HotWater, "60"), label(HotWater, "120")],
                   ["Off", "30 min", "1 hour", "2 hours"]);
        assert_eq!(label(Mode, "fan_only"), "Fan only");
    }

    #[test]
    fn the_glance_counts_lights_switches_and_fans_apart() {
        let on = |ids: &[&str]| {
            let lights: Vec<String> = ids.iter().map(|id| format!(r#"{{"id":"{id}","name":"x","on":true}}"#)).collect();
            let payload = format!(r#"{{"cards":[{{"kind":"lights","name":"Room","icon":"","lights":[{}]}}]}}"#, lights.join(","));
            super::whats_on(&super::super::model::Cards::parse(payload.as_bytes()).unwrap().cards)
        };
        assert_eq!(on(&[]), "All quiet");
        assert_eq!(on(&["light.lamp"]), "1 light on");
        assert_eq!(on(&["light.lamp", "switch.plug"]), "1 light on · 1 switch on", "a smart plug isn't a light");
        assert_eq!(on(&["switch.a", "input_boolean.b", "fan.c", "fan.d", "light.e", "light.f"]),
                   "2 lights on · 2 switches on · 2 fans on");
    }

    #[test]
    fn list_rows_lie_on_the_round_display() {
        // The highlight's ends are half circles: each row's four are the
        // furthest it reaches from the display's middle.
        let (x, w, h) = PILL;
        let r = h / 2.0;
        for slot in 0..ROWS {
            let y = LIST_TOP + slot as f32 * ROW_PITCH;
            for (cx, cy) in [(x + r, y + r), (x + w - r, y + r)] {
                let reach = ((cx - HALF).powi(2) + (cy - HALF).powi(2)).sqrt() + r;
                assert!(reach <= HALF - 4.0, "row {slot} reaches {reach} of {HALF}");
            }
        }
    }

    #[test]
    fn the_glance_s_date_is_the_calendar_s() {
        let date = |days| glance_date(&Glance::default(), days, false);
        assert_eq!(date(0), "Thursday 1 January");
        assert_eq!(date(20_765), "Sunday 8 November");
        assert_eq!(date(19_782), "Thursday 29 February", "a leap day");
    }

    use super::*;

    #[test]
    fn a_room_is_the_average_of_its_lights_that_are_on() {
        let light = |on, brightness: Option<u8>, rgb| Light {
            id: "light.x".into(), name: String::new(), icon: String::new(), on, dimmable: true, brightness, rgb: Some(rgb),
            ..Default::default()
        };
        assert_eq!(room(&[light(false, None, [255, 0, 0])]), None);
        let lights = [light(true, Some(60), [255, 0, 0]), light(false, None, [0, 255, 0]), light(true, Some(20), [0, 0, 255])];
        assert_eq!(room(&lights), Some((40, [191, 0, 63])), "off lights don't count; brighter ones colour more");
        assert_eq!(room(&[light(true, None, [255, 255, 255])]).unwrap().0, 100, "on/off lights are full");
    }
}
