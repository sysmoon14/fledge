//! What nlclient shows, as nlclient itself tells it.
//!
//! nlclient reports every change of its view, its screen and a screen's
//! active layer to its own telemetry (UiDisplayChange: what changed, and the
//! new thing's name). The screen patch (custom_ui_change in custom_screen.c)
//! routes those reports through a function that keeps the latest name of
//! each thing in STATE:
//!
//! ```text
//! <nlclient's pid> <changes so far>
//! <what changed>\t<its name>          one line each, the latest last
//! ```
//!
//! A file whose pid isn't a running nlclient is an earlier nlclient's (a
//! patch from before this hook writes none): then nothing is known, and the
//! UI falls back on reading nlclient's pixels.

use std::path::Path;

pub const STATE: &str = "/var/run/unfr0zen/nlclient.state";

/// One reading of the file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    pub pid: u32,
    /// Counts up with every change: a change and back between two readings
    /// still shows as a change.
    pub changes: u64,
    /// What changed and its name, the latest last.
    pub things: Vec<(String, String)>,
}

/// What nlclient shows, as far as the UI needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shows {
    /// Its home dial (heating, cooling or off).
    Dial,
    /// Its menu (the ring of choices with Back).
    Menu,
    /// Its Settings, or a screen opened from them.
    Settings,
    /// Anything else: a screen on its way, a prompt, the boot logo.
    Other,
}

impl Report {
    pub fn parse(text: &str) -> Option<Report> {
        let mut lines = text.lines();
        let mut header = lines.next()?.split(' ');
        let pid = header.next()?.parse().ok()?;
        let changes = header.next()?.parse().ok()?;
        let things = lines
            .filter_map(|line| line.split_once('\t'))
            .map(|(what, name)| (what.to_owned(), name.to_owned()))
            .collect();
        Some(Report { pid, changes, things })
    }

    /// The latest name of what changed, if it ever did.
    #[cfg(test)]
    pub fn get(&self, what: &str) -> Option<&str> {
        self.things.iter().rev().find(|(w, _)| w == what).map(|(_, name)| name.as_str())
    }

    pub fn shows(&self) -> Shows {
        classify(self)
    }
}

/// The file, if a running nlclient wrote it.
pub fn read() -> Option<Report> {
    read_from(Path::new(STATE), &|pid| {
        std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|comm| comm.trim_end() == "nlclient")
    })
}

fn read_from(path: &Path, running: &dyn Fn(u32) -> bool) -> Option<Report> {
    let report = Report::parse(&std::fs::read_to_string(path).ok()?)?;
    running(report.pid).then_some(report)
}

/// nlclient's views: its thermostat (the home dial and its menu, layers of
/// nlTempScreen) and its Settings (a carousel of screens).
const THERMOSTAT: &str = "thermostatview";
const SETTINGS: &str = "settingsview";
/// nlTempScreen's layers: the dial (heating, cooling or off alike) and the
/// menu. Its active layer is reported as "active layer (nlTempScreen)" or,
/// chosen by name, "active layer via SelectState (nlTempScreen)".
const DIAL: &str = "temperaturesliderlayer";
const MENU: &str = "selectorlayer";

fn classify(report: &Report) -> Shows {
    let view = report.things.iter().rposition(|(what, _)| what == "current view");
    let layer = report.things.iter().rposition(|(what, _)| what.starts_with("active layer") && what.ends_with("(nlTempScreen)"));
    match view.map(|at| report.things[at].1.as_str()) {
        Some(SETTINGS) => Shows::Settings,
        Some(THERMOSTAT) => match layer {
            // Back in the thermostat (from Settings' Done): it opens on its
            // dial, and reports no layer until one changes.
            Some(layer) if layer > view.unwrap() => match report.things[layer].1.as_str() {
                DIAL => Shows::Dial,
                MENU => Shows::Menu,
                _ => Shows::Other,
            },
            _ => Shows::Dial,
        },
        _ => Shows::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_read_line_by_line() {
        let report = Report::parse("6898 3\ncurrent screen\tlogo\ncurrent view\tthermostatview\n").unwrap();
        assert_eq!((report.pid, report.changes), (6898, 3));
        assert_eq!(report.get("current view"), Some("thermostatview"));
        assert_eq!(report.get("active layer"), None);
        assert_eq!(Report::parse(""), None);
        assert_eq!(Report::parse("6898\n"), None, "no count");
        assert_eq!(Report::parse("x 1\n"), None);
    }

    fn shows(lines: &[(&str, &str)]) -> Shows {
        let things = lines.iter().map(|(w, n)| (w.to_string(), n.to_string())).collect();
        Report { pid: 1, changes: lines.len() as u64, things }.shows()
    }

    // As the unit reported them, in order, with nlclient's own dial, its
    // menu, Settings and Done, its menu again and Back.
    const VIA: &str = "active layer via SelectState (nlTempScreen)";
    const LAYER: &str = "active layer (nlTempScreen)";

    #[test]
    fn starting_up_is_neither_dial_nor_menu() {
        assert_eq!(shows(&[("current view", "startupview"), ("current screen", "logo")]), Shows::Other);
        assert_eq!(shows(&[]), Shows::Other);
    }

    #[test]
    fn the_dial_its_menu_and_back() {
        let dial = [("current screen", "logo"), ("current view", "thermostatview"), (VIA, "temperaturesliderlayer")];
        assert_eq!(shows(&dial), Shows::Dial);
        assert_eq!(shows(&dial[..2]), Shows::Dial, "the thermostat's view, no layer reported yet");
        let menu = [dial.as_slice(), &[(LAYER, "selectorlayer")]].concat();
        assert_eq!(shows(&menu), Shows::Menu);
        let back = [&dial[..2], &[(VIA, "temperaturesliderlayer"), (LAYER, "temperaturesliderlayer")]].concat();
        assert_eq!(shows(&back), Shows::Dial);
        let other = [&dial[..2], &[(LAYER, "ecolayer")]].concat();
        assert_eq!(shows(&other), Shows::Other);
    }

    #[test]
    fn settings_then_done_is_the_dial_though_the_menu_was_the_last_layer() {
        let settings = [(LAYER, "selectorlayer"), (VIA, "temperaturesliderlayer"), ("current view", "settingsview"), ("current screen", "temperature")];
        assert_eq!(shows(&settings), Shows::Settings);
        let done = [(LAYER, "selectorlayer"), (VIA, "temperaturesliderlayer"), ("current screen", "temperature"), ("current view", "thermostatview")];
        assert_eq!(shows(&done), Shows::Dial);
    }

    #[test]
    fn only_a_running_nlclient_s_file_counts() {
        let dir = std::env::temp_dir().join(format!("nlstate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nlclient.state");
        std::fs::write(&path, "42 1\ncurrent view\tthermostatview\n").unwrap();
        assert!(read_from(&path, &|pid| pid == 42).is_some());
        assert!(read_from(&path, &|_| false).is_none(), "an earlier nlclient's");
        assert!(read_from(&dir.join("none"), &|_| true).is_none(), "an older patch writes none");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
