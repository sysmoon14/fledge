//! Home Assistant's cards, as its nest_ui blueprint keeps them retained on
//! <base>/set/ui, and the commands the cards send back on <base>/ui/command.

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Cards {
    #[serde(default)]
    pub cards: Vec<Card>,
    /// What the thermostat wakes to: "chooser" (the default), "thermostat",
    /// or a card's place in `cards`, opened.
    #[serde(default)]
    wake: Option<serde_json::Value>,
}

/// What waking shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wake {
    /// The cards, zoomed out on the thermostat.
    Chooser,
    /// The thermostat, its ring setting the temperature: the Nest as it was.
    Thermostat,
    /// A card, opened.
    Card(usize),
}

impl Cards {
    pub fn wake(&self) -> Wake {
        match &self.wake {
            Some(Value::String(s)) if s == "thermostat" => Wake::Thermostat,
            Some(Value::Number(n)) => match n.as_u64() {
                Some(i) if (i as usize) < self.cards.len() => Wake::Card(i as usize),
                _ => Wake::Chooser,
            },
            _ => Wake::Chooser,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Card {
    Lights(LightsCard),
    Climate(ClimateCard),
    Media(MediaCard),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct LightsCard {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub lights: Vec<Light>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Light {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub on: bool,
    #[serde(default)]
    pub dimmable: bool,
    /// Percent; None while Home Assistant doesn't know (an off light).
    #[serde(default)]
    pub brightness: Option<u8>,
    #[serde(default)]
    pub rgb: Option<[u8; 3]>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ClimateCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    /// heat, cool, heat_cool, off, ...
    #[serde(default)]
    pub mode: String,
    /// heating, cooling, idle, off, ...
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub target: Option<f64>,
    #[serde(default)]
    pub current: Option<f64>,
    #[serde(default = "min_temp")]
    pub min: f64,
    #[serde(default = "max_temp")]
    pub max: f64,
    #[serde(default = "temp_step")]
    pub step: f64,
}

fn min_temp() -> f64 { 7.0 }
fn max_temp() -> f64 { 30.0 }
fn temp_step() -> f64 { 0.5 }

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct MediaCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    /// playing, paused, idle, off, ...
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    /// Percent.
    #[serde(default)]
    pub volume: Option<u8>,
}

impl Cards {
    /// Home Assistant's payload; an empty one (the retained cards cleared)
    /// is no cards, which leaves the thermostat as it is.
    pub fn parse(payload: &[u8]) -> Result<Cards, String> {
        if payload.iter().all(u8::is_ascii_whitespace) {
            return Ok(Cards::default());
        }
        let mut cards: Cards = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
        cards.cards.retain(|card| match card {
            Card::Lights(lights) => !lights.lights.is_empty(),
            _ => true,
        });
        Ok(cards)
    }
}

impl Light {
    /// The colour the light shows in: its own while on, Home Assistant's
    /// rgb_color (which it gives colour-temperature lights too), lifted so a
    /// dim colour still reads on black; warm white when it has none.
    pub fn colour(&self) -> [u8; 3] {
        let [r, g, b] = self.rgb.unwrap_or(WARM);
        let top = r.max(g).max(b).max(1) as u32;
        if top >= 170 {
            return [r, g, b];
        }
        let lift = |c: u8| ((c as u32 * 200 / top).min(255)) as u8;
        [lift(r), lift(g), lift(b)]
    }
}

/// A 2700 K lamp's colour, for lights that report none.
pub const WARM: [u8; 3] = [255, 169, 87];

/// One change the thermostat asks of Home Assistant.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    On(String),
    Off(String),
    Brightness(String, u8),
    Temperature(String, f64),
    Volume(String, u8),
    PlayPause(String),
}

impl Command {
    pub fn payload(&self) -> Value {
        match self {
            Command::On(id) => json!({"id": id, "action": "on"}),
            Command::Off(id) => json!({"id": id, "action": "off"}),
            Command::Brightness(id, b) => json!({"id": id, "action": "brightness", "value": b}),
            Command::Temperature(id, t) => json!({"id": id, "action": "temperature", "value": t}),
            Command::Volume(id, v) => json!({"id": id, "action": "volume", "value": v}),
            Command::PlayPause(id) => json!({"id": id, "action": "play_pause"}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_blueprints_cards() {
        let payload = br#"{"cards":[
            {"kind":"lights","name":"Bedroom","icon":"mdi:lightbulb-group","lights":[
                {"id":"light.reading_lamp","name":"Reading Lamp","icon":"m3s:table-lamp","on":true,
                 "dimmable":true,"brightness":60,"rgb":[255,169,87]},
                {"id":"switch.fan","name":"Fan","icon":"","on":false,"dimmable":false,"brightness":null,"rgb":null}]},
            {"kind":"climate","id":"climate.living","name":"Living room","icon":"","mode":"heat",
             "action":"idle","target":19,"current":19.5,"min":7,"max":30,"step":0.5},
            {"kind":"media","id":"media_player.kitchen","name":"Kitchen","icon":"","state":"playing",
             "title":"Mix","artist":"","volume":35},
            {"kind":"lights","name":"Empty","icon":"","lights":[]}]}"#;
        let cards = Cards::parse(payload).unwrap();
        assert_eq!(cards.cards.len(), 3, "a lights card with no lights is dropped");
        let Card::Lights(lights) = &cards.cards[0] else { panic!() };
        assert_eq!(lights.lights[0].brightness, Some(60));
        assert_eq!(lights.lights[1].rgb, None);
        let Card::Climate(climate) = &cards.cards[1] else { panic!() };
        assert_eq!(climate.target, Some(19.0));
        assert_eq!(Cards::parse(b"").unwrap(), Cards::default());
        assert_eq!(cards.wake(), Wake::Chooser, "the chooser by default");
        let wake = |w: &str| Cards::parse(format!(r#"{{"cards":[{{"kind":"climate","id":"climate.x"}}],"wake":{w}}}"#).as_bytes()).unwrap().wake();
        assert_eq!((wake(r#""thermostat""#), wake("0"), wake("5"), wake(r#""chooser""#)),
                   (Wake::Thermostat, Wake::Card(0), Wake::Chooser, Wake::Chooser));
        assert!(Cards::parse(b"{").is_err());
    }

    #[test]
    fn a_dim_colour_is_lifted_and_none_is_warm() {
        let mut light = Light { id: "light.x".into(), name: String::new(), icon: String::new(), on: true,
                                dimmable: true, brightness: Some(10), rgb: Some([40, 0, 20]) };
        assert_eq!(light.colour(), [200, 0, 100]);
        light.rgb = Some([255, 249, 253]);
        assert_eq!(light.colour(), [255, 249, 253]);
        light.rgb = None;
        assert_eq!(light.colour(), WARM);
    }

    #[test]
    fn commands_name_the_entity_and_action() {
        assert_eq!(Command::Brightness("light.x".into(), 40).payload(),
                   json!({"id": "light.x", "action": "brightness", "value": 40}));
        assert_eq!(Command::PlayPause("media_player.k".into()).payload()["action"], "play_pause");
    }
}
