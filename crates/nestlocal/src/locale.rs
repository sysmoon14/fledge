//! The words nestlocal draws on the thermostat's screen, in the language the
//! Nest is set to. English (locales/en.json) is the source catalog and the
//! fallback for any message a translation lacks; locales/README.md says how
//! to add a language.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;

type Catalog = HashMap<String, String>;

/// Every embedded catalog by its language code, English first.
const SOURCES: [(&str, &str); 2] = [
    ("en", include_str!("../locales/en.json")),
    ("nl", include_str!("../locales/nl.json")),
];

/// The key of nlclient's settings file that names the language the Nest is
/// set to: `<s key="current_locale" value="nl_NL"/>`.
const SETTINGS_KEY: &str = "current_locale";

fn catalogs() -> &'static [(&'static str, Catalog)] {
    static CATALOGS: OnceLock<Vec<(&'static str, Catalog)>> = OnceLock::new();
    CATALOGS.get_or_init(|| SOURCES.iter().map(|(code, json)| {
        (*code, serde_json::from_str(json).unwrap_or_else(|e| panic!("locales/{code}.json: {e}")))
    }).collect())
}

fn english() -> &'static Catalog {
    &catalogs()[0].1
}

/// The screen's catalog, as start chose it.
static ACTIVE: OnceLock<&'static Catalog> = OnceLock::new();

/// The language part of a locale name, lowercased: "nl" for nl_NL, nl-NL,
/// nl_NL.UTF-8 or NL.
fn language_code(locale: &str) -> Option<String> {
    let code: String = locale.trim().chars().take_while(char::is_ascii_alphabetic).collect();
    (2..=3).contains(&code.len()).then(|| code.to_ascii_lowercase())
}

/// The Nest's language as its settings file names it.
fn from_settings(settings: &[u8]) -> Option<String> {
    crate::nlsettings::value(settings, SETTINGS_KEY)
        .and_then(|value| language_code(&String::from_utf8_lossy(value)))
}

/// Chooses the screen's language once, before anything is drawn: `wanted`
/// (-language) if set, else the Nest's, from its settings file. Returns the
/// language chosen and where it came from; English when there is no
/// catalog for it.
pub fn start(wanted: &str, settings_path: &str) -> (&'static str, String) {
    let (code, catalog, from) = choose(wanted, settings_path, || std::fs::read(settings_path));
    let _ = ACTIVE.set(catalog);
    (code, from)
}

/// start's choice, the settings file read only when -language names none.
fn choose(wanted: &str, settings_path: &str, read: impl FnOnce() -> std::io::Result<Vec<u8>>)
          -> (&'static str, &'static Catalog, String) {
    let (asked, from) = match language_code(wanted) {
        Some(code) => (Some(code), "-language".to_owned()),
        None => match read() {
            Ok(settings) => match from_settings(&settings) {
                Some(code) => (Some(code), settings_path.to_owned()),
                None => (None, format!("{settings_path} names none")),
            },
            Err(e) => (None, format!("{settings_path}: {e}")),
        },
    };
    let found = asked.as_deref().and_then(|code| catalogs().iter().find(|(c, _)| *c == code));
    match (found, asked) {
        (Some((code, catalog)), _) => (code, catalog, from),
        (None, Some(asked)) => ("en", english(), format!("{from} asks for {asked}, which has no catalog")),
        (None, None) => ("en", english(), from),
    }
}

/// A message in the screen's language, else in English; a key that is in
/// no catalog (a test catches that) shows as itself.
pub fn text(key: &'static str) -> &'static str {
    find(key).unwrap_or(key)
}

fn find(key: &str) -> Option<&'static str> {
    ACTIVE.get().and_then(|c| c.get(key)).or_else(|| english().get(key)).map(String::as_str)
}

/// A word Home Assistant sends as a value (a climate mode, a weather
/// condition), in the screen's language: the message `prefix.value`, if a
/// catalog has it.
pub fn named(prefix: &'static str, value: &str) -> Option<&'static str> {
    find(&format!("{prefix}.{value}"))
}

/// The `i`th of a numbered set of words (the days, Sunday 0; the months,
/// January 0) in the screen's language: the message `prefix.i`, shown as
/// its key if no catalog has it, as text does.
pub fn numbered(prefix: &'static str, i: usize) -> String {
    let key = format!("{prefix}.{i}");
    find(&key).map_or(key, str::to_owned)
}

/// Substitutes named arguments without interpreting braces in their values.
pub fn format(key: &'static str, arguments: &[(&str, &dyn Display)]) -> String {
    render(text(key), arguments)
}

/// A placeholder without its argument, or a brace without its pair, stays as
/// written: a translation's slip shows on the screen rather than stopping
/// nestlocal.
fn render(mut template: &str, arguments: &[(&str, &dyn Display)]) -> String {
    use std::fmt::Write;
    let mut output = String::new();
    while let Some((before, rest)) = template.split_once('{') {
        output.push_str(before);
        let Some((name, after)) = rest.split_once('}') else {
            output.push('{');
            template = rest;
            continue;
        };
        let _ = match arguments.iter().find(|(argument, _)| *argument == name) {
            Some((_, value)) => write!(output, "{value}"),
            None => write!(output, "{{{name}}}"),
        };
        template = after;
    }
    output.push_str(template);
    output
}

/// A message in the screen's language, its named arguments substituted.
#[macro_export]
macro_rules! tr {
    ($key:expr $(,)?) => { $crate::locale::text($key) };
    ($key:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::locale::format($key, &[$((stringify!($name), &$value as &dyn std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{').skip(1).filter_map(|rest| rest.split_once('}').map(|(name, _)| name)).collect()
    }

    /// Every .rs file under `dir`, with its contents.
    fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((path.display().to_string(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }

    /// The string literal each `call` in `source` starts with: `call` as a
    /// whole name, so tr!(" is not include_str!(".
    fn literals<'a>(source: &'a str, call: &str) -> Vec<&'a str> {
        source.match_indices(call)
            .filter(|(at, _)| !source[..*at].ends_with(|c: char| c.is_alphanumeric() || c == '_'))
            .map(|(at, _)| source[at + call.len()..].split_once('"').expect("a literal key").0)
            .collect()
    }

    #[test]
    fn named_arguments_can_be_reordered_and_repeated() {
        assert_eq!(render("{count} {device}: {device}", &[("device", &"Nest"), ("count", &2)]), "2 Nest: Nest");
    }

    #[test]
    fn argument_values_are_not_interpreted_as_templates() {
        assert_eq!(render("File: {path}", &[("path", &"/{count}/x")]), "File: /{count}/x");
    }

    #[test]
    fn a_locale_name_gives_its_language() {
        for name in ["nl_NL", "nl-NL", "nl_NL.UTF-8", "NL", " nl "] {
            assert_eq!(language_code(name).as_deref(), Some("nl"), "{name}");
        }
        assert_eq!(language_code(""), None);
        assert_eq!(language_code("x"), None);
    }

    #[test]
    fn the_nest_s_language_comes_from_its_settings_file() {
        let settings = br#"<settings>
  <s key="structure_id" value="abc"/>
  <s key="current_locale" value="nl_NL"/>
</settings>"#;
        assert_eq!(from_settings(settings).as_deref(), Some("nl"));
        assert_eq!(from_settings(br#"<settings><s key="current_locale" value="fr_CA"/></settings>"#).as_deref(), Some("fr"));
        assert_eq!(from_settings(br#"<settings><s key="locale" value="nl_NL"/></settings>"#), None);
        assert_eq!(from_settings(b"<settings/>"), None);
    }

    #[test]
    fn the_choice_says_where_it_came_from() {
        let path = "/media/user-config/settings.config";
        let nest = |value: &str| {
            let settings = format!(r#"<s key="current_locale" value="{value}"/>"#).into_bytes();
            move || Ok(settings)
        };
        let said = |(code, _, from): (&'static str, &'static Catalog, String)| (code, from);
        assert_eq!(said(choose("", path, nest("nl_NL"))), ("nl", path.to_owned()));
        assert_eq!(said(choose("en", path, nest("nl_NL"))), ("en", "-language".to_owned()));
        assert_eq!(said(choose("", path, nest("fr_FR"))), ("en", format!("{path} asks for fr, which has no catalog")));
        assert_eq!(said(choose("", path, || Ok(b"<settings/>".to_vec()))), ("en", format!("{path} names none")));
        let unread = || Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory"));
        assert_eq!(said(choose("", path, unread)), ("en", format!("{path}: No such file or directory")));
    }

    #[test]
    fn every_catalog_has_english_s_messages_and_placeholders() {
        for (code, catalog) in catalogs() {
            for (key, text) in catalog {
                let source = english().get(key).unwrap_or_else(|| panic!("{code}.json: {key} is not in en.json"));
                assert_eq!(placeholders(text), placeholders(source), "{code}.json: {key}'s placeholders");
                assert!(!text.trim().is_empty(), "{code}.json: {key} is empty");
            }
            for key in english().keys() {
                assert!(catalog.contains_key(key), "{code}.json lacks {key}");
            }
        }
    }

    #[test]
    fn every_message_drawn_is_in_the_catalog_and_every_message_is_drawn() {
        let mut files = Vec::new();
        sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let mut used = BTreeSet::new();
        for (path, source) in files.iter().filter(|(path, _)| !path.ends_with("locale.rs")) {
            for key in literals(source, "tr!(\"") {
                assert!(english().contains_key(key), "{path}: en.json lacks {key}");
                used.insert(key.to_owned());
            }
            // Looked up by a value Home Assistant sends ("climate.heat_cool")
            // or by a number ("day.0"): every message under the prefix.
            for prefix in literals(source, "locale::named(\"").into_iter().chain(literals(source, "locale::numbered(\"")) {
                let under: Vec<&String> = english().keys().filter(|k| k.starts_with(&format!("{prefix}."))).collect();
                assert!(!under.is_empty(), "{path}: en.json has no {prefix}.*");
                used.extend(under.into_iter().cloned());
            }
        }
        for key in english().keys() {
            assert!(used.contains(key), "en.json's {key} is never drawn");
        }
    }
}
