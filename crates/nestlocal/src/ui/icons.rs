//! Icons by Home Assistant's names for them (utils/icons.py builds the set).

use std::collections::HashMap;
use std::sync::OnceLock;

static DEFLATED: &[u8] = include_bytes!("icons.deflate");

/// An icon: its SVG path and the side of its square viewBox, 24 (MDI's
/// 0 0 24 24) or 960 (Material Symbols' 0 -960 960 960).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Icon {
    pub path: &'static str,
    pub box_size: f32,
}

fn all() -> &'static HashMap<&'static str, Icon> {
    static ICONS: OnceLock<HashMap<&'static str, Icon>> = OnceLock::new();
    ICONS.get_or_init(|| {
        let text = miniz_oxide::inflate::decompress_to_vec(DEFLATED).unwrap_or_default();
        let text: &'static str = Box::leak(String::from_utf8(text).unwrap_or_default().into_boxed_str());
        text.lines().filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let (name, size, path) = (parts.next()?, parts.next()?, parts.next()?);
            Some((name, Icon { path, box_size: size.parse().ok()? }))
        }).collect()
    })
}

/// Home Assistant's icon `name` if it is one this carries, else the
/// default for the entity's domain (light, switch, ...).
pub fn lookup(name: &str, entity: &str) -> Option<Icon> {
    let icons = all();
    if let Some(icon) = icons.get(name) {
        return Some(*icon);
    }
    // A filled Material Symbol this lacks: its outline.
    if let Some((prefix, rest)) = name.split_once(':') {
        if prefix.starts_with("m3") && prefix.ends_with('f') {
            if let Some(icon) = icons.get(format!("{}:{rest}", &prefix[..prefix.len() - 1]).as_str()) {
                return Some(*icon);
            }
        }
    }
    let domain = entity.split('.').next().unwrap_or_default();
    let fallback = match domain {
        "light" => "mdi:lightbulb",
        "switch" | "input_boolean" => "mdi:toggle-switch-variant",
        "fan" => "mdi:fan",
        "climate" => "mdi:thermostat",
        "media_player" => "mdi:speaker",
        "lights" => "mdi:lightbulb-group",
        "settings" => "mdi:cog",
        "back" => "mdi:arrow-left",
        _ => return None,
    };
    icons.get(fallback).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_home_assistants_names_and_falls_back_by_domain() {
        let group = lookup("mdi:lightbulb-group", "light.bedroom").unwrap();
        assert_eq!(group.box_size, 24.0);
        let lamp = lookup("m3s:table-lamp", "light.ben_s_lamp").unwrap();
        assert_eq!(lamp.box_size, 960.0);
        assert!(lookup("m3sf:table-lamp", "light.x").is_some());
        assert_eq!(lookup("mdi:no-such-icon", "light.x"), lookup("mdi:lightbulb", ""));
        assert!(lookup("", "switch.kettle").is_some());
        assert!(lookup("", "settings").is_some() && lookup("", "back").is_some());
        assert_eq!(lookup("", "sensor.x"), None);
    }
}
