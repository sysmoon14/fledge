//! A Works With Nest boiler. Firmware 6.4-8 on a gen3 Display-3.x or
//! Display-5.x expects a Heat Link, which only connects through the radio
//! nestlocal holds with Zigbee or Thread. nlclient skips that check for a
//! Works With Nest boiler, which it reads at start from using_wwn_boiler in
//! its settings file: unfr0zen's rcS sets it before nlclient starts on a
//! Heat Link model when the run asked for it (--wwn-boiler). With it the unit
//! heats through W1 and keeps its safety temperature, but reports can_heat
//! false (bridge.rs reports it true).

//!
//! Home Assistant can choose it too (its Works With Nest boiler switch,
//! commands.rs): the choice is kept in CHOICE with a fingerprint of rcS, and
//! the unit reboots; -prepare-settings, before nlclient starts and after
//! rcS's own block, applies it. Flashed again since (rcS changed), the
//! flasher's choice stands and the kept one is dropped.

use std::io;
use std::path::Path;

use crate::nlsettings;

pub const KEY: &str = "using_wwn_boiler";
/// Home Assistant's choice: "1" or "0", and rcS's fingerprint when it chose.
pub const CHOICE: &str = "/media/user-config/.fledge-wwn-boiler";
/// What the flasher's rcS block marks a setting it made with (patch.rs):
/// gone, a flash without the option leaves the setting alone.
pub const MARKER: &str = "/media/user-config/.unfr0zen-wwn-boiler";
pub const RCS: &str = "/etc/init.d/rcS";

/// Whether a model has a Heat Link to miss: gen3 Display-3.x and Display-5.x.
pub fn heat_link_model(model: &str) -> bool {
    model.starts_with("Display-3.") || model.starts_with("Display-5.")
}

/// FNV-1a: the same on every build, unlike std's hasher.
fn fingerprint(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf29ce484222325, |h, c| (h ^ *c as u64).wrapping_mul(0x100000001b3))
}

/// Keeps Home Assistant's choice, for the next start of nlclient.
pub fn choose(choice: &Path, rcs: &Path, on: bool) -> io::Result<()> {
    let print = fingerprint(&std::fs::read(rcs).unwrap_or_default());
    nlsettings::write(choice, format!("{} {print:016x}\n", u8::from(on)).as_bytes())
}

/// Home Assistant's choice waiting to apply, if it made one (and the unit
/// hasn't been flashed since).
pub fn chosen(choice: &Path, rcs: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(choice).ok()?;
    let (on, print) = text.trim().split_once(' ')?;
    let now = format!("{:016x}", fingerprint(&std::fs::read(rcs).unwrap_or_default()));
    (print == now).then_some(on == "1")
}

/// What prepare did.
#[derive(Debug, PartialEq)]
pub enum Prepared {
    /// Home Assistant never chose.
    Nothing,
    /// Flashed since it chose: the flasher's choice stands.
    Flashed,
    /// Set on or off.
    Applied(bool),
}

/// Before nlclient starts: Home Assistant's choice in its settings.
pub fn prepare(settings: &Path, choice: &Path, rcs: &Path, marker: &Path) -> io::Result<Prepared> {
    if !choice.exists() {
        return Ok(Prepared::Nothing);
    }
    let Some(on) = chosen(choice, rcs) else {
        std::fs::remove_file(choice)?;
        return Ok(Prepared::Flashed);
    };
    let old = std::fs::read(settings).or_else(|e| match e.kind() {
        io::ErrorKind::NotFound => Ok(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?>\n<settings>\n</settings>\n".to_vec()),
        _ => Err(e),
    })?;
    // One entry, however many a hand edit left.
    let mut new = nlsettings::remove(&old, KEY);
    if on {
        new = nlsettings::set(&new, KEY, "1")?;
    } else {
        // Nothing left for a flash without the option to take back.
        match std::fs::remove_file(marker) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    if new != old {
        nlsettings::write(settings, &new)?;
    }
    Ok(Prepared::Applied(on))
}

/// Whether the settings file at path has using_wwn_boiler on, as nlclient
/// reads it: a number other than 0.
pub fn using(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|b| on(&b))
}

fn on(b: &[u8]) -> bool {
    nlsettings::value(b, KEY).and_then(|v| std::str::from_utf8(v).ok()?.trim().parse::<i64>().ok()).is_some_and(|v| v != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("nestlocal-heatlink-{}-{}", std::process::id(),
                                                   N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn home_assistant_s_choice_applies_before_nlclient_starts() {
        let d = dir();
        let (settings, choice, rcs, marker) = (d.join("settings.config"), d.join("choice"), d.join("rcS"), d.join("marker"));
        std::fs::write(&rcs, "rcS as flashed\n").unwrap();
        // A hand edit run twice left two.
        let twice = "<settings>\n  <s key=\"a\" value=\"1\"/>\n  <s key=\"using_wwn_boiler\" value=\"1\"/>\n  <s key=\"using_wwn_boiler\" value=\"1\"/>\n</settings>\n";
        std::fs::write(&settings, twice).unwrap();
        assert_eq!(prepare(&settings, &choice, &rcs, &marker).unwrap(), Prepared::Nothing);
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), twice, "untouched without a choice");

        std::fs::write(&marker, "").unwrap();
        choose(&choice, &rcs, false).unwrap();
        assert_eq!(chosen(&choice, &rcs), Some(false));
        assert_eq!(prepare(&settings, &choice, &rcs, &marker).unwrap(), Prepared::Applied(false));
        assert!(!using(&settings) && !marker.exists());
        assert!(std::fs::read_to_string(&settings).unwrap().contains(r#"<s key="a" value="1"/>"#), "the rest kept");

        choose(&choice, &rcs, true).unwrap();
        assert_eq!(prepare(&settings, &choice, &rcs, &marker).unwrap(), Prepared::Applied(true));
        let text = std::fs::read_to_string(&settings).unwrap();
        assert!(using(&settings) && text.matches(KEY).count() == 1);
        assert_eq!(prepare(&settings, &choice, &rcs, &marker).unwrap(), Prepared::Applied(true), "every start");

        std::fs::write(&rcs, "rcS flashed again\n").unwrap();
        assert_eq!(chosen(&choice, &rcs), None);
        assert_eq!(prepare(&settings, &choice, &rcs, &marker).unwrap(), Prepared::Flashed);
        assert!(!choice.exists() && using(&settings), "the flasher's choice stands");
    }

    #[test]
    fn a_missing_settings_file_gets_one() {
        let d = dir();
        let (settings, choice, rcs) = (d.join("settings.config"), d.join("choice"), d.join("rcS"));
        choose(&choice, &rcs, true).unwrap();
        assert_eq!(prepare(&settings, &choice, &rcs, &d.join("marker")).unwrap(), Prepared::Applied(true));
        assert!(using(&settings));
    }

    #[test]
    fn heat_link_models() {
        assert!(heat_link_model("Display-3.4") && heat_link_model("Display-5.1"));
        assert!(!heat_link_model("Display-2.9") && !heat_link_model(""));
    }

    #[test]
    fn zero_is_off() {
        assert!(!on(br#"<s key="using_wwn_boiler" value="0"/>"#) && on(br#"<s key="using_wwn_boiler" value="1"/>"#));
        assert!(!on(b"<settings/>") && !using(Path::new("/nonexistent/settings.config")));
    }
}
