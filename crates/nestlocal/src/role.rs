//! Whether this Nest acts as a thermostat, or is only a panel on the wall
//! (someone heats with something else). Off, it is announced to Home
//! Assistant without its climate entity or anything else that controls
//! heating, its own thermostat card and settings go from its screen, and
//! its heating (and hot water) are switched off, what they were kept in
//! the role's file to put back when it acts as a thermostat again. Its
//! temperature and humidity sensors stay.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{Map, Value};

/// Where the role lives on the unit: there, it is not a thermostat; the
/// file holds what its heating was.
const FILE: &str = "/media/data/fledge-not-thermostat";

pub struct Role {
    file: Mutex<PathBuf>,
    acting: AtomicBool,
}

impl Default for Role {
    fn default() -> Role {
        Role::at(PathBuf::from(FILE))
    }
}

impl Role {
    /// The role kept at `file`.
    pub fn at(file: PathBuf) -> Role {
        let acting = !file.exists();
        Role { file: Mutex::new(file), acting: AtomicBool::new(acting) }
    }

    /// Kept at `file` from now on, as found there (tests: never the unit's).
    #[cfg(test)]
    pub fn move_to(&self, file: PathBuf) {
        self.acting.store(!file.exists(), Ordering::Relaxed);
        *self.file.lock().unwrap() = file;
    }

    /// Whether it acts as a thermostat.
    pub fn acting(&self) -> bool {
        self.acting.load(Ordering::Relaxed)
    }

    /// No longer a thermostat, `was` (what its heating was) kept to put back.
    pub fn stop(&self, was: &Map<String, Value>) -> std::io::Result<()> {
        std::fs::write(&*self.file.lock().unwrap(), Value::Object(was.clone()).to_string())?;
        self.acting.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// A thermostat again: what its heating was when it stopped.
    pub fn start(&self) -> std::io::Result<Map<String, Value>> {
        let was = std::fs::read_to_string(&*self.file.lock().unwrap()).ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| match v { Value::Object(m) => Some(m), _ => None })
            .unwrap_or_default();
        match std::fs::remove_file(&*self.file.lock().unwrap()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        self.acting.store(true, Ordering::Relaxed);
        Ok(was)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    /// A file for a role in a test, not there yet.
    pub fn temp_file() -> PathBuf {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("nestlocal-role-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("role{}", N.fetch_add(1, Ordering::Relaxed)))
    }

    #[test]
    fn stopping_keeps_what_the_heating_was_until_it_starts_again() {
        let file = temp_file();
        let role = Role::at(file.clone());
        assert!(role.acting());
        let was = json!({"mode": "heat", "hot_water_schedule": true});
        role.stop(was.as_object().unwrap()).unwrap();
        assert!(!role.acting() && !Role::at(file.clone()).acting(), "kept across a restart");
        assert_eq!(Value::Object(role.start().unwrap()), was);
        assert!(role.acting() && !file.exists());
        assert!(role.start().unwrap().is_empty(), "starting again finds nothing to put back");
    }
}
