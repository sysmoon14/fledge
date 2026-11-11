//! A showcase of the screens as frames for a video: a walk through the
//! cards, drawn by the code that draws them on the thermostat, moving as
//! they move there (the same easing, a frame every sixtieth of a second).
//!
//!     UNFR0ZEN_UI_FONTS=<Nest's fonts> UNFR0ZEN_UI_FILM=<dir> \
//!         cargo test -p nestlocal ui::film -- --ignored
//!
//! writes <dir>/00000.png onwards, upright, and <dir>/captions.txt: the
//! frame each part starts at, and what it shows.

use std::sync::Arc;

use super::model::{self, Art, Card, Cards, Nest};
use super::screens::{self, Screen};
use super::{device, motion, paint, zoom, FOLLOW, ZOOM_EASE};

const SIDE: usize = paint::SIZE as usize;
/// A frame's length, and how far a follower gets in one.
const FRAME: f32 = 1.0 / 60.0;

fn towards(ease: f32) -> f32 {
    1.0 - (-FRAME / ease).exp()
}

/// The cards in the ring's order: each drawn, round, and zoomed out.
struct Deck {
    names: Vec<String>,
    full: Vec<device::Frame>,
    shrunk: Vec<device::Frame>,
}

impl Deck {
    fn new(fonts: &paint::Fonts, cards: &[(&str, Screen)]) -> Deck {
        let mut deck = Deck { names: Vec::new(), full: Vec::new(), shrunk: Vec::new() };
        for (name, screen) in cards {
            deck.names.push((*name).to_owned());
            deck.full.push(Vec::new());
            deck.shrunk.push(Vec::new());
            let at = deck.full.len() - 1;
            deck.set(fonts, at, screen);
        }
        deck
    }

    fn set(&mut self, fonts: &paint::Fonts, at: usize, screen: &Screen) {
        let mut frame = device::frame(screens::draw(fonts, screen, None));
        zoom::round(&mut frame);
        self.shrunk[at] = zoom::shrink(&frame);
        self.full[at] = frame;
    }

    /// The cards at place `at`, zoomed to `z`, as the UI shows them.
    fn frame(&self, fonts: &paint::Fonts, at: f32, z: f32) -> device::Frame {
        let mut out = vec![0u8; SIDE * SIDE * 4];
        let count = self.full.len() as i64;
        let shrunk = z < 0.95;
        let z = if (z - zoom::zoomed()).abs() < 0.003 { zoom::zoomed() } else { z };
        let get = |k: i64| -> Option<&[u8]> {
            let cards = if shrunk { &self.shrunk } else { &self.full };
            (0..count).contains(&k).then(|| cards[k as usize].as_slice())
        };
        zoom::compose(&mut out, at, z, if shrunk { zoom::zoomed() } else { 1.0 }, get);
        let fade = ((1.0 - z) / (1.0 - zoom::zoomed())).clamp(0.0, 1.0);
        let nearest = at.round().clamp(0.0, count as f32 - 1.0) as usize;
        let title = fade * (1.0 - 2.0 * (at - at.round()).abs()).clamp(0.0, 1.0);
        paint::around_cards(&mut out, fonts, &self.names[nearest], title, count as usize, nearest, fade);
        out
    }
}

struct Film {
    dir: String,
    frames: usize,
    last: device::Frame,
    captions: Vec<(usize, String)>,
}

impl Film {
    /// One frame, as the panel holds it: saved upright.
    fn show(&mut self, frame: &[u8]) {
        let mut upright = vec![0u8; SIDE * SIDE * 3];
        for y in 0..SIDE {
            for x in 0..SIDE {
                let from = 4 * (x * SIDE + (SIDE - 1 - y));
                let to = 3 * (y * SIDE + x);
                upright[to..to + 3].copy_from_slice(&[frame[from + 2], frame[from + 1], frame[from]]);
            }
        }
        let file = std::fs::File::create(format!("{}/{:05}.png", self.dir, self.frames)).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), SIDE as u32, SIDE as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.write_header().unwrap().write_image_data(&upright).unwrap();
        self.frames += 1;
        self.last = frame.to_vec();
    }

    fn hold(&mut self, frames: usize) {
        let last = self.last.clone();
        for _ in 0..frames {
            self.show(&last);
        }
    }

    /// Fades from what shows to `to`, as the UI's fade does (280 ms).
    fn fade(&mut self, to: &[u8]) {
        let from = self.last.clone();
        for i in 1..=17 {
            let t = motion::curve(i as f32 / 17.0);
            let frame: Vec<u8> = from.iter().zip(to).map(|(&a, &b)| (a as f32 + (b as f32 - a as f32) * t).round() as u8).collect();
            self.show(&frame);
        }
    }

    fn caption(&mut self, text: &str) {
        self.captions.push((self.frames, text.to_owned()));
    }

    /// The ring turned from card `from` to card `to`, a card every `pace`
    /// frames, the cards following as they do; then settled.
    fn turn(&mut self, fonts: &paint::Fonts, deck: &Deck, from: usize, to: usize, pace: usize) {
        let (mut view, end) = (from as f32, to as f32);
        let steps = pace * from.abs_diff(to);
        let mut frame = 0;
        loop {
            frame += 1;
            let hand = from as f32 + (end - from as f32) * (frame as f32 / steps as f32).min(1.0);
            view += (hand - view) * towards(FOLLOW);
            if frame >= steps && (view - end).abs() < 0.001 {
                view = end;
            }
            self.show(&deck.frame(fonts, view, zoom::zoomed()));
            if view == end {
                return;
            }
        }
    }

    /// The cards at `at` zooming from `from` to `to`.
    fn zoom(&mut self, fonts: &paint::Fonts, deck: &Deck, at: usize, from: f32, to: f32) {
        let mut z = from;
        while z != to {
            z += (to - z) * towards(ZOOM_EASE);
            if (z - to).abs() < 0.002 {
                z = to;
            }
            self.show(&deck.frame(fonts, at as f32, z));
        }
    }

    /// A value's screen as the ring turns it from `from` to `to` over
    /// `frames`: `screen(value)` drawn with its ring `sweep(value)` of the
    /// way round, the ring following the hand; then settled.
    fn dial<'a>(&mut self, fonts: &paint::Fonts, from: f32, to: f32, frames: usize, shown: &mut f32,
                screen: &dyn Fn(f32) -> (Screen<'a>, f32)) {
        let mut frame = 0;
        // The frame as drawn last, and a change of colour under way: what it
        // fades from, and how far along it is.
        let mut drawn_last: Option<device::Frame> = None;
        let mut recolour: Option<(device::Frame, usize)> = None;
        const RECOLOUR: usize = 42;
        loop {
            frame += 1;
            let value = from + (to - from) * (frame as f32 / frames as f32).min(1.0);
            let (drawn, sweep) = screen(value);
            *shown += (sweep - *shown) * towards(FOLLOW);
            let settled = frame >= frames && (*shown - sweep).abs() < 0.002;
            if settled {
                *shown = sweep;
            }
            let drawn = device::frame(screens::draw(fonts, &drawn, Some(*shown)));
            // The whole face another colour (the thermostat starting to
            // heat): faded to over 700 ms, as the UI does.
            if let Some(last) = &drawn_last {
                let apart: usize = last.iter().zip(&drawn).step_by(97).map(|(&a, &b)| a.abs_diff(b) as usize).sum();
                if apart / (last.len() / 97) > 30 {
                    recolour = Some((self.last.clone(), 0));
                }
            }
            drawn_last = Some(drawn.clone());
            match &mut recolour {
                Some((before, step)) if *step < RECOLOUR => {
                    *step += 1;
                    let t = motion::curve(*step as f32 / RECOLOUR as f32);
                    let mixed: Vec<u8> = before.iter().zip(&drawn).map(|(&a, &b)| (a as f32 + (b as f32 - a as f32) * t).round() as u8).collect();
                    self.show(&mixed);
                }
                _ => self.show(&drawn),
            }
            if settled && recolour.as_ref().is_none_or(|(_, step)| *step >= RECOLOUR) {
                return;
            }
        }
    }
}

/// An album's cover for the speaker's card: dusk over water.
fn cover() -> Arc<Art> {
    let size = 168u32;
    let rgb = (0..size * size).flat_map(|i| {
        let (x, y) = ((i % size) as f32 / size as f32, (i / size) as f32 / size as f32);
        let sun = ((x - 0.5).powi(2) + (y - 0.52).powi(2)).sqrt();
        let sky = [40.0 + 215.0 * y.powf(1.6), 30.0 + 120.0 * y.powf(2.2), 110.0 + 40.0 * (1.0 - y)];
        let lit = if y < 0.62 && sun < 0.17 { [255.0, 214.0, 150.0] } else { sky };
        // Water under the horizon: darker, with the sun's path down it.
        let water = y > 0.62;
        let path = water && (x - 0.5).abs() < 0.05 + (y - 0.62) * 0.25 && ((y * 60.0) as u32).is_multiple_of(2);
        let [r, g, b] = match (water, path) {
            (true, true) => [240.0, 170.0, 120.0],
            (true, false) => [30.0 + 50.0 * (1.0 - y), 24.0 + 40.0 * (1.0 - y), 70.0 + 60.0 * (1.0 - y)],
            _ => lit,
        };
        [r as u8, g as u8, b as u8]
    }).collect();
    Arc::new(Art { size, rgb })
}

fn nest(target: f64, current: f64) -> Nest {
    let action = if target > current { "heating" } else { "idle" };
    let state = serde_json::json!({"mode": "heat", "action": action, "target": target, "current_temperature": current,
        "preset": "none", "temperature_scale": "C", "can_heat": true, "has_hot_water_control": true,
        "hot_water_boost": "off", "away_temperature_low": 12.0, "learning_mode": true, "auto_away_enable": true,
        "version": "v0.5.3", "update": null, "next_change": "22:30",
        "time_to_target": if target > current { serde_json::json!("2026-11-08T21:35:00Z") } else { serde_json::Value::Null }});
    Nest::from_state(state.as_object().unwrap())
}

#[test]
#[ignore]
fn ui_film() {
    let (Ok(fonts), Ok(dir)) = (std::env::var("UNFR0ZEN_UI_FONTS"), std::env::var("UNFR0ZEN_UI_FILM")) else {
        panic!("set UNFR0ZEN_UI_FONTS and UNFR0ZEN_UI_FILM");
    };
    let fonts = paint::Fonts::load(&fonts).unwrap();
    paint::for_panel();
    let now = 1_794_172_380;
    let parse = |brightness: u8, zone: f64, volume: u8, title: &str| {
        let json = format!(r#"{{"cards":[
            {{"kind":"lights","name":"Reading Lamp","icon":"m3s:table-lamp","lights":[
                {{"id":"light.reading_lamp","name":"Reading Lamp","icon":"m3s:table-lamp","on":true,"dimmable":true,"brightness":{brightness},"rgb":[255,169,87]}}]}},
            {{"kind":"climate","id":"climate.kitchen","name":"Kitchen","icon":"","mode":"heat","action":"{}","target":{zone},"current":19.5,"min":7,"max":30,"step":0.5}},
            {{"kind":"media","id":"media_player.speaker","name":"Kitchen speaker","icon":"","state":"playing","title":"{title}","artist":"Low Harbour","volume":{volume}}}]}}"#,
            if zone > 19.5 { "heating" } else { "idle" });
        let mut cards = Cards::parse(json.as_bytes()).unwrap();
        if let Card::Media(media) = &mut cards.cards[2] {
            media.art = Some(cover());
        }
        cards
    };
    let scene = model::ActionCard { id: "scene.goodnight".into(), name: "Goodnight".into(), icon: "mdi:weather-night".into() };
    let blind = model::CoverCard { id: "cover.blind".into(), name: "Bedroom blind".into(), icon: String::new(),
        state: "open".into(), position: Some(60), positionable: true };
    let glance = model::Glance { offset: 0, weather: Some(model::Weather { condition: "partlycloudy".into(),
        temperature: Some(8.4), unit: "°C".into() }), ..model::Glance::default() };
    let mut film = Film { dir, frames: 0, last: Vec::new(), captions: Vec::new() };

    // The glance, a long title scrolling through its line.
    let glanced = |film: &mut Film, fonts: &paint::Fonts, frames: usize, fade: bool| {
        let cards = parse(60, 21.5, 35, "The Lighthouse at the End of the Bay");
        let at = screens::GlanceIn { glance: &glance, cards: &cards.cards, nest: None, now };
        let still = screens::glance_still(fonts, &at);
        let mut canvas = still.clone();
        let round = screens::glance_scroll(fonts, &at).expect("a line that scrolls");
        for frame in 0..frames {
            screens::glance_words(&mut canvas, &still, fonts, &at, super::ticker_offset(frame as f32 * FRAME, round));
            let shown = canvas.pixmap.data().to_vec();
            if frame == 0 && fade {
                film.fade(&shown);
            } else {
                film.show(&shown);
            }
        }
    };
    // The cards as they stand: a lamp at `brightness`, a zone set to `zone`,
    // a speaker at `volume`, the thermostat set to `target`.
    let deck = |fonts: &paint::Fonts, brightness: u8, zone: f64, volume: u8, target: f64| {
        let cards = parse(brightness, zone, volume, "Slow Tide");
        let [Card::Lights(lamp), Card::Climate(kitchen), Card::Media(speaker)] = &cards.cards[..] else { panic!() };
        let thermostat = nest(target, 21.0);
        Deck::new(fonts, &[
            ("Thermostat", Screen::NestCard { nest: std::borrow::Cow::Borrowed(&thermostat), now: now as f64 }),
            ("Reading Lamp", Screen::LightsCard { card: lamp }),
            ("Kitchen", Screen::NestCard { nest: std::borrow::Cow::Owned(Nest::of_card(kitchen)), now: 0.0 }),
            ("Kitchen speaker", Screen::MediaCard { card: speaker, volume: None, progress: Some(0.4) }),
            ("Goodnight", Screen::ActionCard { card: &scene, done: false, opened: false }),
            ("Bedroom blind", Screen::CoverCard { card: &blind }),
            ("Settings", Screen::SettingsCard),
        ])
    };
    // A card opened (zoomed into, then its own screen faded to), its value
    // turned through `stops`, and left again for the cards as `after`.
    #[allow(clippy::too_many_arguments)]
    fn open<'a>(film: &mut Film, fonts: &paint::Fonts, before: &Deck, after: &Deck, at: usize, stops: &[(f32, usize)],
                screen: &dyn Fn(f32) -> (Screen<'a>, f32)) {
        film.zoom(fonts, before, at, zoom::zoomed(), 1.0);
        let (first, sweep) = screen(stops[0].0);
        let mut shown = sweep;
        film.fade(&device::frame(screens::draw(fonts, &first, Some(sweep))));
        film.hold(24);
        for pair in stops.windows(2) {
            film.dial(fonts, pair[0].0, pair[1].0, pair[1].1, &mut shown, screen);
            film.hold(26);
        }
        film.fade(&after.frame(fonts, at as f32, 1.0));
        film.zoom(fonts, after, at, 1.0, zoom::zoomed());
        film.hold(18);
    }

    for theme in [paint::Theme::Fledge, paint::Theme::Nest] {
        paint::set_theme(theme);
        let fledge = theme == paint::Theme::Fledge;
        let lamp_cards = parse(60, 21.5, 35, "Slow Tide");
        let [Card::Lights(lamp), Card::Climate(kitchen), Card::Media(speaker)] = &lamp_cards.cards[..] else { panic!() };
        let light = &lamp.lights[0];
        let dim = |value: f32| (Screen::LightDim { light, on: true, value: value.round() as u8 }, value / 100.0);
        let zone = |value: f32| {
            let target = (value as f64 * 2.0).round() / 2.0;
            (Screen::NestSet { nest: std::borrow::Cow::Owned(Nest::of_card(kitchen)), target, pulse: None, setting: true, shown: None }, ((value as f64 - kitchen.min) / (kitchen.max - kitchen.min)) as f32)
        };
        let volume = |value: f32| (Screen::Volume { card: speaker, value: value.round() as u8 }, value / 100.0);
        let thermostat = nest(20.0, 21.0);
        let set = |value: f32| {
            let target = (value as f64 * 2.0).round() / 2.0;
            let card = &thermostat.card;
            (Screen::NestSet { nest: std::borrow::Cow::Borrowed(&thermostat), target, pulse: None, setting: true, shown: None },
             ((value as f64 - card.min) / (card.max - card.min)) as f32)
        };
        if fledge {
            film.caption("Across the room: the time, the weather, what's on");
            glanced(&mut film, &fonts, 300, false);
            film.caption("Walk up: your devices, around the thermostat");
            let start = deck(&fonts, 60, 21.5, 35, 20.0);
            film.fade(&start.frame(&fonts, 0.0, 1.0));
            film.hold(20);
            film.zoom(&fonts, &start, 0, 1.0, zoom::zoomed());
            film.hold(40);
            film.caption("Turn to browse");
            film.turn(&fonts, &start, 0, 1, 22);
            film.hold(30);
            film.caption("Press to open. Turn to dim");
            let dimmed = deck(&fonts, 85, 21.5, 35, 20.0);
            open(&mut film, &fonts, &start, &dimmed, 1, &[(60.0, 0), (22.0, 55), (85.0, 65)], &dim);
            film.caption("Other heating zones");
            film.turn(&fonts, &dimmed, 1, 2, 22);
            film.hold(24);
            let warmer = deck(&fonts, 85, 23.0, 35, 20.0);
            open(&mut film, &fonts, &dimmed, &warmer, 2, &[(21.5, 0), (23.0, 50)], &zone);
            film.caption("Speakers, with what's playing");
            film.turn(&fonts, &warmer, 2, 3, 22);
            film.hold(50);
            let louder = deck(&fonts, 85, 23.0, 52, 20.0);
            open(&mut film, &fonts, &warmer, &louder, 3, &[(35.0, 0), (52.0, 45)], &volume);
            film.caption("Scenes, blinds, and the rest");
            film.turn(&fonts, &louder, 3, 4, 22);
            film.hold(26);
            film.turn(&fonts, &louder, 4, 5, 22);
            film.hold(26);
            film.caption("And the thermostat itself");
            film.turn(&fonts, &louder, 5, 0, 9);
            film.hold(24);
            let heating = deck(&fonts, 85, 23.0, 52, 22.5);
            open(&mut film, &fonts, &louder, &heating, 0, &[(20.0, 0), (22.5, 80)], &set);
            film.hold(20);
            film.caption("Fledge's look, or the Nest's own");
            film.turn(&fonts, &heating, 0, 6, 8);
            film.hold(16);
            film.zoom(&fonts, &heating, 6, zoom::zoomed(), 1.0);
            film.fade(&device::frame(screens::draw(&fonts, &Screen::ThemePick { fledge: true }, None)));
            film.hold(40);
            film.show(&device::frame(screens::draw(&fonts, &Screen::ThemePick { fledge: false }, None)));
            film.hold(40);
        } else {
            let heating = deck(&fonts, 85, 23.0, 52, 22.5);
            film.fade(&heating.frame(&fonts, 6.0, 1.0));
            film.zoom(&fonts, &heating, 6, 1.0, zoom::zoomed());
            film.hold(20);
            film.turn(&fonts, &heating, 6, 3, 26);
            film.hold(40);
            film.turn(&fonts, &heating, 3, 1, 26);
            film.hold(24);
            let softer = deck(&fonts, 40, 23.0, 52, 22.5);
            let lit = parse(85, 23.0, 52, "Slow Tide");
            let Card::Lights(lamp) = &lit.cards[0] else { panic!() };
            let light = &lamp.lights[0];
            let dim = |value: f32| (Screen::LightDim { light, on: true, value: value.round() as u8 }, value / 100.0);
            open(&mut film, &fonts, &heating, &softer, 1, &[(85.0, 0), (40.0, 70)], &dim);
            film.turn(&fonts, &softer, 1, 0, 26);
            film.hold(24);
            let thermostat = nest(22.5, 21.0);
            let set = |value: f32| {
                let target = (value as f64 * 2.0).round() / 2.0;
                let card = &thermostat.card;
                (Screen::NestSet { nest: std::borrow::Cow::Borrowed(&thermostat), target, pulse: None, setting: true, shown: None },
                 ((value as f64 - card.min) / (card.max - card.min)) as f32)
            };
            let settled = deck(&fonts, 40, 23.0, 52, 21.5);
            open(&mut film, &fonts, &softer, &settled, 0, &[(22.5, 0), (19.5, 70), (21.5, 55)], &set);
        }
    }
    // It ends as it began, in Fledge's look.
    paint::set_theme(paint::Theme::Fledge);
    film.caption("Fledge: Home Assistant on your Nest's own screen");
    glanced(&mut film, &fonts, 210, true);
    let captions: String = film.captions.iter().map(|(frame, text)| format!("{frame}\t{text}\n")).collect();
    std::fs::write(format!("{}/captions.txt", film.dir), captions).unwrap();
    println!("{} frames, {:.1} s", film.frames, film.frames as f32 * FRAME);
}
