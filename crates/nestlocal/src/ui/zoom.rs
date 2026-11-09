//! The cards zoomed out while the ring moves between them, as the Nest's
//! own menus: each card smaller, its neighbours showing at the edges, so
//! it reads as moving between cards rather than working one.
//!
//! All in the framebuffer's layout (device::Frame): an upright column is a
//! framebuffer row, so the cards' sideways movement moves whole rows.

use super::device::Frame;

const SIDE: usize = 480;
const ROW: usize = SIDE * 4;
const CENTRE: f32 = SIDE as f32 / 2.0;

/// What's round and between the cards: black, or Fledge's face, as the
/// framebuffer holds it (BGR).
fn backdrop() -> [u8; 3] {
    if super::paint::fledge() {
        let [r, g, b] = super::paint::FACE;
        [b, g, r]
    } else {
        [0, 0, 0]
    }
}

/// A frame of nothing but the backdrop.
fn backdrop_frame() -> Frame {
    let [b, g, r] = backdrop();
    [b, g, r, 0xff].repeat(SIDE * SIDE)
}

/// How small the cards are while the ring moves between them: room round
/// them for the name and dots, and the next ones showing at the sides.
pub fn zoomed() -> f32 {
    0.62
}

/// From one card's centre to the next at zoom `z`: side by side at full
/// size, and a little apart when zoomed out, the next one's edge showing.
pub fn spacing(z: f32) -> f32 {
    // About 24 px between the cards zoomed out, and some 67 px of the next
    // ones showing at the sides.
    SIDE as f32 - (1.0 - z) * 416.0
}

/// Black outside the display's circle, the edge softened over a pixel: a
/// card as the round display shows it, so a coloured one (a thermostat
/// heating) shows no square corners when zoomed out beside the others.
pub fn round(frame: &mut [u8]) {
    let back = backdrop();
    let pixels = frame.as_chunks_mut::<4>().0;
    for r in 0..SIDE {
        let dy = r as f32 + 0.5 - CENTRE;
        for c in 0..SIDE {
            let dx = c as f32 + 0.5 - CENTRE;
            let cover = (CENTRE + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            if cover < 1.0 {
                for (channel, b) in pixels[r * SIDE + c][..3].iter_mut().zip(back) {
                    *channel = (*channel as f32 * cover + b as f32 * (1.0 - cover)) as u8;
                }
            }
        }
    }
}

/// `frame` at zoomed(), centred, black around it: drawn once per card, with
/// each pixel the average of the four it falls between, so a card moved
/// whole pixels at a time doesn't shimmer. In whole numbers, with each
/// output row's and column's sources worked out once: a few milliseconds.
pub fn shrink(frame: &[u8]) -> Frame {
    let mut out = backdrop_frame();
    let half = CENTRE * zoomed();
    let (low, high) = ((CENTRE - half).floor() as usize, ((CENTRE + half).ceil() as usize).min(SIDE));
    // For each output line: its two source lines and the second's weight, of 256.
    let sources: Vec<(usize, usize, u32)> = (0..SIDE)
        .map(|i| {
            let source = (i as f32 + 0.5 - CENTRE) / zoomed() + CENTRE - 0.5;
            let first = source.floor().clamp(0.0, (SIDE - 1) as f32) as usize;
            let weight = ((source - source.floor()) * 256.0).round().clamp(0.0, 256.0) as u32;
            (first, (first + 1).min(SIDE - 1), weight)
        })
        .collect();
    let pixels = frame.as_chunks::<4>().0;
    let output = out.as_chunks_mut::<4>().0;
    for r in low..high {
        let (r0, r1, wr) = sources[r];
        let (top, bottom) = (&pixels[r0 * SIDE..(r0 + 1) * SIDE], &pixels[r1 * SIDE..(r1 + 1) * SIDE]);
        for c in low..high {
            let (c0, c1, wc) = sources[c];
            let mut pixel = [0u8, 0, 0, 0xff];
            for k in 0..3 {
                let upper = top[c0][k] as u32 * (256 - wc) + top[c1][k] as u32 * wc;
                let lower = bottom[c0][k] as u32 * (256 - wc) + bottom[c1][k] as u32 * wc;
                pixel[k] = ((upper * (256 - wr) + lower * wr + (1 << 15)) >> 16) as u8;
            }
            output[r * SIDE + c] = pixel;
        }
    }
    outline(&mut out, half);
    out
}

/// A card's thin edge, zoomed out: a card without a ring of its own
/// (Settings, a scene) still reads as a card among the others.
const EDGE: [u8; 3] = [70, 74, 82];

/// Draws the thin edge of a card of radius `radius` around the middle of
/// `frame`, over what is there, softened over a pixel each side.
fn outline(frame: &mut [u8], radius: f32) {
    const WIDTH: f32 = 1.4;
    let pixels = frame.as_chunks_mut::<4>().0;
    let (low, high) = ((CENTRE - radius - 2.0).max(0.0) as usize, ((CENTRE + radius + 2.0) as usize).min(SIDE));
    for r in low..high {
        let dy = r as f32 + 0.5 - CENTRE;
        for c in low..high {
            let dx = c as f32 + 0.5 - CENTRE;
            let from_edge = ((dx * dx + dy * dy).sqrt() - (radius - WIDTH / 2.0 - 0.5)).abs();
            let cover = (WIDTH / 2.0 + 0.5 - from_edge).clamp(0.0, 1.0);
            if cover <= 0.0 {
                continue;
            }
            let pixel = &mut pixels[r * SIDE + c];
            // BGRX, as the framebuffer holds it.
            for (k, value) in [EDGE[2], EDGE[1], EDGE[0]].into_iter().enumerate() {
                let old = pixel[k] as f32;
                pixel[k] = (old + (value as f32 - old) * cover).round() as u8;
            }
        }
    }
}

/// The cards around place `at` in the deck, `card(k)` giving card k (any
/// integer: the caller wraps it) drawn at zoom `scale` (1, or zoomed() from
/// shrink), each shown at zoom `z`, into `out`.
///
/// At zoomed() from shrunk frames: whole rows, a card's centre on a whole
/// pixel. Otherwise (zooming in or out, briefly) scaled, nearest pixel:
/// from shrunk frames enlarged, which loses no thin strokes, until nearly
/// full size.
pub fn compose<'a>(out: &mut [u8], at: f32, z: f32, scale: f32, card: impl Fn(i64) -> Option<&'a [u8]>) {
    let shrunk = scale == z;
    let factor = scale / z;
    let back = backdrop();
    if back == [0, 0, 0] {
        out.fill(0);
    } else {
        for pixel in out.as_chunks_mut::<4>().0 {
            *pixel = [back[0], back[1], back[2], 0xff];
        }
    }
    let gap = spacing(z);
    let half = CENTRE * z;
    let first = (at - CENTRE / gap).floor() as i64 - 1;
    let last = (at + CENTRE / gap).ceil() as i64 + 1;
    // Columns are the same for every card: the cards only move sideways.
    let columns: Vec<usize> = (0..SIDE)
        .map(|c| (((c as f32 + 0.5 - CENTRE) * factor + CENTRE) as isize).clamp(0, SIDE as isize - 1) as usize)
        .collect();
    let (low, high) = ((CENTRE - half).max(0.0) as usize, ((CENTRE + half).ceil() as usize).min(SIDE));
    for k in first..=last {
        let Some(frame) = card(k) else { continue };
        let centre = CENTRE + (k as f32 - at) * gap;
        let (top, bottom) = ((centre - half).floor().max(0.0) as i64, ((centre + half).ceil() as i64).min(SIDE as i64));
        for r in top.max(0)..bottom {
            let r = r as usize;
            let row = &mut out[r * ROW..(r + 1) * ROW];
            if shrunk {
                let source = r as i64 - centre.round() as i64 + CENTRE as i64;
                if (0..SIDE as i64).contains(&source) {
                    let source = source as usize;
                    row.copy_from_slice(&frame[source * ROW..(source + 1) * ROW]);
                }
            } else {
                let source = (r as f32 + 0.5 - centre) * factor + CENTRE;
                if !(0.0..SIDE as f32).contains(&source) {
                    continue;
                }
                let line = frame[source as usize * ROW..(source as usize + 1) * ROW].as_chunks::<4>().0;
                let pixels = row.as_chunks_mut::<4>().0;
                for (pixel, &from) in pixels[low..high].iter_mut().zip(&columns[low..high]) {
                    *pixel = line[from];
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(shade: u8) -> Frame {
        let mut frame = vec![0u8; SIDE * ROW];
        for (i, b) in frame.iter_mut().enumerate() {
            *b = if i % 4 == 3 { 0xff } else { shade };
        }
        frame
    }

    #[test]
    fn a_card_is_round() {
        let mut a = card(200);
        round(&mut a);
        assert_eq!(a[240 * ROW + 240 * 4], 200, "the middle as it was");
        assert_eq!(a[240 * ROW + 2 * 4], 200, "the edge at the middle row");
        assert_eq!(a[5 * ROW + 5 * 4], 0, "a corner black");
        let edge = a[240 * ROW];
        assert!(edge < 200, "the very edge softened: {edge}");
    }

    #[test]
    fn at_full_size_a_card_fills_the_display() {
        let a = card(100);
        let mut out = vec![0u8; SIDE * ROW];
        compose(&mut out, 0.0, 1.0, 1.0, |k| (k == 0).then_some(a.as_slice()));
        assert!(out == a);
    }

    #[test]
    fn zoomed_out_the_neighbours_show_at_the_edges() {
        let (a, b, c) = (shrink(&card(100)), shrink(&card(150)), shrink(&card(200)));
        let mut out = vec![0u8; SIDE * ROW];
        compose(&mut out, 1.0, zoomed(), zoomed(), |k| match k {
            0 => Some(a.as_slice()),
            1 => Some(b.as_slice()),
            2 => Some(c.as_slice()),
            _ => None,
        });
        let pixel = |r: usize| out[r * ROW + 240 * 4];
        assert_eq!(pixel(240), 150, "the card itself in the middle");
        assert_eq!(pixel(470), 200, "the next one's edge after it");
        assert_eq!(pixel(10), 100, "the one before at the start");
        assert_eq!(pixel(80), 0, "space between them");
        assert_eq!(out[240 * ROW + 20 * 4], 0, "the card smaller than the display");
    }

    #[test]
    fn zooming_from_a_shrunk_card_enlarges_it_in_place() {
        let a = shrink(&card(100));
        let mut out = vec![0u8; SIDE * ROW];
        compose(&mut out, 0.0, 0.9, zoomed(), |k| (k == 0).then_some(a.as_slice()));
        let pixel = |r: usize, c: usize| out[r * ROW + c * 4];
        assert_eq!(pixel(240, 240), 100);
        assert_eq!(pixel(240, 240 - 210), 100, "0.9 of the way out from the centre");
        assert_eq!(pixel(240, 240 - 222), 0, "and no further");
        assert_eq!(pixel(240 - 210, 240), 100);
    }
}
