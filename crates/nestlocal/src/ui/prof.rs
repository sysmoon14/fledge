//! Where a frame's time went, for the log's line about frames that missed
//! the display's 60 Hz: each part of drawing adds what it took, and the UI's
//! loop takes the sums frame by frame.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

#[derive(Clone, Copy)]
pub enum Part {
    /// A card drawn (screens::draw).
    Draw,
    /// A card made round, and zoomed out (zoom::round, zoom::shrink).
    Round,
    Shrink,
    /// The cards put side by side, and the name and dots over them.
    Compose,
    Overlay,
    /// A frame copied to the display, the display turned to it, and the
    /// display read back.
    Copy,
    Pan,
    Read,
}

pub const PARTS: usize = 8;
const NAMES: [&str; PARTS] = ["draw", "round", "shrink", "compose", "overlay", "copy", "pan", "read"];

static SPENT: [AtomicU32; PARTS] = [const { AtomicU32::new(0) }; PARTS];

/// Runs `work`, adding how long it took (in microseconds) to `part`.
pub fn time<T>(part: Part, work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = work();
    SPENT[part as usize].fetch_add(started.elapsed().as_micros() as u32, Ordering::Relaxed);
    result
}

/// What each part took since the last call, in microseconds.
pub fn take() -> [u32; PARTS] {
    std::array::from_fn(|i| SPENT[i].swap(0, Ordering::Relaxed))
}

/// The parts that took a tenth of a millisecond or more, as "draw 41.2, copy 3.1".
pub fn describe(spent: &[u32; PARTS], frames: u32) -> String {
    let frames = frames.max(1);
    NAMES.iter().zip(spent).filter(|(_, &us)| us / frames >= 100)
        .map(|(name, &us)| format!("{name} {:.1}", us as f32 / frames as f32 / 1000.0))
        .collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_that_took_no_time_are_left_out() {
        let mut spent = [0; PARTS];
        spent[Part::Draw as usize] = 82_400;
        spent[Part::Pan as usize] = 90;
        assert_eq!(describe(&spent, 2), "draw 41.2");
    }
}
