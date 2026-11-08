//! The UI's screens, each drawn whole as the Nest draws its own: black, a
//! ring of ticks, large numbers, and colour only for what is active.

use tiny_skia::Pixmap;

use super::icons;
use super::model::{ClimateCard, Light, LightsCard, MediaCard};
use super::paint::{self, *};

pub enum Screen<'a> {
    LightsCard { card: &'a LightsCard },
    ClimateCard { card: &'a ClimateCard, target: Option<f64> },
    MediaCard { card: &'a MediaCard, volume: Option<u8> },
    SettingsCard,
    /// The lights card pressed: its lights and Back, `selected` highlighted.
    LightList { card: &'a LightsCard, selected: usize },
    LightDim { light: &'a Light, on: bool, value: u8 },
    ClimateSet { card: &'a ClimateCard, target: f64 },
    Volume { card: &'a MediaCard, value: u8 },
}

/// Where a card is among them all, for the dots.
#[derive(Clone, Copy)]
pub struct Position {
    pub index: usize,
    pub count: usize,
}

const HALF: f32 = 240.0;

pub fn draw(fonts: &Fonts, screen: &Screen, at: Position) -> Pixmap {
    match screen {
        Screen::LightsCard { card } => lights_card(fonts, card, at),
        Screen::ClimateCard { card, target } => climate(fonts, card, target.or(card.target), Some(at), false),
        Screen::MediaCard { card, volume } => media_card(fonts, card, volume.or(card.volume), at),
        Screen::SettingsCard => settings_card(fonts, at),
        Screen::LightList { card, selected } => light_list(fonts, card, *selected),
        Screen::LightDim { light, on, value } => light_dim(fonts, light, *on, *value),
        Screen::ClimateSet { card, target } => climate(fonts, card, Some(*target), None, true),
        Screen::Volume { card, value } => volume(fonts, card, *value),
    }
}

fn label(pixmap: &mut Pixmap, fonts: &Fonts, y: f32, text: &str, c: [u8; 3]) {
    let text = paint::fit(&fonts.medium, 22.0, &text.to_uppercase(), 1.5, 300.0);
    paint::text(pixmap, &fonts.medium, 22.0, HALF, y, &text, 1.5, c);
}

fn line(pixmap: &mut Pixmap, fonts: &Fonts, y: f32, text: &str, c: [u8; 3]) {
    let text = paint::fit(&fonts.regular, 20.0, text, 0.0, 320.0);
    paint::text(pixmap, &fonts.regular, 20.0, HALF, y, &text, 0.0, c);
}

/// A large number as the Nest's dial shows a temperature: 21, or 21 with a
/// small raised 5 for the half.
fn big_number(pixmap: &mut Pixmap, fonts: &Fonts, value: f64, half: bool, unit: &str, c: [u8; 3]) {
    let size = 168.0;
    let whole = if half { value.floor() as i64 } else { value.round() as i64 };
    let digits = whole.to_string();
    let small = if half && value.fract() >= 0.25 { "5" } else { unit };
    let width = paint::measure(&fonts.regular, size, &digits, -4.0);
    let small_width = if small.is_empty() { 0.0 } else { paint::measure(&fonts.regular, 52.0, small, 0.0) + 6.0 };
    let x = HALF - (width + small_width) / 2.0;
    let end = paint::text_at(pixmap, &fonts.regular, size, x, 318.0, &digits, -4.0, c, 1.0);
    if !small.is_empty() {
        // Top-aligned with the digits, as the Nest's dial sets its half degree.
        paint::text_at(pixmap, &fonts.regular, 52.0, end + 4.0, 236.0, small, 0.0, c, 1.0);
    }
}

fn icon(pixmap: &mut Pixmap, name: &str, entity: &str, cx: f32, cy: f32, size: f32, c: [u8; 3]) {
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

fn lights_card(fonts: &Fonts, card: &LightsCard, at: Position) -> Pixmap {
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
    paint::dots(&mut p, at.count, at.index, WHITE);
    p
}

fn climate(fonts: &Fonts, card: &ClimateCard, target: Option<f64>, at: Option<Position>, setting: bool) -> Pixmap {
    let (background, active) = match card.action.as_str() {
        "heating" | "preheating" => (HEAT, true),
        "cooling" => (COOL, true),
        _ => (BLACK, false),
    };
    let mut p = paint::canvas(background);
    let fraction = |t: f64| ((t - card.min) / (card.max - card.min).max(0.5)) as f32;
    let marks: Vec<usize> = target.map(|t| vec![paint::tick_at(fraction(t))]).unwrap_or_default();
    let now = card.current.map(|t| paint::tick_at(fraction(t)));
    paint::ring(&mut p, |i| {
        if Some(i) == now { alpha(WHITE, 0.9) } else if active { alpha(WHITE, 0.4) } else { colour(DIM) }
    }, &marks);
    let text_colour = if active { WHITE } else { GREY };
    label(&mut p, fonts, 150.0, &card.name, text_colour);
    match target {
        Some(t) if card.mode != "off" => big_number(&mut p, fonts, t, card.step < 1.0, "", WHITE),
        _ => paint::text(&mut p, &fonts.regular, 96.0, HALF, 300.0, "OFF", 0.0, WHITE),
    }
    let bottom = if setting {
        "Press when done".to_owned()
    } else {
        let now = card.current.map(|t| format!("{t:.1}° now")).unwrap_or_default();
        let state = match card.action.as_str() {
            "" => card.mode.replace('_', " "),
            action => action.to_owned(),
        };
        [now, state].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")
    };
    line(&mut p, fonts, 366.0, &bottom, if active { [255, 225, 205] } else { GREY });
    if let Some(at) = at {
        paint::dots(&mut p, at.count, at.index, WHITE);
    }
    p
}

fn media_card(fonts: &Fonts, card: &MediaCard, volume: Option<u8>, at: Position) -> Pixmap {
    let mut p = paint::canvas(BLACK);
    let playing = card.state == "playing";
    let lit = volume.map(|v| paint::tick_at(v as f32 / 100.0));
    paint::ring(&mut p, |i| match lit {
        Some(top) if i <= top && playing => colour(MEDIA),
        Some(top) if i <= top => alpha(MEDIA, 0.45),
        _ => colour(DIM),
    }, &[]);
    let accent = if playing { MEDIA } else { GREY };
    label(&mut p, fonts, 124.0, &card.name, accent);
    icon(&mut p, &card.icon, &card.id, HALF, 210.0, 92.0, accent);
    let title = if card.title.is_empty() { capitalise(&card.state) } else { card.title.clone() };
    let title = paint::fit(&fonts.medium, 28.0, &title, 0.0, 320.0);
    paint::text(&mut p, &fonts.medium, 28.0, HALF, 312.0, &title, 0.0, WHITE);
    let under = if card.artist.is_empty() { String::new() } else { card.artist.clone() };
    line(&mut p, fonts, 348.0, &under, GREY);
    paint::dots(&mut p, at.count, at.index, WHITE);
    p
}

fn settings_card(fonts: &Fonts, at: Position) -> Pixmap {
    let mut p = paint::canvas(BLACK);
    paint::ring(&mut p, |_| colour(DIM), &[]);
    icon(&mut p, "mdi:cog", "settings", HALF, 168.0, 96.0, WHITE);
    paint::text(&mut p, &fonts.regular, 40.0, HALF, 278.0, "Nest settings", 0.0, WHITE);
    line(&mut p, fonts, 316.0, "Schedule · Eco · Energy · System", GREY);
    paint::dots(&mut p, at.count, at.index, WHITE);
    p
}

const ROWS: usize = 5;

fn light_list(fonts: &Fonts, card: &LightsCard, selected: usize) -> Pixmap {
    let mut p = paint::canvas(BLACK);
    let accent = card.lights.iter().find(|l| l.on).map_or(GREY, |l| l.colour());
    label(&mut p, fonts, 100.0, &card.name, accent);
    let rows = card.lights.len() + 1;
    let first = selected.saturating_sub(ROWS - 1).min(rows.saturating_sub(ROWS));
    for (slot, row) in (first..rows.min(first + ROWS)).enumerate() {
        let y = 128.0 + slot as f32 * 62.0;
        let (x, w, h) = (62.0, 356.0, 54.0);
        let chosen = row == selected;
        if chosen {
            paint::pill(&mut p, x, y, w, h, colour(SELECT));
        }
        let (name, value, tint, value_tint, icon_name, entity) = match card.lights.get(row) {
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

fn light_dim(fonts: &Fonts, light: &Light, on: bool, value: u8) -> Pixmap {
    let mut p = paint::canvas(BLACK);
    let accent = if on { light.colour() } else { GREY };
    let top = paint::tick_at(value as f32 / 100.0);
    paint::ring(&mut p, |i| if on && i <= top { colour(accent) } else { colour(DIM) }, &if on { vec![top] } else { vec![] });
    icon(&mut p, &light.icon, &light.id, HALF, 98.0, 40.0, accent);
    label(&mut p, fonts, 150.0, &light.name, accent);
    if on {
        big_number(&mut p, fonts, value as f64, false, "%", WHITE);
    } else {
        paint::text(&mut p, &fonts.regular, 96.0, HALF, 300.0, "Off", 0.0, WHITE);
    }
    line(&mut p, fonts, 366.0, if on { "Press to turn off" } else { "Press to turn on" }, GREY);
    p
}

fn volume(fonts: &Fonts, card: &MediaCard, value: u8) -> Pixmap {
    let mut p = paint::canvas(BLACK);
    let top = paint::tick_at(value as f32 / 100.0);
    paint::ring(&mut p, |i| if i <= top { colour(MEDIA) } else { colour(DIM) }, &[top]);
    label(&mut p, fonts, 150.0, &card.name, MEDIA);
    big_number(&mut p, fonts, value as f64, false, "", WHITE);
    let action = if card.state == "playing" { "Volume · press to pause" } else { "Volume · press to play" };
    line(&mut p, fonts, 366.0, action, GREY);
    p
}

fn capitalise(text: &str) -> String {
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
        };
        assert_eq!(room(&[light(false, None, [255, 0, 0])]), None);
        let lights = [light(true, Some(60), [255, 0, 0]), light(false, None, [0, 255, 0]), light(true, Some(20), [0, 0, 255])];
        assert_eq!(room(&lights), Some((40, [191, 0, 63])), "off lights don't count; brighter ones colour more");
        assert_eq!(room(&[light(true, None, [255, 255, 255])]).unwrap().0, 100, "on/off lights are full");
    }
}
