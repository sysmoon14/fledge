//! nestlocal's own start options (/etc/nestlocal/options, which
//! nestlocal.init reads, else /media/system-config/nestlocal.options), changed
//! from Home Assistant: the external temperature option, which only applies
//! from a start of both nestlocal and nlclient, so the unit reboots into it.

use std::io;
use std::path::{Path, PathBuf};

use crate::logf;

const OPTIONS: &str = "/etc/nestlocal/options";
const FALLBACK: &str = "/media/system-config/nestlocal.options";
/// The flag the switch adds or takes away. Its -store and -timeout
/// companions are left as they are.
pub const EXTERNAL_TEMPERATURE: &str = "external-temperature";

/// The file nestlocal.init starts nestlocal with.
pub fn path() -> PathBuf {
    if Path::new(OPTIONS).exists() { PathBuf::from(OPTIONS) } else { PathBuf::from(FALLBACK) }
}

/// What a token sets of a boolean `flag`: Some(on), or None for any other
/// token (another flag, a value, or flag-something).
fn sets(token: &str, flag: &str) -> Option<bool> {
    let name = token.strip_prefix("--").or_else(|| token.strip_prefix('-'))?;
    if name == flag {
        return Some(true);
    }
    match name.strip_prefix(flag)?.strip_prefix('=')? {
        "true" | "1" | "t" | "T" | "TRUE" | "True" => Some(true),
        _ => Some(false),
    }
}

/// Whether the options turn a boolean `flag` on: its last mention wins, as
/// Go's flag package (and flags.rs) read them.
pub fn has(text: &str, flag: &str) -> bool {
    text.split_whitespace().filter_map(|t| sets(t, flag)).last().unwrap_or(false)
}

/// The options with `flag` on or off, everything else as it was.
pub fn with(text: &str, flag: &str, on: bool) -> String {
    let mut lines: Vec<String> = text.lines()
        .map(|line| line.split_whitespace().filter(|t| sets(t, flag).is_none()).collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    if on {
        match lines.last_mut() {
            Some(last) => last.push_str(&format!(" -{flag}")),
            None => lines.push(format!("-{flag}")),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Turns `flag` on or off in the options at `file`, keeping the first
/// version as <file>.before-<flag>. Whether anything changed.
pub fn set(file: &Path, flag: &str, on: bool) -> io::Result<bool> {
    let text = std::fs::read_to_string(file)?;
    if has(&text, flag) == on {
        return Ok(false);
    }
    let backup = file.with_file_name(format!(
        "{}.before-{flag}", file.file_name().and_then(|n| n.to_str()).unwrap_or("options")));
    if !backup.exists() {
        std::fs::copy(file, &backup)?;
    }
    let new = file.with_extension("new");
    std::fs::write(&new, with(&text, flag, on))?;
    std::fs::File::open(&new)?.sync_all()?;
    std::fs::rename(&new, file)?;
    Ok(true)
}

/// Whether the unit starts with the external temperature option now.
pub fn external_temperature() -> bool {
    std::fs::read_to_string(path()).is_ok_and(|text| has(&text, EXTERNAL_TEMPERATURE))
}

/// Reboots the unit in a few seconds, detached: nestlocal.init then starts
/// nestlocal with its new options before nlclient.
pub fn reboot_soon(why: &str) {
    logf!("nestlocal: rebooting for {why}");
    let _ = crate::wake::stay_awake(crate::wake::MAX_STAY_AWAKE);
    let status = std::process::Command::new("sh")
        .args(["-c", "(sync; sleep 3; reboot) > /dev/null 2>&1 &"]).status();
    if let Err(e) = status {
        logf!("nestlocal: reboot: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAG: &str = EXTERNAL_TEMPERATURE;

    #[test]
    fn reads_the_flag_as_go_does() {
        assert!(has("-mqtt tcp://b:1883 -external-temperature\n", FLAG));
        assert!(has("--external-temperature=true", FLAG));
        assert!(!has("-external-temperature -external-temperature=false", FLAG), "the last wins");
        assert!(!has("-external-temperature-timeout 30m -external-temperature-store /x", FLAG), "only its companions");
        assert!(!has("", FLAG));
    }

    #[test]
    fn turns_it_on_and_off_leaving_the_rest() {
        let text = "-mqtt tcp://b:1883 -mqtt-user u\n-external-temperature-timeout 20m\n";
        let on = with(text, FLAG, true);
        assert_eq!(on, "-mqtt tcp://b:1883 -mqtt-user u\n-external-temperature-timeout 20m -external-temperature\n");
        assert!(has(&on, FLAG));
        assert_eq!(with(&on, FLAG, false), text);
        assert_eq!(with("-a --external-temperature=false -b\n", FLAG, true), "-a -b -external-temperature\n");
        assert_eq!(with("", FLAG, true), "-external-temperature\n");
    }

    #[test]
    fn writes_the_file_keeping_the_first_version() {
        let dir = std::env::temp_dir().join(format!("nestlocal-options-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("options");
        std::fs::write(&file, "-mqtt tcp://b:1883\n").unwrap();
        assert!(set(&file, FLAG, true).unwrap());
        assert!(!set(&file, FLAG, true).unwrap(), "already on: nothing to do");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "-mqtt tcp://b:1883 -external-temperature\n");
        assert!(set(&file, FLAG, false).unwrap());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "-mqtt tcp://b:1883\n");
        assert_eq!(std::fs::read_to_string(dir.join("options.before-external-temperature")).unwrap(), "-mqtt tcp://b:1883\n");
        assert!(!dir.join("options.new").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
