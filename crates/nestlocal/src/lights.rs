//! Home Assistant's lights on the UNFR0ZEN screen. Home Assistant (its
//! lights blueprint) keeps the list, with each light's state, retained on
//!
//!   <base>/set/lights       [{"id": "light.snug_lamp", "name": "Snug lamp",
//!                             "on": true, "brightness": 60, "dimmable": true}]
//!
//! and republishes it when a light changes. This writes it for the screen,
//! and sends what the screen asks for to Home Assistant on
//!
//!   <base>/lights/command   {"id": "light.snug_lamp", "on": true, "brightness": 60}
//!
//! where a missing brightness turns the light on at its last. Which lights,
//! their names and order all come from Home Assistant: none of it is built
//! into the unit.
//!
//! The screen reads `lights` in settings_ui's directory:
//!
//!   <generation>\n
//!   <on 0|1> <brightness -1..100> <dimmable 0|1> <name>\n   (one per light)
//!
//! and leaves `lights.request`, "<generation> <index> <on 0|1> <brightness>":
//! the whole state it wants for one light. The generation identifies the
//! list (entities, names, order), so a request made against an older one
//! is dropped rather than sent to whichever light now has its index.

use std::sync::Mutex;

use serde_json::{json, Value};

use crate::{logf, settings_ui};

/// The screen's limits: lights listed, and bytes of a name.
pub const MAX: usize = 6;
const NAME: usize = 31;

#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub id: String,
    pub name: String,
    pub on: bool,
    /// Percent; None while Home Assistant doesn't know (an off light).
    pub brightness: Option<u8>,
    pub dimmable: bool,
}

#[derive(Default)]
pub struct Lights {
    list: Mutex<(u32, Vec<Light>)>,
}

impl Lights {
    /// Takes Home Assistant's list and writes it for the screen. An empty
    /// payload (the retained list cleared) removes the Lights choice.
    pub fn configure(&self, payload: &[u8]) -> Result<(), String> {
        let lights = parse(payload)?;
        let generation = generation(&lights);
        let text = screen(generation, &lights);
        *self.list.lock().unwrap() = (generation, lights);
        let written = if text.is_empty() {
            match std::fs::remove_file(std::path::Path::new(settings_ui::DIR).join("lights")) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            settings_ui::publish("lights", text.as_bytes())
        };
        written.map_err(|e| format!("lights: {e}"))
    }

    /// The command for one of the screen's requests, if it is for the list
    /// Home Assistant last sent.
    pub fn command(&self, request: &str) -> Result<Value, String> {
        let fields: Vec<&str> = request.split_whitespace().collect();
        let [generation, index, on, brightness] = fields[..] else {
            return Err(format!("unexpected request {request:?}"));
        };
        let parse = |field: &str| field.parse::<i64>().map_err(|_| format!("unexpected request {request:?}"));
        let (generation, index, on, brightness) = (parse(generation)?, parse(index)?, parse(on)?, parse(brightness)?);
        let list = self.list.lock().unwrap();
        if generation != list.0 as i64 {
            return Err("the list changed since the screen showed it".into());
        }
        let light = usize::try_from(index).ok().and_then(|i| list.1.get(i))
            .ok_or_else(|| format!("no light {index}"))?;
        let mut command = json!({"id": light.id, "on": on != 0});
        if on != 0 && light.dimmable && (1..=100).contains(&brightness) {
            command["brightness"] = brightness.into();
        }
        Ok(command)
    }
}

fn parse(payload: &[u8]) -> Result<Vec<Light>, String> {
    if payload.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_slice(payload).map_err(|_| "expected a JSON list of lights")?;
    let items = value.as_array().ok_or("expected a JSON list of lights")?;
    if items.len() > MAX {
        logf!("lights: the screen lists {MAX}, not {}", items.len());
    }
    items.iter().take(MAX).map(|item| {
        let id = item.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())
            .ok_or("each light needs an id")?;
        let name = item.get("name").and_then(Value::as_str).unwrap_or(id);
        let brightness = item.get("brightness").and_then(Value::as_f64)
            .filter(|b| (0.0..=100.0).contains(b)).map(|b| b.round() as u8);
        Ok(Light {
            id: id.to_string(),
            name: clean(name, id),
            on: item.get("on").and_then(Value::as_bool).unwrap_or(false),
            brightness,
            dimmable: item.get("dimmable").and_then(Value::as_bool).unwrap_or(false),
        })
    }).collect()
}

/// One line of the screen's text, at most NAME bytes, whole characters.
fn clean(name: &str, id: &str) -> String {
    let flat: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = match flat.trim() {
        "" => id,
        trimmed => trimmed,
    };
    let mut end = flat.len().min(NAME);
    while !flat.is_char_boundary(end) {
        end -= 1;
    }
    flat[..end].to_string()
}

/// FNV-1a of the entities and names in order, below 10^9 so the screen
/// reads it as a plain int.
fn generation(lights: &[Light]) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for light in lights {
        for byte in light.id.bytes().chain([0]).chain(light.name.bytes()).chain([0]) {
            hash = (hash ^ byte as u32).wrapping_mul(0x01000193);
        }
    }
    hash % 1_000_000_000
}

fn screen(generation: u32, lights: &[Light]) -> String {
    if lights.is_empty() {
        return String::new();
    }
    let mut text = format!("{generation}\n");
    for light in lights {
        let brightness = light.brightness.map_or(-1, i32::from);
        text += &format!("{} {brightness} {} {}\n", light.on as u8, light.dimmable as u8, light.name);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(payload: &str) -> Lights {
        let lights = Lights::default();
        let parsed = parse(payload.as_bytes()).unwrap();
        *lights.list.lock().unwrap() = (generation(&parsed), parsed);
        lights
    }

    #[test]
    fn home_assistants_list_becomes_the_screens_text() {
        let lights = parse(br#"[{"id": "light.snug", "name": "Snug lamp", "on": true, "brightness": 59.6, "dimmable": true},
                                {"id": "switch.hall", "name": "Hall", "on": false}]"#).unwrap();
        let g = generation(&lights);
        assert_eq!(screen(g, &lights), format!("{g}\n1 60 1 Snug lamp\n0 -1 0 Hall\n"));
        assert!(g < 1_000_000_000);
        assert_eq!(screen(0, &[]), "");
        assert_eq!(parse(b"  ").unwrap(), vec![]);
        assert!(parse(b"{}").is_err());
        assert!(parse(br#"[{"name": "no id"}]"#).is_err());
    }

    #[test]
    fn names_fit_one_line_of_the_screen() {
        assert_eq!(clean("Snug\nlamp", "x"), "Snug lamp");
        assert_eq!(clean("  ", "light.x"), "light.x");
        assert_eq!(clean("A very long light name that does not fit", "x").len(), NAME);
        // Truncation keeps whole characters.
        assert_eq!(clean(&"é".repeat(20), "x"), "é".repeat(15));
    }

    #[test]
    fn the_list_is_capped_at_what_the_screen_shows() {
        let many: Vec<String> = (0..9).map(|i| format!(r#"{{"id": "light.l{i}"}}"#)).collect();
        assert_eq!(parse(format!("[{}]", many.join(",")).as_bytes()).unwrap().len(), MAX);
    }

    #[test]
    fn the_generation_follows_the_list_not_the_state() {
        let a = parse(br#"[{"id": "light.a", "name": "A", "on": true}]"#).unwrap();
        let b = parse(br#"[{"id": "light.a", "name": "A", "on": false, "brightness": 3}]"#).unwrap();
        let renamed = parse(br#"[{"id": "light.a", "name": "B"}]"#).unwrap();
        assert_eq!(generation(&a), generation(&b));
        assert_ne!(generation(&a), generation(&renamed));
    }

    #[test]
    fn requests_become_commands_for_the_current_list() {
        let lights = configured(r#"[{"id": "light.snug", "dimmable": true}, {"id": "switch.hall"}]"#);
        let g = lights.list.lock().unwrap().0;
        assert_eq!(lights.command(&format!("{g} 0 1 60\n")).unwrap(), json!({"id": "light.snug", "on": true, "brightness": 60}));
        assert_eq!(lights.command(&format!("{g} 0 1 -1")).unwrap(), json!({"id": "light.snug", "on": true}));
        assert_eq!(lights.command(&format!("{g} 0 0 -1")).unwrap(), json!({"id": "light.snug", "on": false}));
        // A brightness for an on/off light is not passed on.
        assert_eq!(lights.command(&format!("{g} 1 1 40")).unwrap(), json!({"id": "switch.hall", "on": true}));
        assert!(lights.command(&format!("{} 0 1 60", g + 1)).is_err());
        assert!(lights.command(&format!("{g} 2 1 60")).is_err());
        assert!(lights.command(&format!("{g} -1 1 60")).is_err());
        assert!(lights.command("garbage").is_err());
    }
}
