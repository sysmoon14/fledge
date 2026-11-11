//! The UI's screens, each drawn whole as the Nest draws its own: black, a
//! ring of ticks, large numbers, and colour only for what is active.


use super::icons;
use super::machine::Setting;
use super::model::{ActionCard, AlarmCard, AlarmMode, Alert, CameraCard, Card, ClimateCard, CoverCard, Glance, Light, LightsCard,
                   MediaCard, Nest, OptionsCard};
use super::paint::{self, *};

pub enum Screen<'a> {
    LightsCard { card: &'a LightsCard },
    ClimateCard { card: &'a ClimateCard, target: Option<f64> },
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
    OptionList { card: &'a OptionsCard, selected: usize },
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
    /// The time (`now`, Unix seconds), the weather and what's on.
    Glance { glance: &'a Glance, cards: &'a [Card], now: i64 },
    /// The lights card pressed: its lights and Back, `selected` highlighted.
    LightList { card: &'a LightsCard, selected: usize },
    LightDim { light: &'a Light, on: bool, value: u8 },
    /// A light's white (kelvin) and colour (hue, degrees) dials.
    LightWarmth { light: &'a Light, value: u16 },
    LightHue { light: &'a Light, value: u16 },
    ClimateSet { card: &'a ClimateCard, target: f64 },
    Volume { card: &'a MediaCard, value: u8 },
    /// This thermostat: its card (`now`, Unix seconds, for its time to
    /// temperature), and its dial setting `target`.
    NestCard { nest: &'a Nest, now: f64 },
    /// `pulse`: how far a pulse from now to the target has run (0..1).
    /// `setting`: the ring just turned, so it says what it's set to.
    /// `shown`: how far the temperature now's number has faded in (0..1),
    /// as the target leaves it or comes to it; None, as it stands.
    /// With it, a target on the side it shows (it stays where it was as it
    /// fades, rather than jumping across as the target passes).
    NestSet { nest: &'a Nest, target: f64, pulse: Option<f32>, setting: bool, shown: Option<(f32, f64)> },
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
    ModePrompt { nest: &'a Nest, selected: usize },
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
        Screen::ClimateCard { card, target } => {
            let target = target.or(card.target);
            let mut p = climate_still(fonts, card, target, false);
            climate_moving(&mut p, None, fonts, card, target, None, None, None);
            p
        }
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
        Screen::Glance { glance, cards, now } => glance_screen(fonts, glance, cards, *now),
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
        Screen::LightDim { light, on, .. } => light_dim_still(fonts, light, *on, true),
        Screen::LightWarmth { light, .. } => light_dial_still(fonts, light, "Warmth"),
        Screen::LightHue { light, .. } => light_dial_still(fonts, light, "Colour"),
        Screen::ClimateSet { card, target } => climate_still(fonts, card, Some(*target), true),
        Screen::Volume { card, .. } => volume_still(fonts, card),
        Screen::CoverSet { card, .. } => cover_still(fonts, card),
        Screen::NestSet { nest, target, setting, .. } => {
            nest_still(fonts, &predicted(nest, *target), Some(*target), *setting, unix_now())
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
        Screen::ClimateSet { card, target } => climate_moving(p, live.as_deref_mut(), fonts, card, Some(*target), sweep, None, None),
        Screen::Volume { value, .. } => volume_moving(p, live.as_deref_mut(), fonts, *value, sweep),
        Screen::CoverSet { card, value } => cover_moving(p, live.as_deref_mut(), fonts, card, *value, sweep),
        Screen::NestSet { nest, target, pulse, shown, .. } => {
            climate_moving(p, live.as_deref_mut(), fonts, &predicted(nest, *target).card, Some(*target), sweep, *pulse, *shown)
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
    match screen {
        Screen::LightDim { light, on, .. } => {
            Some(format!("dim {} {} {} {:?} {} {on}", light.id, light.name, light.icon, light.rgb, light.dimmable))
        }
        Screen::LightWarmth { light, .. } => Some(format!("warmth {}", light.id)),
        Screen::LightHue { light, .. } => Some(format!("hue {}", light.id)),
        Screen::ClimateSet { card, .. } => Some(format!("set {} {} {} {:?} {:?}", card.name, card.mode, card.action,
                                                        card.current, card.presets.is_empty())),
        Screen::Volume { card, .. } => Some(format!("volume {} {}", card.name, card.state)),
        Screen::CoverSet { card, .. } => Some(format!("cover {} {} {} {} {}", card.id, card.name, card.icon, card.state,
                                                     card.positionable)),
        Screen::NestSet { nest, target, setting, .. } => {
            let nest = predicted(nest, *target);
            // Turning only changes it in Eco (its leaf gives way).
            let setting = *setting && nest.eco;
            // (The temperature now's number is drawn with the ring, so which
            // side of its tick it's on doesn't change this.)
            Some(format!("nest {:?} {:?} {} {:?} {} {setting} {:?}", nest.card.action, nest.card.current,
                nest.eco, nest.hot_water_boost,
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
    // Among the cards, no hold hint: a hold there goes back.
    let mut p = light_dim_still(fonts, &light, light.on, false);
    if light.dimmable {
        light_dim_moving(&mut p, None, fonts, &light, light.on, value, None);
    } else {
        // On or off: the ring full in its colour, or dim.
        paint::ring(&mut p, |_| if light.on { colour(light.colour()) } else { colour(DIM) }, &[]);
    }
    p
}

pub fn climate_colours(card: &ClimateCard) -> ([u8; 3], bool) {
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

fn climate_still(fonts: &Fonts, card: &ClimateCard, target: Option<f64>, setting: bool) -> Canvas {
    let (background, active) = climate_colours(card);
    let mut p = paint::face(background);
    let text_colour = if active { WHITE } else { GREY };
    label(&mut p, fonts, 150.0, &card.name, text_colour);
    if target.is_none() || card.mode == "off" {
        paint::text(&mut p, fonts.digits(), 96.0, HALF, 300.0, "OFF", 0.0, WHITE);
    }
    let bottom = if setting {
        if card.presets.is_empty() { "Press when done".to_owned() } else { "Press for presets".to_owned() }
    } else {
        let now = card.current.map(|t| format!("{t:.1}° now")).unwrap_or_default();
        let state = match card.action.as_str() {
            "" => card.mode.replace('_', " "),
            action => action.to_owned(),
        };
        [now, state].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
    };
    line(&mut p, fonts, 366.0, &bottom, if active { [255, 225, 205] } else { GREY });
    p
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
        big_number_on(p, live, fonts, t, card.step < 1.0, "", WHITE);
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
    let hint = if done { "Done" } else if opened { "Hold to run · press for more" } else { "" };
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
    let hint = "Press · hold to dismiss";
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
                             step: 0.5, presets: Vec::new(), preset: None };
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
fn nest_still(fonts: &Fonts, nest: &Nest, target: Option<f64>, setting: bool, now: f64) -> Canvas {
    let card = &nest.card;
    let (background, active) = climate_colours(card);
    let mut p = paint::face(background);
    let soft = if active { [255, 225, 205] } else { GREY };
    let off = card.mode == "off";
    if off {
        paint::text(&mut p, fonts.digits(), 96.0, HALF, NUMBER_BASELINE - 18.0, "OFF", 0.0, WHITE);
        return p;
    }
    let _ = target;
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
            _ => Some(if card.mode == "cool" { "Cool set to" } else { "Heat set to" }.to_owned()),
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
        if let Some(until) = &nest.next_change {
            dial_words(&mut p, fonts, UNDER_BASELINE, &format!("Until {until}"), soft, None);
        }
    }
    if let Some(minutes) = nest.hot_water_boost {
        line(&mut p, fonts, 392.0, &format!("Hot water boost · {minutes} min"), soft);
    }
    p
}

/// The thermostat's card: as its dial shows it, or Eco and its leaf.
fn nest_card(fonts: &Fonts, nest: &Nest, now: f64) -> Canvas {
    let card = &nest.card;
    // Off: the ring without a target on it.
    let target = card.target.filter(|_| card.mode != "off");
    let mut p = nest_still(fonts, nest, target, false, now);
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
        Screen::OptionList { card, .. } => Screen::OptionList { card, selected },
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
fn light_dim_still(fonts: &Fonts, light: &Light, on: bool, opened: bool) -> Canvas {
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
    if opened {
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
    let top = paint::tick_at(sweep.unwrap_or(value as f32 / 100.0));
    ring_on(p, live.as_deref_mut(), |i| if on && i <= top { colour(accent) } else { colour(DIM) },
            &if on { vec![top] } else { vec![] });
    if on {
        big_number_on(p, live, fonts, value as f64, false, "%", WHITE);
    }
}

/// A light's warmth or colour dial without its ring and value: its name and
/// what the dial sets.
fn light_dial_still(fonts: &Fonts, light: &Light, what: &str) -> Canvas {
    let mut p = paint::face(BLACK);
    icon(&mut p, &light.icon, &light.id, HALF, 98.0, 40.0, GREY);
    label(&mut p, fonts, 150.0, &light.name, GREY);
    line(&mut p, fonts, 366.0, &format!("{what} · press when done"), GREY);
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
    let [low, high] = light.kelvins.unwrap_or([2000, 6500]);
    let span = (high as f32 - low as f32).max(1.0);
    let fraction = sweep.unwrap_or((value as f32 - low as f32) / span);
    let mark = paint::tick_at(fraction);
    let last = (paint::TICKS - 1) as f32;
    ring_on(p, live.as_deref_mut(), |i| colour(kelvin_rgb(low as f32 + span * i as f32 / last)), &[mark]);
    big_text_on(p, live, fonts, &value.to_string(), "K", 120.0, WHITE);
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

fn volume_still(fonts: &Fonts, card: &MediaCard) -> Canvas {
    let mut p = paint::face(BLACK);
    label(&mut p, fonts, 150.0, &card.name, MEDIA);
    let action = if card.state == "playing" { "Volume · hold to pause" } else { "Volume · hold to play" };
    line(&mut p, fonts, 366.0, action, GREY);
    p
}

fn volume_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, value: u8, sweep: Option<f32>) {
    let top = paint::tick_at(sweep.unwrap_or(value as f32 / 100.0));
    ring_on(p, live.as_deref_mut(), |i| if i <= top { colour(MEDIA) } else { colour(DIM) }, &[top]);
    big_number_on(p, live, fonts, value as f64, false, "", WHITE);
}

/// The glance: the time large, the date, the weather and what's on.
fn glance_screen(fonts: &Fonts, glance: &Glance, cards: &[Card], now: i64) -> Canvas {
    let mut p = glance_still(fonts, glance, cards, now);
    let still = p.clone();
    glance_words(&mut p, &still, fonts, cards, 0.0);
    p
}

/// The colour of Fledge's line of what's on.
const FLEDGE_ON: [u8; 3] = [255, 211, 154];

/// The glance's line of what's on: how large, how much room it has, and
/// how wide its words are.
struct GlanceLine {
    size: f32,
    room: f32,
    words: f32,
}

impl GlanceLine {
    fn new(fonts: &Fonts, on: &str) -> GlanceLine {
        let (size, room) = if paint::fledge() { (23.0, 290.0) } else { (24.0, 300.0) };
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

/// How far the glance's line of what's on scrolls before it is back where
/// it started, in pixels; None where it fits and stays put.
pub fn glance_scroll(fonts: &Fonts, cards: &[Card]) -> Option<f32> {
    let line = GlanceLine::new(fonts, &whats_on(cards));
    line.scrolls().then_some(line.words + GLANCE_GAP)
}

/// Everything the glance shows: another key, another glance to draw.
pub fn glance_key(glance: &Glance, cards: &[Card], now: i64) -> String {
    format!("{glance:?} {} {} {}", (now + glance.offset).div_euclid(60), whats_on(cards), paint::fledge())
}

/// The glance's line of what's on, `offset` pixels along, over glance_still
/// (`still`), where it scrolls: a title on the speaker, a film, a book, too
/// long to show whole. Returns the upright columns drawn in, if any were.
pub fn glance_words(p: &mut Canvas, still: &Canvas, fonts: &Fonts, cards: &[Card], offset: f32) -> Option<(u32, u32)> {
    let on = whats_on(cards);
    let line = GlanceLine::new(fonts, &on);
    if !line.scrolls() {
        return None;
    }
    let (baseline, colour) = if paint::fledge() { (404.0 + line.size * 0.36, FLEDGE_ON) } else { (402.0, ACTION) };
    let ticker = paint::Ticker { font: fonts.medium(), size: line.size, text: &on, words: line.words,
        left: HALF - line.room / 2.0, width: line.room, baseline, colour, gap: GLANCE_GAP };
    Some(paint::ticker(p, still, &ticker, offset))
}

/// The glance without the words of a line that scrolls (glance_words).
pub fn glance_still(fonts: &Fonts, glance: &Glance, cards: &[Card], now: i64) -> Canvas {
    // Big enough to read across a room: the time across most of the face,
    // the rest under it filling the round display's width.
    let mut p = paint::face(BLACK);
    let local = now + glance.offset;
    let (minutes, days) = (local.rem_euclid(86_400) / 60, local.div_euclid(86_400));
    let time = format!("{:02}:{:02}", minutes / 60, minutes % 60);
    let size = if paint::fledge() { 196.0 } else { 160.0 };
    paint::text(&mut p, fonts.clock(), size, HALF, 236.0, &time, -3.0, WHITE);
    let day = paint::fit(fonts.regular(), 30.0, &date(days), 0.0, 380.0);
    paint::text(&mut p, fonts.regular(), 30.0, HALF, 290.0, &day, 0.0, [196, 203, 211]);
    if let Some(weather) = &glance.weather {
        let temperature = weather.temperature.map(|t| format!("{t:.0}{}", if weather.unit.contains('F') { "°F" } else { "°" }));
        let text = [temperature.unwrap_or_default(), condition_name(&weather.condition)]
            .into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        let text = paint::fit(fonts.regular(), 34.0, &text, 0.0, 330.0);
        let name = format!("mdi:{}", weather_icon(&weather.condition));
        let width = paint::measure(fonts.regular(), 34.0, &text, 0.0);
        let left = HALF - (width + 56.0) / 2.0;
        icon(&mut p, &name, "weather", left + 21.0, 340.0, 44.0, WHITE);
        paint::text_at(&mut p, fonts.regular(), 34.0, left + 56.0, 352.0, &text, 0.0, WHITE, 1.0);
    }
    // What's on: in one line where it fits, else scrolling through it (the
    // words drawn by glance_words, over this).
    let on = whats_on(cards);
    let line = GlanceLine::new(fonts, &on);
    match (line.scrolls(), paint::fledge()) {
        (false, true) => pill_words_at(&mut p, fonts, 404.0, &on, FLEDGE_ON, None, line.size, line.room),
        (false, false) => paint::text(&mut p, fonts.medium(), line.size, HALF, 402.0, &on, 0.0, ACTION),
        (true, true) => {
            let (pad, height) = (line.size * 0.95, line.size * 2.1);
            paint::pill(&mut p, HALF - line.room / 2.0 - pad, 404.0 - height / 2.0, line.room + 2.0 * pad, height,
                        alpha(FLEDGE_ON, 0.18));
        }
        (true, false) => {}
    }
    p
}

/// "Sunday 8 November" from days since 1970.
fn date(days: i64) -> String {
    const DAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September",
                                "October", "November", "December"];
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{} {} {}", DAYS[days.rem_euclid(7) as usize], day, MONTHS[(month - 1) as usize])
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

/// What's on among the cards: lights, and what's playing by its title.
fn whats_on(cards: &[Card]) -> String {
    let lights: usize = cards.iter().map(|c| match c {
        Card::Lights(card) => card.lights.iter().filter(|l| l.on).count(),
        _ => 0,
    }).sum();
    let playing = cards.iter().find_map(|c| match c {
        Card::Media(m) if m.state == "playing" => Some(if m.title.is_empty() { m.name.clone() } else { m.title.clone() }),
        _ => None,
    });
    let mut parts = Vec::new();
    match lights {
        0 => {}
        1 => parts.push("1 light on".to_owned()),
        n => parts.push(format!("{n} lights on")),
    }
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
    line(&mut p, fonts, 334.0, "Press to choose", GREY);
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
    let mut p = cover_still(fonts, card);
    let value = card.position.unwrap_or(if open { 100 } else { 0 });
    cover_moving(&mut p, None, fonts, card, value, None);
    p
}

fn cover_still(fonts: &Fonts, card: &CoverCard) -> Canvas {
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
    line(&mut p, fonts, 366.0, if card.positionable { "How open · press for more" } else { "Press to open or close" }, GREY);
    p
}

fn cover_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, card: &CoverCard, value: u8,
                sweep: Option<f32>) {
    let top = paint::tick_at(sweep.unwrap_or(value as f32 / 100.0));
    ring_on(p, live.as_deref_mut(), |i| if i <= top && value > 0 { colour(COVER) } else { colour(DIM) }, &[top]);
    if card.positionable {
        big_number_on(p, live, fonts, value as f64, false, "%", WHITE);
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
        Screen::ClimateSet { card, .. } if card.mode == "cool" => COOL,
        Screen::NestSet { nest, .. } if nest.card.mode == "cool" => COOL,
        Screen::ClimateSet { .. } | Screen::NestSet { .. } => HEAT,
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
