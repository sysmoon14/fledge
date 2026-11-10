//! How screens hand over to each other: one set of durations and one curve
//! for everything that moves (D4), and each transition's frame at a point
//! of the way, worked out from the screen it leaves and the one it goes to.
//!
//! All in the framebuffer's layout (device::Frame): an upright column is a
//! framebuffer row (upright x = row), and upright y runs backwards along a
//! row (column = 479 - y). So sideways moves are whole rows, and up and
//! down moves are offsets within each row.

use std::time::Duration;

const SIDE: usize = 480;
const ROW: usize = SIDE * 4;
const CENTRE: f32 = SIDE as f32 / 2.0;

/// Things leaving, press feedback, small labels.
pub const FAST: Duration = Duration::from_millis(180);
/// Screens in and out, menus, a list's rows sliding.
pub const STANDARD: Duration = Duration::from_millis(300);
/// The glance giving way, sweeps to Off, settling.
pub const SLOW: Duration = Duration::from_millis(450);

/// The one curve: a gentle start and a soft landing (CSS's
/// cubic-bezier(.4, 0, .2, 1)), `t` and the result 0 to 1.
pub fn curve(t: f32) -> f32 {
    bezier(0.4, 0.0, 0.2, 1.0, t)
}

/// A cubic Bézier timing curve through (0,0), (x1,y1), (x2,y2), (1,1): y at x = `t`.
fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 || t >= 1.0 {
        return t.clamp(0.0, 1.0);
    }
    let at = |a: f32, b: f32, s: f32| 3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s;
    // x is monotonic in s: a few halvings find it closely enough for a frame.
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..20 {
        let mid = (low + high) / 2.0;
        if at(x1, x2, mid) < t { low = mid } else { high = mid }
    }
    at(y1, y2, (low + high) / 2.0)
}

/// How one screen gives way to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// No movement: the new screen at once.
    Cut,
    /// One screen fading into the other.
    Fade,
    /// The glance giving way: the new screen fading up beneath it, growing
    /// from 96%, as it fades away (A1). Also a camera opening (A4).
    Through,
    /// Deeper into a card: the old screen slides left and fades, the new one
    /// comes in from the right (A2).
    Push,
    /// Back: the reverse of Push.
    Pop,
    /// The ring of icons arriving over its screen: turning in a few degrees
    /// and growing as what's under it dims (A3).
    MenuIn,
    /// The ring of icons leaving: turning on a little and shrinking as its
    /// screen comes back.
    MenuOut,
    /// A notification dropping in from the top over what shows (A4).
    Drop,
    /// A notification lifting away upwards.
    Lift,
}

impl Kind {
    pub fn length(self) -> Duration {
        match self {
            Kind::Cut => Duration::ZERO,
            Kind::Fade | Kind::Push | Kind::Pop | Kind::MenuIn | Kind::Drop => STANDARD,
            // Things leaving go quickly.
            Kind::MenuOut | Kind::Lift => FAST,
            Kind::Through => SLOW,
        }
    }
}

/// How far a pushed screen moves: 14% of the display.
const PUSH: f32 = SIDE as f32 * 0.14;
/// How far a notification drops from.
const DROP: f32 = 96.0;

/// The frame `t` (0 to 1, in time) of the way from `from` to `to`, into `out`.
pub fn frame(kind: Kind, from: &[u8], to: &[u8], t: f32, out: &mut [u8]) {
    let t = t.clamp(0.0, 1.0);
    let e = curve(t);
    match kind {
        Kind::Cut => out.copy_from_slice(to),
        Kind::Fade => blend(from, to, e, out),
        Kind::Through => {
            // The glance gone in the first half, faster at its end; the new
            // screen in from a little after the start, settling as it grows.
            let away = 1.0 - bezier(0.4, 0.0, 1.0, 1.0, (t / 0.55).min(1.0));
            let come = bezier(0.0, 0.0, 0.2, 1.0, ((t - 0.08) / 0.92).clamp(0.0, 1.0));
            transform(to, 0.96 + 0.04 * come, 0.0, out);
            // out = new * come, the glance over it at `away`.
            fade_over(out, from, come, away);
        }
        Kind::Push | Kind::Pop => {
            let sign = if kind == Kind::Push { 1.0 } else { -1.0 };
            // The old screen leaves towards -sign, the new one comes from +sign.
            let old = -sign * PUSH * e;
            let new = sign * PUSH * (1.0 - e);
            for r in 0..SIDE {
                let row = &mut out[r * ROW..(r + 1) * ROW];
                let a = row_at(from, r as f32 - old);
                let b = row_at(to, r as f32 - new);
                mix_rows(a, b, e, row);
            }
        }
        Kind::MenuIn => {
            transform(to, 0.9 + 0.1 * e, (-10.0f32).to_radians() * (1.0 - e), out);
            let menu = out.to_vec();
            blend(from, &menu, e, out);
        }
        Kind::MenuOut => {
            transform(from, 1.0 - 0.06 * e, 6.0f32.to_radians() * e, out);
            let menu = out.to_vec();
            blend(&menu, to, e, out);
        }
        Kind::Drop => {
            shift_y(to, -DROP * (1.0 - e), out);
            let alert = out.to_vec();
            blend(from, &alert, e, out);
        }
        Kind::Lift => {
            shift_y(from, -DROP * e, out);
            let alert = out.to_vec();
            blend(&alert, to, e, out);
        }
    }
}

/// Framebuffer row `r` (an upright column) of `frame`, or black off its edge.
fn row_at(frame: &[u8], r: f32) -> Option<&[u8]> {
    let r = r.round();
    (0.0..SIDE as f32).contains(&r).then(|| &frame[r as usize * ROW..(r as usize + 1) * ROW])
}

/// `a` (or black) and `b` (or black) mixed, `b` at `e`.
fn mix_rows(a: Option<&[u8]>, b: Option<&[u8]>, e: f32, out: &mut [u8]) {
    let w = (e * 256.0).round() as u32;
    match (a, b) {
        (Some(a), Some(b)) => mix(a, b, w, out),
        (Some(a), None) => scale(a, 256 - w, out),
        (None, Some(b)) => scale(b, w, out),
        (None, None) => out.fill(0),
    }
}

/// `to` over `from` at `e`, the whole frame.
pub fn blend(from: &[u8], to: &[u8], e: f32, out: &mut [u8]) {
    mix(from, to, (e.clamp(0.0, 1.0) * 256.0).round() as u32, out);
}

/// Pixel by pixel, `b` at `w` of 256 over `a`: two channels to a multiply.
fn mix(a: &[u8], b: &[u8], w: u32, out: &mut [u8]) {
    let w = w.min(256);
    for ((o, a), b) in out.as_chunks_mut::<4>().0.iter_mut().zip(a.as_chunks::<4>().0).zip(b.as_chunks::<4>().0) {
        let (a, b) = (u32::from_le_bytes(*a), u32::from_le_bytes(*b));
        let even = ((a & 0x00ff_00ff) * (256 - w) + (b & 0x00ff_00ff) * w) >> 8 & 0x00ff_00ff;
        let odd = (((a >> 8) & 0x00ff_00ff) * (256 - w) + ((b >> 8) & 0x00ff_00ff) * w) & 0xff00_ff00;
        *o = (even | odd).to_le_bytes();
    }
}

/// `a` darkened to `w` of 256.
fn scale(a: &[u8], w: u32, out: &mut [u8]) {
    let w = w.min(256);
    for (o, a) in out.as_chunks_mut::<4>().0.iter_mut().zip(a.as_chunks::<4>().0) {
        let a = u32::from_le_bytes(*a);
        let even = ((a & 0x00ff_00ff) * w) >> 8 & 0x00ff_00ff;
        let odd = (((a >> 8) & 0x00ff_00ff) * w) & 0xff00_ff00;
        *o = (even | odd).to_le_bytes();
    }
}

/// In place: `out` (the new screen) faded in to `come`, with `over` on top
/// at `away`: over * away + out * come * (1 - away).
fn fade_over(out: &mut [u8], over: &[u8], come: f32, away: f32) {
    let under = ((come * (1.0 - away)) * 256.0).round() as u32;
    let top = (away * 256.0).round() as u32;
    for (o, a) in out.as_chunks_mut::<4>().0.iter_mut().zip(over.as_chunks::<4>().0) {
        let (b, a) = (u32::from_le_bytes(*o), u32::from_le_bytes(*a));
        let even = ((a & 0x00ff_00ff) * top + (b & 0x00ff_00ff) * under) >> 8 & 0x00ff_00ff;
        let odd = (((a >> 8) & 0x00ff_00ff) * top + ((b >> 8) & 0x00ff_00ff) * under) & 0xff00_ff00;
        *o = (even | odd).to_le_bytes();
    }
}

/// `frame` grown or shrunk to `zoom` and turned `angle` (radians,
/// clockwise as it stands) about the middle, nearest pixel, black where
/// nothing falls: into `out`.
pub fn transform(frame: &[u8], zoom: f32, angle: f32, out: &mut [u8]) {
    if (zoom - 1.0).abs() < 0.001 && angle.abs() < 0.0005 {
        out.copy_from_slice(frame);
        return;
    }
    // Rotations look the same turned a quarter, so the framebuffer's layout
    // turns as the upright picture does (its direction aside, which a
    // quarter turn keeps).
    let (sin, cos) = angle.sin_cos();
    let (a, b) = (cos / zoom, sin / zoom);
    let source = frame.as_chunks::<4>().0;
    let pixels = out.as_chunks_mut::<4>().0;
    for r in 0..SIDE {
        let y = r as f32 + 0.5 - CENTRE;
        // Source for column 0, then a step per column.
        let x0 = 0.5 - CENTRE;
        let (mut u, mut v) = (a * x0 - b * y + CENTRE, b * x0 + a * y + CENTRE);
        for pixel in &mut pixels[r * SIDE..(r + 1) * SIDE] {
            *pixel = if (0.0..SIDE as f32).contains(&u) && (0.0..SIDE as f32).contains(&v) {
                source[v as usize * SIDE + u as usize]
            } else {
                [0, 0, 0, 0xff]
            };
            u += a;
            v += b;
        }
    }
}

/// `frame` moved `dy` pixels down as it stands (negative: up), black where
/// it leaves: into `out`.
pub fn shift_y(frame: &[u8], dy: f32, out: &mut [u8]) {
    // Upright y runs backwards along a framebuffer row: down is towards its start.
    let by = dy.round() as isize;
    let n = by.unsigned_abs().min(SIDE);
    for r in 0..SIDE {
        let (from, to) = (&frame[r * ROW..(r + 1) * ROW], &mut out[r * ROW..(r + 1) * ROW]);
        if by >= 0 {
            to[..(SIDE - n) * 4].copy_from_slice(&from[n * 4..]);
            to[(SIDE - n) * 4..].fill(0);
        } else {
            to[n * 4..].copy_from_slice(&from[..(SIDE - n) * 4]);
            to[..n * 4].fill(0);
        }
    }
}

/// Where a press lights the screen while the button is down (D2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pressed {
    /// The card in the middle of the chooser, of this radius.
    Card(f32),
    /// A list's highlighted row: its pill (x, y, w, h) as it stands.
    Row(f32, f32, f32, f32),
    /// The whole screen, a little.
    Screen,
}

/// `frame` with `pressed` lit a little towards white.
pub fn press(frame: &mut [u8], pressed: Pressed) {
    let lift = |pixel: &mut [u8; 4], amount: u32| {
        for channel in &mut pixel[..3] {
            *channel += (((255 - *channel as u32) * amount) >> 8) as u8;
        }
    };
    let pixels = frame.as_chunks_mut::<4>().0;
    match pressed {
        Pressed::Screen => pixels.iter_mut().for_each(|p| lift(p, 14)),
        Pressed::Card(radius) => {
            for r in 0..SIDE {
                let dx = r as f32 + 0.5 - CENTRE;
                let half = (radius * radius - dx * dx).max(0.0).sqrt();
                let (a, b) = ((CENTRE - half).max(0.0) as usize, ((CENTRE + half) as usize).min(SIDE));
                pixels[r * SIDE + a..r * SIDE + b].iter_mut().for_each(|p| lift(p, 30));
            }
        }
        Pressed::Row(x, y, w, h) => {
            // Upright x is a framebuffer row; upright y runs back along it.
            let (r0, r1) = (x.max(0.0) as usize, ((x + w) as usize).min(SIDE));
            let (c0, c1) = ((SIDE as f32 - (y + h)).max(0.0) as usize, ((SIDE as f32 - y) as usize).min(SIDE));
            for r in r0..r1 {
                pixels[r * SIDE + c0..r * SIDE + c1].iter_mut().for_each(|p| lift(p, 40));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(shade: u8) -> Vec<u8> {
        [shade, shade, shade, 0xff].repeat(SIDE * SIDE)
    }

    #[test]
    fn the_curve_starts_gently_and_lands_softly() {
        assert_eq!(curve(0.0), 0.0);
        assert!((curve(1.0) - 1.0).abs() < 1e-3);
        assert!(curve(0.1) < 0.1, "a gentle start: {}", curve(0.1));
        assert!(curve(0.5) > 0.6, "most of the way by half time: {}", curve(0.5));
        assert!((0..100).all(|i| curve(i as f32 / 100.0) <= curve((i + 1) as f32 / 100.0)), "only forwards");
    }

    #[test]
    fn every_transition_starts_on_the_old_screen_and_ends_on_the_new() {
        let (from, to) = (flat(40), flat(200));
        let mut out = vec![0; SIDE * ROW];
        for kind in [Kind::Fade, Kind::Through, Kind::Push, Kind::Pop, Kind::MenuIn, Kind::MenuOut, Kind::Drop, Kind::Lift] {
            frame(kind, &from, &to, 0.0, &mut out);
            assert_eq!(out[240 * ROW + 240 * 4], 40, "{kind:?} at the start");
            frame(kind, &from, &to, 1.0, &mut out);
            assert_eq!(out[240 * ROW + 240 * 4], 200, "{kind:?} at the end");
        }
    }

    #[test]
    fn push_moves_whole_rows_the_way_it_goes() {
        // A bright column (framebuffer row) in the middle of the new screen
        // starts to its right (a higher row) and comes left as it arrives.
        let from = flat(0);
        let mut to = flat(0);
        to[240 * ROW..241 * ROW].fill(255);
        let mut out = vec![0; SIDE * ROW];
        frame(Kind::Push, &from, &to, 0.3, &mut out);
        let lit: Vec<usize> = (0..SIDE).filter(|&r| out[r * ROW + 100 * 4] > 0).collect();
        assert!(lit.iter().all(|&r| r > 240), "still to the right: {lit:?}");
        frame(Kind::Pop, &from, &to, 0.3, &mut out);
        let lit: Vec<usize> = (0..SIDE).filter(|&r| out[r * ROW + 100 * 4] > 0).collect();
        assert!(lit.iter().all(|&r| r < 240), "back comes from the left: {lit:?}");
    }

    #[test]
    fn a_drop_comes_from_above() {
        // A bright row near the top as it stands (upright y 50: column 429).
        let mut alert = flat(0);
        for r in 0..SIDE {
            alert[r * ROW + 429 * 4..r * ROW + 430 * 4].fill(255);
        }
        let mut out = vec![0; SIDE * ROW];
        shift_y(&alert, -30.0, &mut out);
        assert_eq!(out[100 * ROW + 459 * 4], 255, "30 up is upright y 20");
        shift_y(&alert, 30.0, &mut out);
        assert_eq!(out[100 * ROW + 399 * 4], 255, "30 down is upright y 80");
    }

    #[test]
    fn transforming_by_nothing_changes_nothing() {
        let mut from = flat(10);
        from[1234] = 99;
        let mut out = vec![0; SIDE * ROW];
        transform(&from, 1.0, 0.0, &mut out);
        assert_eq!(out, from);
        transform(&flat(77), 0.5, 0.3, &mut out);
        assert_eq!(out[240 * ROW + 240 * 4], 77, "the middle stays");
        assert_eq!(out[2 * ROW + 2 * 4], 0, "the corner falls outside the shrunk screen");
    }
}
