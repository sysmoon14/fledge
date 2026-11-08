//! The UI's screens, each drawn whole as the Nest draws its own: black, a
//! ring of ticks, large numbers, and colour only for what is active.


use super::icons;
use super::model::{ActionCard, Alert, CameraCard, ClimateCard, CoverCard, Light, LightsCard, MediaCard, OptionsCard};
use super::paint::{self, *};

pub enum Screen<'a> {
    LightsCard { card: &'a LightsCard },
    ClimateCard { card: &'a ClimateCard, target: Option<f64> },
    /// `progress`: how far through its track (0..1), for its bar.
    MediaCard { card: &'a MediaCard, volume: Option<u8>, progress: Option<f32> },
    SettingsCard,
    /// A scene, script or button; `done` just after it ran.
    ActionCard { card: &'a ActionCard, done: bool },
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
    /// The lights card pressed: its lights and Back, `selected` highlighted.
    LightList { card: &'a LightsCard, selected: usize },
    LightDim { light: &'a Light, on: bool, value: u8 },
    /// A light's white (kelvin) and colour (hue, degrees) dials.
    LightWarmth { light: &'a Light, value: u16 },
    LightHue { light: &'a Light, value: u16 },
    ClimateSet { card: &'a ClimateCard, target: f64 },
    Volume { card: &'a MediaCard, value: u8 },
}

const HALF: f32 = 240.0;

/// Draws `screen`; `sweep`, where it has a value on its ring, is how far the
/// ring shows (0..1): the value itself, or on its way there.
pub fn draw(fonts: &Fonts, screen: &Screen, sweep: Option<f32>) -> Canvas {
    match screen {
        Screen::LightsCard { card } => lights_card(fonts, card),
        Screen::ClimateCard { card, target } => {
            let target = target.or(card.target);
            let mut p = climate_still(fonts, card, target, false);
            climate_moving(&mut p, None, fonts, card, target, None);
            p
        }
        Screen::MediaCard { card, volume, progress } => media_card(fonts, card, volume.or(card.volume), *progress),
        Screen::SettingsCard => settings_card(fonts),
        Screen::ActionCard { card, done } => action_card(fonts, card, *done),
        Screen::CameraCard { card } => camera_card(fonts, card),
        Screen::CameraView { card } => camera_view(fonts, card),
        Screen::Alert { alert } => alert_screen(fonts, alert),
        Screen::OptionsCard { card } => options_card(fonts, card),
        Screen::OptionList { card, selected } => option_list(fonts, card, *selected),
        Screen::Members { card, selected } => lights_listed(fonts, &card.name, &card.members, *selected),
        Screen::CoverCard { card } => cover_card(fonts, card),
        Screen::LightList { card, selected } => light_list(fonts, card, *selected),
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
        Screen::LightDim { light, on, .. } => light_dim_still(fonts, light, *on),
        Screen::LightWarmth { light, .. } => light_dial_still(fonts, light, "Warmth"),
        Screen::LightHue { light, .. } => light_dial_still(fonts, light, "Colour"),
        Screen::ClimateSet { card, target } => climate_still(fonts, card, Some(*target), true),
        Screen::Volume { card, .. } => volume_still(fonts, card),
        Screen::CoverSet { card, .. } => cover_still(fonts, card),
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
        Screen::ClimateSet { card, target } => climate_moving(p, live.as_deref_mut(), fonts, card, Some(*target), sweep),
        Screen::Volume { value, .. } => volume_moving(p, live.as_deref_mut(), fonts, *value, sweep),
        Screen::CoverSet { card, value } => cover_moving(p, live.as_deref_mut(), fonts, card, *value, sweep),
        _ => {}
    }
    if let Some(live) = live {
        paint::settle(live);
    }
}

/// What still() draws a screen from, to know when it can be kept.
pub fn still_key(screen: &Screen) -> Option<String> {
    match screen {
        Screen::LightDim { light, on, .. } => Some(format!("dim {light:?} {on}")),
        Screen::LightWarmth { light, .. } => Some(format!("warmth {}", light.id)),
        Screen::LightHue { light, .. } => Some(format!("hue {}", light.id)),
        Screen::ClimateSet { card, .. } => Some(format!("set {card:?}")),
        Screen::Volume { card, .. } => Some(format!("volume {card:?}")),
        Screen::CoverSet { card, .. } => Some(format!("cover {card:?}")),
        _ => None,
    }
}

fn label(pixmap: &mut Canvas, fonts: &Fonts, y: f32, text: &str, c: [u8; 3]) {
    let text = paint::fit(&fonts.medium, 22.0, &text.to_uppercase(), 1.5, 300.0);
    paint::text(pixmap, &fonts.medium, 22.0, HALF, y, &text, 1.5, c);
}

fn line(pixmap: &mut Canvas, fonts: &Fonts, y: f32, text: &str, c: [u8; 3]) {
    let text = paint::fit(&fonts.regular, 20.0, text, 0.0, 320.0);
    paint::text(pixmap, &fonts.regular, 20.0, HALF, y, &text, 0.0, c);
}

/// A large number as the Nest's dial shows a temperature: 21, or 21 with a
/// small raised 5 for the half.
/// big_number, on a Live screen when there is one.
#[allow(clippy::too_many_arguments)]
fn big_number_on(pixmap: &mut Canvas, live: Option<&mut paint::Live>, fonts: &Fonts, value: f64, half: bool, unit: &str,
                 c: [u8; 3]) {
    let size = 168.0;
    let whole = if half { value.floor() as i64 } else { value.round() as i64 };
    let digits = whole.to_string();
    let small = if half && value.fract() >= 0.25 { "5" } else { unit };
    let width = paint::measure(&fonts.regular, size, &digits, -4.0);
    let small_width = if small.is_empty() { 0.0 } else { paint::measure(&fonts.regular, 52.0, small, 0.0) + 6.0 };
    let x = HALF - (width + small_width) / 2.0;
    // Top-aligned with the digits, as the Nest's dial sets its half degree.
    let small_x = x + width + 4.0;
    let mut parts = vec![(&fonts.regular, size, x, 318.0, digits.as_str(), -4.0)];
    if !small.is_empty() {
        parts.push((&fonts.regular, 52.0, small_x, 236.0, small, 0.0));
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
    let mut p = paint::canvas(BLACK);
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
            paint::text(&mut p, &fonts.regular, 96.0, HALF, 320.0, "Off", 0.0, WHITE);
            line(&mut p, fonts, 366.0, &format!("{} lights", lights.len()), GREY);
        }
        Some((level, _)) => {
            let on = lights.iter().filter(|l| l.on).count();
            paint::text(&mut p, &fonts.regular, 112.0, HALF, 322.0, &on.to_string(), 0.0, WHITE);
            line(&mut p, fonts, 366.0, &format!("of {} on · {level}%", lights.len()), GREY);
        }
    }
    p
}

/// A card of one light: its brightness on the ring and in the middle.
fn light_card(fonts: &Fonts, card: &LightsCard, light: &Light) -> Canvas {
    let mut p = paint::canvas(BLACK);
    let tint = light.colour();
    let level = light.on.then(|| light.brightness.unwrap_or(100));
    let top = level.map(|level| paint::tick_at(level as f32 / 100.0));
    paint::ring(&mut p, |i| match top {
        Some(top) if i <= top => colour(tint),
        _ => colour(DIM),
    }, &[]);
    let accent = if light.on { tint } else { GREY };
    let icon_name = if light.icon.is_empty() { &card.icon } else { &light.icon };
    icon(&mut p, icon_name, &light.id, HALF, 132.0, 72.0, accent);
    label(&mut p, fonts, 214.0, &card.name, accent);
    let middle = match (light.on, light.dimmable) {
        (false, _) => "Off".to_owned(),
        (true, true) => format!("{}%", level.unwrap_or(100)),
        (true, false) => "On".to_owned(),
    };
    paint::text(&mut p, &fonts.regular, 96.0, HALF, 320.0, &middle, 0.0, WHITE);
    p
}

fn climate_colours(card: &ClimateCard) -> ([u8; 3], bool) {
    match card.action.as_str() {
        "heating" | "preheating" => (HEAT, true),
        "cooling" => (COOL, true),
        _ => (BLACK, false),
    }
}

fn climate_still(fonts: &Fonts, card: &ClimateCard, target: Option<f64>, setting: bool) -> Canvas {
    let (background, active) = climate_colours(card);
    let mut p = paint::canvas(background);
    let text_colour = if active { WHITE } else { GREY };
    label(&mut p, fonts, 150.0, &card.name, text_colour);
    if target.is_none() || card.mode == "off" {
        paint::text(&mut p, &fonts.regular, 96.0, HALF, 300.0, "OFF", 0.0, WHITE);
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

fn climate_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, card: &ClimateCard,
                  target: Option<f64>, sweep: Option<f32>) {
    let (_, active) = climate_colours(card);
    let fraction = |t: f64| ((t - card.min) / (card.max - card.min).max(0.5)) as f32;
    let marks: Vec<usize> = target.map(|t| vec![paint::tick_at(sweep.unwrap_or(fraction(t)))]).unwrap_or_default();
    let now = card.current.map(|t| paint::tick_at(fraction(t)));
    ring_on(p, live.as_deref_mut(), |i| {
        if Some(i) == now { alpha(WHITE, 0.9) } else if active { alpha(WHITE, 0.4) } else { colour(DIM) }
    }, &marks);
    if let Some(t) = target.filter(|_| card.mode != "off") {
        big_number_on(p, live, fonts, t, card.step < 1.0, "", WHITE);
    }
}

/// How wide a media card's progress bar is.
pub const PROGRESS_WIDTH: f32 = 180.0;

fn media_card(fonts: &Fonts, card: &MediaCard, volume: Option<u8>, progress: Option<f32>) -> Canvas {
    let mut p = paint::canvas(BLACK);
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
    let title = paint::fit(&fonts.medium, 28.0, &title, 0.0, 300.0);
    paint::text(&mut p, &fonts.medium, 28.0, HALF, top + 42.0, &title, 0.0, WHITE);
    // How far through the track, under its title.
    let mut below = top + 76.0;
    if let Some(done) = progress {
        let (x, y) = (HALF - PROGRESS_WIDTH / 2.0, top + 58.0);
        paint::pill(&mut p, x, y, PROGRESS_WIDTH, 5.0, colour(DIM));
        let width = (PROGRESS_WIDTH * done).max(5.0);
        paint::pill(&mut p, x, y, width, 5.0, colour(if playing { MEDIA } else { GREY }));
        below += 14.0;
    }
    let under = if card.artist.is_empty() { String::new() } else { card.artist.clone() };
    line(&mut p, fonts, below, &under, GREY);
    p
}

/// Scenes' and scripts' accent: a warm gold.
const ACTION: [u8; 3] = [255, 196, 87];

/// No ring: there's nothing to turn. Done, its icon and words turn gold.
fn action_card(fonts: &Fonts, card: &ActionCard, done: bool) -> Canvas {
    let mut p = paint::canvas(BLACK);
    // Its own icon if the thermostat has it, else a play button.
    let found = icons::lookup(&card.icon, &card.id).or_else(|| icons::lookup("mdi:play-circle-outline", ""));
    if let Some(found) = found {
        paint::icon(&mut p, found, HALF, 178.0, 104.0, colour(if done { ACTION } else { WHITE }));
    }
    let name = paint::fit(&fonts.regular, 40.0, &card.name, 0.0, 330.0);
    paint::text(&mut p, &fonts.regular, 40.0, HALF, 288.0, &name, 0.0, WHITE);
    line(&mut p, fonts, 330.0, if done { "Done" } else { "Press to run" }, if done { ACTION } else { GREY });
    p
}

/// A camera's latest still, the whole display; before the first, its icon
/// and name.
fn camera_card(fonts: &Fonts, card: &CameraCard) -> Canvas {
    let mut p = paint::canvas(BLACK);
    match &card.art {
        // Its name shows above the cards: the picture alone.
        Some(art) => paint::picture(&mut p, &art.rgb, art.size, HALF, HALF, 0.0),
        None => {
            paint::ring(&mut p, |_| colour(DIM), &[]);
            let icon_name = if card.icon.is_empty() { "mdi:cctv" } else { &card.icon };
            icon(&mut p, icon_name, &card.id, HALF, 178.0, 96.0, WHITE);
            let name = paint::fit(&fonts.regular, 36.0, &card.name, 0.0, 330.0);
            paint::text(&mut p, &fonts.regular, 36.0, HALF, 286.0, &name, 0.0, WHITE);
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
        let name = paint::fit(&fonts.medium, 24.0, &card.name, 0.0, 280.0);
        let width = paint::measure(&fonts.medium, 24.0, &name, 0.0) + 36.0;
        paint::pill(&mut p, HALF - width / 2.0, 386.0, width, 40.0, alpha(BLACK, 0.6));
        paint::text(&mut p, &fonts.medium, 24.0, HALF, 414.0, &name, 0.0, WHITE);
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
    let mut p = paint::canvas(BLACK);
    let hint = "Press · hold to dismiss";
    match &alert.art {
        Some(art) => {
            paint::picture(&mut p, &art.rgb, art.size, HALF, HALF, 0.0);
            // The title over the picture, on a band, in up to two lines.
            let title = if alert.title.is_empty() { &alert.message } else { &alert.title };
            let lines = wrap(&fonts.medium, 26.0, title, 300.0, 2);
            let width = lines.iter().map(|l| paint::measure(&fonts.medium, 26.0, l, 0.0)).fold(0.0, f32::max) + 44.0;
            let height = 18.0 + 32.0 * lines.len() as f32;
            let top = 412.0 - height;
            paint::pill(&mut p, HALF - width / 2.0, top, width, height, alpha(BLACK, 0.65));
            for (i, text) in lines.iter().enumerate() {
                paint::text(&mut p, &fonts.medium, 26.0, HALF, top + 34.0 + 32.0 * i as f32, text, 0.0, WHITE);
            }
        }
        None => {
            let icon_name = if alert.icon.is_empty() { "mdi:bell-ring" } else { &alert.icon };
            icon(&mut p, icon_name, "alert", HALF, 132.0, 80.0, ACTION);
            let mut y = 222.0;
            if !alert.title.is_empty() {
                let title = paint::fit(&fonts.medium, 34.0, &alert.title, 0.0, 340.0);
                paint::text(&mut p, &fonts.medium, 34.0, HALF, y, &title, 0.0, WHITE);
                y += 44.0;
            }
            for text in wrap(&fonts.regular, 24.0, &alert.message, 330.0, 3) {
                paint::text(&mut p, &fonts.regular, 24.0, HALF, y, &text, 0.0, [220, 223, 228]);
                y += 32.0;
            }
            line(&mut p, fonts, 372.0, hint, GREY);
        }
    }
    p
}

/// No ring: there's nothing to turn.
fn settings_card(fonts: &Fonts) -> Canvas {
    let mut p = paint::canvas(BLACK);
    icon(&mut p, "mdi:cog", "settings", HALF, 168.0, 96.0, WHITE);
    paint::text(&mut p, &fonts.regular, 40.0, HALF, 278.0, "Nest settings", 0.0, WHITE);
    line(&mut p, fonts, 316.0, "Schedule · Eco · Energy · System", GREY);
    p
}

const ROWS: usize = 5;

fn light_list(fonts: &Fonts, card: &LightsCard, selected: usize) -> Canvas {
    lights_listed(fonts, &card.name, &card.lights, selected)
}

/// Lights listed under `name`, each with its state, then Back.
fn lights_listed(fonts: &Fonts, name: &str, lights: &[Light], selected: usize) -> Canvas {
    let mut p = paint::canvas(BLACK);
    let accent = lights.iter().find(|l| l.on).map_or(GREY, |l| l.colour());
    label(&mut p, fonts, 100.0, name, accent);
    let rows = lights.len() + 1;
    let first = selected.saturating_sub(ROWS - 1).min(rows.saturating_sub(ROWS));
    for (slot, row) in (first..rows.min(first + ROWS)).enumerate() {
        let y = 128.0 + slot as f32 * 62.0;
        let (x, w, h) = (62.0, 356.0, 54.0);
        let chosen = row == selected;
        if chosen {
            paint::pill(&mut p, x, y, w, h, colour(SELECT));
        }
        let (name, value, tint, value_tint, icon_name, entity) = match lights.get(row) {
            Some(light) => {
                let value = match (light.on, light.brightness) {
                    (true, Some(b)) if light.dimmable => format!("{b}%"),
                    (true, _) => "On".to_owned(),
                    (false, _) => "Off".to_owned(),
                };
                let tint = if light.on { light.colour() } else { DIM };
                (light.name.as_str(), value, tint, if light.on { light.colour() } else { [107, 112, 121] },
                 light.icon.as_str(), light.id.as_str())
            }
            None => ("Back", String::new(), GREY, GREY, "mdi:arrow-left", "back"),
        };
        let (tint, value_tint, text_tint) = if chosen { (WHITE, WHITE, WHITE) } else { (tint, value_tint, WHITE) };
        icon(&mut p, icon_name, entity, x + 34.0, y + h / 2.0, 28.0, tint);
        let value_width = paint::measure(&fonts.regular, 20.0, &value, 0.0);
        let name = paint::fit(&fonts.regular, 24.0, name, 0.0, w - 90.0 - value_width - 16.0);
        paint::text_at(&mut p, &fonts.regular, 24.0, x + 62.0, y + 36.0, &name, 0.0, text_tint, 1.0);
        paint::text_at(&mut p, &fonts.regular, 20.0, x + w - 24.0 - value_width, y + 35.0, &value, 0.0, value_tint, 1.0);
    }
    p
}

fn light_dim_still(fonts: &Fonts, light: &Light, on: bool) -> Canvas {
    let mut p = paint::canvas(BLACK);
    let accent = if on { light.colour() } else { GREY };
    icon(&mut p, &light.icon, &light.id, HALF, 98.0, 40.0, accent);
    label(&mut p, fonts, 150.0, &light.name, accent);
    if !on {
        paint::text(&mut p, &fonts.regular, 96.0, HALF, 300.0, "Off", 0.0, WHITE);
    }
    line(&mut p, fonts, 366.0, if on { "Press to turn off" } else { "Press to turn on" }, GREY);
    p
}

#[allow(clippy::too_many_arguments)]
fn light_dim_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, light: &Light, on: bool,
                    value: u8, sweep: Option<f32>) {
    let accent = if on { light.colour() } else { GREY };
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
    let mut p = paint::canvas(BLACK);
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
    let width = paint::measure(&fonts.regular, size, digits, -3.0);
    let small = size * 0.31;
    let small_width = if unit.is_empty() { 0.0 } else { paint::measure(&fonts.regular, small, unit, 0.0) + 6.0 };
    let x = HALF - (width + small_width) / 2.0;
    let mut parts = vec![(&fonts.regular, size, x, 300.0, digits, -3.0)];
    if !unit.is_empty() {
        parts.push((&fonts.regular, small, x + width + 4.0, 300.0 - size * 0.7 + small, unit, 0.0));
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
    let mut p = paint::canvas(BLACK);
    label(&mut p, fonts, 150.0, &card.name, MEDIA);
    let action = if card.state == "playing" { "Volume · press to pause" } else { "Volume · press to play" };
    line(&mut p, fonts, 366.0, action, GREY);
    p
}

fn volume_moving(p: &mut Canvas, mut live: Option<&mut paint::Live>, fonts: &Fonts, value: u8, sweep: Option<f32>) {
    let top = paint::tick_at(sweep.unwrap_or(value as f32 / 100.0));
    ring_on(p, live.as_deref_mut(), |i| if i <= top { colour(MEDIA) } else { colour(DIM) }, &[top]);
    big_number_on(p, live, fonts, value as f64, false, "", WHITE);
}

/// A device's ring of actions, as the Nest's own press menu: the device's
/// screen dimmed, the actions' icons around the ring from lower left, the
/// chosen one blue and named under the device's name.
pub fn menu(p: &mut Canvas, fonts: &Fonts, title: &str, items: &[(&str, &str)], selected: usize) {
    paint::dim(p, 0.06);
    for (i, (name, _)) in items.iter().enumerate() {
        let angle = (-135.0 + 55.0 * i as f32).to_radians();
        let (x, y) = (HALF + 178.0 * angle.sin(), HALF - 178.0 * angle.cos());
        let chosen = i == selected;
        let colour = if chosen { [26, 115, 232] } else { [138, 143, 152] };
        icon(p, name, "back", x, y, if chosen { 52.0 } else { 44.0 }, colour);
    }
    let title = paint::fit(&fonts.regular, 30.0, title, 0.0, 300.0);
    paint::text(p, &fonts.regular, 30.0, HALF, 232.0, &title, 0.0, WHITE);
    if let Some((_, label)) = items.get(selected) {
        paint::text(p, &fonts.medium, 19.0, HALF, 270.0, label, 0.2, [74, 144, 226]);
    }
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
    let mut p = paint::canvas(BLACK);
    let found = icons::lookup(&card.icon, &card.id).or_else(|| icons::lookup("mdi:format-list-bulleted", ""));
    if let Some(found) = found {
        paint::icon(&mut p, found, HALF, 150.0, 84.0, colour(WHITE));
    }
    label(&mut p, fonts, 232.0, &card.name, GREY);
    let current = paint::fit(&fonts.regular, 40.0, card.current.as_deref().unwrap_or("—"), 0.0, 330.0);
    paint::text(&mut p, &fonts.regular, 40.0, HALF, 292.0, &current, 0.0, WHITE);
    line(&mut p, fonts, 334.0, "Press to choose", GREY);
    p
}

/// A select's options, the current one marked, then Back.
fn option_list(fonts: &Fonts, card: &OptionsCard, selected: usize) -> Canvas {
    let mut p = paint::canvas(BLACK);
    label(&mut p, fonts, 100.0, &card.name, GREY);
    let rows = card.options.len() + 1;
    let first = selected.saturating_sub(ROWS - 1).min(rows.saturating_sub(ROWS));
    for (slot, row) in (first..rows.min(first + ROWS)).enumerate() {
        let y = 128.0 + slot as f32 * 62.0;
        let (x, w, h) = (62.0, 356.0, 54.0);
        if row == selected {
            paint::pill(&mut p, x, y, w, h, colour(SELECT));
        }
        let (name, now) = match card.options.get(row) {
            Some(option) => (option.as_str(), card.current.as_deref() == Some(option.as_str())),
            None => ("Back", false),
        };
        if row == card.options.len() {
            icon(&mut p, "mdi:arrow-left", "back", x + 34.0, y + h / 2.0, 28.0, if row == selected { WHITE } else { GREY });
        }
        let value = if now { "Now" } else { "" };
        let value_width = paint::measure(&fonts.regular, 20.0, value, 0.0);
        let name = paint::fit(&fonts.regular, 24.0, name, 0.0, w - 90.0 - value_width - 16.0);
        paint::text_at(&mut p, &fonts.regular, 24.0, x + 62.0, y + 36.0, &name, 0.0, WHITE, 1.0);
        if now {
            paint::text_at(&mut p, &fonts.regular, 20.0, x + w - 22.0 - value_width, y + 35.0, value, 0.0, ACTION, 1.0);
        }
    }
    p
}

/// A cover: how open on the ring, if it knows; open or closed in words.
fn cover_card(fonts: &Fonts, card: &CoverCard) -> Canvas {
    let mut p = paint::canvas(BLACK);
    let open = card.state == "open" || card.state == "opening";
    if let Some(position) = card.position {
        let top = paint::tick_at(position as f32 / 100.0);
        paint::ring(&mut p, |i| if i <= top && position > 0 { colour(COVER) } else { colour(DIM) }, &[]);
    }
    let name = if open { "mdi:window-shutter-open" } else { "mdi:window-shutter" };
    let found = icons::lookup(&card.icon, &card.id).or_else(|| icons::lookup(name, ""));
    if let Some(found) = found {
        paint::icon(&mut p, found, HALF, 132.0, 72.0, colour(if open { COVER } else { GREY }));
    }
    label(&mut p, fonts, 214.0, &card.name, if open { COVER } else { GREY });
    let middle = match (card.position, card.state.as_str()) {
        (Some(p), _) if card.positionable => format!("{p}%"),
        (_, "opening") => "Opening".to_owned(),
        (_, "closing") => "Closing".to_owned(),
        (_, state) => capitalise(state),
    };
    paint::text(&mut p, &fonts.regular, 80.0, HALF, 316.0, &middle, 0.0, WHITE);
    p
}

fn cover_still(fonts: &Fonts, card: &CoverCard) -> Canvas {
    let mut p = paint::canvas(BLACK);
    label(&mut p, fonts, 150.0, &card.name, COVER);
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

#[cfg(test)]
mod tests {
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
