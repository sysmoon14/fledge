//! Drawing in the Nest's own look: its font (Akkurat, from the unit), its
//! ring of ticks, and Home Assistant's icons.

use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

use super::icons::Icon;

/// The round display's side, in pixels.
pub const SIZE: u32 = 480;
const CENTRE: f32 = SIZE as f32 / 2.0;
/// The ring's outer edge, and how long its ticks are: as nlclient's dial.
/// Measured from nlclient's own dial: ticks from 179 to 232 out, the
/// target's mark from 160.
pub const RING: f32 = 233.0;
pub const TICK: f32 = 54.0;
pub const MARK: f32 = 73.0;
/// The ring's ticks span 292°, open at the bottom, 2° apart: as measured
/// from nlclient's dial.
pub const TICKS: usize = 147;
const SPAN: f32 = 292.0;

/// The unit's fonts.
pub const FONT_DIR: &str = "/nestlabs/share/fonts";

/// The screens' fonts: the Nest's own (Akkurat, from the unit), and the
/// Fledge theme's, built in (DM Sans for words, Outfit for numbers; both
/// under the SIL Open Font License, in fonts/).
pub struct Fonts {
    nest_regular: FontArc,
    nest_medium: FontArc,
    fledge_regular: FontArc,
    fledge_medium: FontArc,
    fledge_digits: FontArc,
    fledge_clock: FontArc,
}

impl Fonts {
    pub fn load(dir: &str) -> Result<Fonts, String> {
        let load = |name: &str| -> Result<FontArc, String> {
            let path = format!("{dir}/{name}");
            let data = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
            FontArc::try_from_vec(data).map_err(|e| format!("{path}: {e}"))
        };
        let built_in = |data: &'static [u8]| FontArc::try_from_slice(data).map_err(|e| format!("a built-in font: {e}"));
        Ok(Fonts {
            nest_regular: load("AkkuratNest-Regular.ttf")?,
            nest_medium: load("AkkuratNest-Medium.ttf")?,
            fledge_regular: built_in(include_bytes!("fonts/DMSans-Regular.ttf"))?,
            fledge_medium: built_in(include_bytes!("fonts/DMSans-Medium.ttf"))?,
            fledge_digits: built_in(include_bytes!("fonts/Outfit-ExtraLight.ttf"))?,
            fledge_clock: built_in(include_bytes!("fonts/Outfit-Light.ttf"))?,
        })
    }

    /// Words, regular and medium, in the theme's font.
    pub fn regular(&self) -> &FontArc {
        if fledge() { &self.fledge_regular } else { &self.nest_regular }
    }

    pub fn medium(&self) -> &FontArc {
        if fledge() { &self.fledge_medium } else { &self.nest_medium }
    }

    /// The glance's clock: read across a room, so Outfit's light (not its
    /// extra light) on Fledge.
    pub fn clock(&self) -> &FontArc {
        if fledge() { &self.fledge_clock } else { &self.nest_regular }
    }

    /// A dial's large number: the Nest's regular, or Outfit's extra light.
    pub fn digits(&self) -> &FontArc {
        if fledge() { &self.fledge_digits } else { &self.nest_regular }
    }
}

/// How the screens look: the Nest's own, or Fledge's (a continuous arc, a
/// dark face that glows with what's on, light numbers).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Theme {
    Nest,
    Fledge,
}

static THEME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_theme(theme: Theme) {
    THEME.store(theme == Theme::Fledge, std::sync::atomic::Ordering::Relaxed);
}

pub fn fledge() -> bool {
    THEME.load(std::sync::atomic::Ordering::Relaxed)
}

/// Fledge's face, its ring's track, and its list highlight.
pub const FACE: [u8; 3] = [9, 13, 21];
pub const TRACK: [u8; 3] = [27, 35, 48];
/// The lift in the middle of Fledge's face: a deep blue.
const FACE_LIFT: [u8; 3] = [22, 31, 48];
const FLEDGE_SELECT: [u8; 3] = [35, 40, 47];

/// A list's highlight, in the theme's colour.
pub fn select() -> [u8; 3] {
    if fledge() { FLEDGE_SELECT } else { SELECT }
}

/// A screen's face: `background` whole (the Nest's), or Fledge's dark face
/// glowing from within with it (heating's amber strongest).
///
/// Fledge's glow is worked out pixel by pixel, which on the unit's one slow
/// core costs most of what drawing a list or a card does: each face is made
/// once (per background, theme and layout) and copied after.
pub fn face(background: [u8; 3]) -> Canvas {
    if !fledge() {
        return canvas(background);
    }
    type Key = ([u8; 3], bool);
    static FACES: std::sync::Mutex<Vec<(Key, Canvas)>> = std::sync::Mutex::new(Vec::new());
    let key = (background, PANEL.load(std::sync::atomic::Ordering::Relaxed));
    let mut faces = FACES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, face)) = faces.iter().find(|(k, _)| *k == key) {
        return face.clone();
    }
    let mut p = canvas(FACE);
    glow(&mut p, FACE_LIFT, 240.0, 210.0, 330.0, 0.7);
    if background != BLACK {
        let strength = if background == HEAT { 0.62 } else { 0.42 };
        glow(&mut p, background, 240.0, 272.0, 300.0, strength);
    }
    // A few backgrounds in all (black, heating, cooling): the oldest goes
    // once there are more.
    if faces.len() >= 4 {
        faces.remove(0);
    }
    faces.push((key, p.clone()));
    p
}

/// A soft radial glow of `c` round (cx, cy), `radius` out, `strength` (0..1)
/// at its middle.
pub fn glow(p: &mut Canvas, c: [u8; 3], cx: f32, cy: f32, radius: f32, strength: f32) {
    let c = p.rgb(c);
    let panel = p.panel;
    let data = p.pixmap.data_mut();
    for y in 0..SIZE {
        let dy = y as f32 + 0.5 - cy;
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - cx;
            let d = (dx * dx + dy * dy).sqrt() / radius;
            if d >= 1.0 {
                continue;
            }
            let a = strength * (1.0 - d) * (1.0 - d);
            let at = 4 * if panel { x * SIZE + (SIZE - 1 - y) } else { y * SIZE + x } as usize;
            for k in 0..3 {
                let old = data[at + k] as f32;
                data[at + k] = (old + (c[k] as f32 - old) * a).round() as u8;
            }
        }
    }
}

pub fn colour([r, g, b]: [u8; 3]) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

pub fn alpha([r, g, b]: [u8; 3], a: f32) -> Color {
    Color::from_rgba8(r, g, b, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

pub const WHITE: [u8; 3] = [255, 255, 255];
pub const GREY: [u8; 3] = [168, 173, 181];
pub const DIM: [u8; 3] = [92, 96, 104];
pub const BLACK: [u8; 3] = [0, 0, 0];
pub const HEAT: [u8; 3] = [241, 90, 0];
pub const COOL: [u8; 3] = [26, 111, 216];
pub const MEDIA: [u8; 3] = [63, 209, 195];
pub const SELECT: [u8; 3] = [26, 115, 232];

static PANEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Draws for the unit's panel from now on: in the framebuffer's own layout,
/// so a frame goes to the display as it is, with no turning.
pub fn for_panel() {
    PANEL.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// What a screen is drawn on. Its coordinates are always the display's,
/// upright; on the panel it holds the framebuffer's layout, the picture
/// turned a quarter anticlockwise in BGRX, as the panel shows the
/// framebuffer turned a quarter clockwise. Otherwise (previews) it holds
/// the picture upright in RGBA.
#[derive(Clone)]
pub struct Canvas {
    pub pixmap: Pixmap,
    panel: bool,
}

impl Canvas {
    /// A colour as this canvas stores it.
    fn rgb(&self, [r, g, b]: [u8; 3]) -> [u8; 3] {
        if self.panel { [b, g, r] } else { [r, g, b] }
    }

    fn colour(&self, c: Color) -> Color {
        if self.panel { Color::from_rgba(c.blue(), c.green(), c.red(), c.alpha()).unwrap_or(c) } else { c }
    }

    /// Where upright paths land.
    fn transform(&self) -> Transform {
        if self.panel { Transform::from_row(0.0, 1.0, -1.0, 0.0, SIZE as f32, 0.0) } else { Transform::identity() }
    }

    /// The byte of upright pixel (x, y), which must be on the canvas.
    fn at(&self, x: u32, y: u32) -> usize {
        4 * if self.panel { x * SIZE + (SIZE - 1 - y) } else { y * SIZE + x } as usize
    }

    /// Puts back the upright rectangle (x, y, w, h) from `still`.
    fn restore(&mut self, still: &Canvas, x: i32, y: i32, w: usize, h: usize) {
        let (x0, y0) = (x.max(0) as u32, y.max(0) as u32);
        let (x1, y1) = ((x + w as i32).clamp(0, SIZE as i32) as u32, (y + h as i32).clamp(0, SIZE as i32) as u32);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let from = still.pixmap.data();
        let to = self.pixmap.data_mut();
        if self.panel {
            // An upright column is a framebuffer row.
            for column in x0..x1 {
                let (a, b) = (4 * (column * SIZE + (SIZE - y1)) as usize, 4 * (column * SIZE + (SIZE - y0)) as usize);
                to[a..b].copy_from_slice(&from[a..b]);
            }
        } else {
            for row in y0..y1 {
                let (a, b) = (4 * (row * SIZE + x0) as usize, 4 * (row * SIZE + x1) as usize);
                to[a..b].copy_from_slice(&from[a..b]);
            }
        }
    }

    fn fill_path(&mut self, path: &tiny_skia::Path, c: Color) {
        let (c, transform) = (self.colour(c), self.transform());
        self.pixmap.fill_path(path, &paint(c), FillRule::Winding, transform, None);
    }
}

impl Canvas {
    /// A frame as a canvas in the upright layout (previews and tests).
    #[cfg(test)]
    pub fn from_frame(frame: Vec<u8>) -> Canvas {
        Canvas { pixmap: Pixmap::from_vec(frame, tiny_skia::IntSize::from_wh(SIZE, SIZE).expect("480")).expect("480x480"), panel: false }
    }
}

impl Canvas {
    /// The picture upright, as RGB (a preview drawn small elsewhere).
    pub fn rgb_upright(&self) -> Vec<u8> {
        let data = self.pixmap.data();
        let mut out = Vec::with_capacity((SIZE * SIZE * 3) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let at = self.at(x, y);
                let px = [data[at], data[at + 1], data[at + 2]];
                out.extend(if self.panel { [px[2], px[1], px[0]] } else { px });
            }
        }
        out
    }
}

pub fn canvas(background: [u8; 3]) -> Canvas {
    canvas_for(background, PANEL.load(std::sync::atomic::Ordering::Relaxed))
}

fn canvas_for(background: [u8; 3], panel: bool) -> Canvas {
    let mut canvas = Canvas { pixmap: Pixmap::new(SIZE, SIZE).expect("480x480"), panel };
    let background = canvas.rgb(background);
    canvas.pixmap.fill(colour(background));
    canvas
}

fn paint(c: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(c);
    paint.anti_alias = true;
    paint
}

/// The angle of tick `i` of `n`, in degrees clockwise from straight up.
pub fn tick_angle(i: usize, n: usize) -> f32 {
    -SPAN / 2.0 + SPAN * i as f32 / (n.max(2) - 1) as f32
}

/// A tick's shape: the ring's own, long (the dial's target, reaching in),
/// or wide (the temperature now, as the Nest marks it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Short,
    Long,
    Wide,
}

impl RingPixel {
    fn cover(&self, shape: Shape) -> u8 {
        match shape {
            Shape::Short => self.short,
            Shape::Long => self.long,
            Shape::Wide => self.wide,
        }
    }
}

/// The whole ring, each tick coloured by `of(i)`; `mark` ticks drawn long
/// and white, as the dial's target.
pub fn ring(canvas: &mut Canvas, of: impl Fn(usize) -> Color, marks: &[usize]) {
    ring_shaped(canvas, |i| if marks.contains(&i) { (colour(WHITE), Shape::Long) } else { (of(i), Shape::Short) });
}

/// The whole ring, each tick's colour and shape from `of(i)`.
///
/// Drawn from the ring's coverage, worked out once (ring_pixels): every
/// pixel near a tick, which tick, and how much of it each shape covers. A
/// frame then only blends those, where stroking 150 lines took most of a
/// frame on the unit.
pub fn ring_shaped(canvas: &mut Canvas, of: impl Fn(usize) -> (Color, Shape)) {
    if fledge() {
        return arc(canvas, None, of);
    }
    let mut colours = [[0u16; 4]; TICKS];
    let mut shapes = [Shape::Short; TICKS];
    for (i, c) in colours.iter_mut().enumerate() {
        let (tint, shape) = of(i);
        let u = canvas.colour(tint).to_color_u8();
        *c = [u.red() as u16, u.green() as u16, u.blue() as u16, u.alpha() as u16];
        shapes[i] = shape;
    }
    let panel = canvas.panel;
    let data = canvas.pixmap.data_mut();
    for p in ring_pixels() {
        let tick = p.tick as usize;
        let cover = p.cover(shapes[tick]) as u16;
        let [r, g, b, a] = colours[tick];
        let a = cover * a / 255;
        if a == 0 {
            continue;
        }
        let at = 4 * if panel { p.panel } else { p.upright } as usize;
        let blend = |d: u8, s: u16| ((s * a + d as u16 * (255 - a)) / 255) as u8;
        data[at] = blend(data[at], r);
        data[at + 1] = blend(data[at + 1], g);
        data[at + 2] = blend(data[at + 2], b);
    }
}

/// Something drawn over the ring: its box (x, y, width, height, upright)
/// and the ticks under it.
type Drawn = ((i32, i32, usize, usize), std::ops::RangeInclusive<usize>);

/// One of Fledge's circles over its arc: where, how big, its colour (RGBA).
type Extra = (f32, f32, f32, [u8; 4]);

/// A value screen kept between frames, so a frame redraws only what
/// changed: ticks whose colour changed, and the number when it changed,
/// over the screen's still part.
pub struct Live {
    still: Canvas,
    drawn: [[u16; 4]; TICKS],
    shapes: [Shape; TICKS],
    fresh: bool,
    number: String,
    /// The old number's glyphs, (x, y, width, height), upright.
    glyphs: Vec<(i32, i32, usize, usize)>,
    /// The upright columns (the panel's rows) this frame changed.
    pub columns: Option<(u32, u32)>,
    /// What Fledge's arc drew over itself last frame (its knob, dots and
    /// rounded ends): each one's box, and the ticks under it.
    extras: Vec<(Extra, Drawn)>,
}

impl Live {
    pub fn new(still: &Canvas) -> Live {
        Live { still: still.clone(), drawn: [[0; 4]; TICKS], shapes: [Shape::Short; TICKS], fresh: true,
               number: String::new(), glyphs: Vec::new(), columns: None, extras: Vec::new() }
    }

    fn changed(&mut self, from: u32, to: u32) {
        let (a, b) = self.columns.unwrap_or((from, to));
        self.columns = Some((a.min(from), b.max(to)));
    }

    /// Puts the upright rectangle (x, y, w, h) of `canvas` back as the still
    /// part has it, and has `ticks` drawn again over it: for what's drawn
    /// over the ring each frame (the temperature now's number), as it fades.
    pub fn clear(&mut self, canvas: &mut Canvas, (x, y, w, h): (i32, i32, usize, usize), ticks: std::ops::RangeInclusive<usize>) {
        canvas.restore(&self.still, x, y, w, h);
        self.changed(x.max(0) as u32, (x + w as i32).clamp(0, SIZE as i32) as u32);
        for tick in ticks {
            if let Some(drawn) = self.drawn.get_mut(tick) {
                *drawn = [u16::MAX; 4];
            }
        }
    }
}

/// The ring as ring() draws it, on a Live screen: only ticks that changed.
pub fn ring_live(canvas: &mut Canvas, live: &mut Live, of: impl Fn(usize) -> Color, marks: &[usize]) {
    ring_shaped_live(canvas, live, |i| if marks.contains(&i) { (colour(WHITE), Shape::Long) } else { (of(i), Shape::Short) });
}

/// The ring as ring_shaped() draws it, on a Live screen: only ticks that changed.
pub fn ring_shaped_live(canvas: &mut Canvas, live: &mut Live, of: impl Fn(usize) -> (Color, Shape)) {
    if fledge() {
        return arc(canvas, Some(live), of);
    }
    let panel = canvas.panel;
    for tick in 0..TICKS {
        let (tint, shape) = of(tick);
        let u = canvas.colour(tint).to_color_u8();
        let colour = [u.red() as u16, u.green() as u16, u.blue() as u16, u.alpha() as u16];
        if !live.fresh && live.drawn[tick] == colour && live.shapes[tick] == shape {
            continue;
        }
        (live.drawn[tick], live.shapes[tick]) = (colour, shape);
        let [r, g, b, a] = colour;
        let still = live.still.pixmap.data();
        let data = canvas.pixmap.data_mut();
        let (mut low, mut high) = (u32::MAX, 0);
        for p in tick_pixels(tick) {
            let at = 4 * if panel { p.panel } else { p.upright } as usize;
            let cover = p.cover(shape) as u16 * a / 255;
            let blend = |d: u8, s: u16| ((s * cover + d as u16 * (255 - cover)) / 255) as u8;
            data[at] = blend(still[at], r);
            data[at + 1] = blend(still[at + 1], g);
            data[at + 2] = blend(still[at + 2], b);
            let column = p.upright % SIZE;
            (low, high) = (low.min(column), high.max(column));
        }
        if low <= high {
            live.changed(low, high);
        }
    }
}

/// Draws `text` as text_at does on a Live screen, the same text again
/// drawing nothing: the old text's glyphs go back to the still part first.
#[allow(clippy::too_many_arguments)]
pub fn number_live(canvas: &mut Canvas, live: &mut Live, parts: &[(&FontArc, f32, f32, f32, &str, f32)], c: [u8; 3]) {
    let key = parts.iter().map(|(_, size, x, y, text, _)| format!("{size}/{x}/{y}/{text}")).collect::<Vec<_>>().join("|");
    if !live.fresh && key == live.number {
        return;
    }
    for (x, y, w, h) in std::mem::take(&mut live.glyphs) {
        canvas.restore(&live.still, x, y, w, h);
        live.changed(x.max(0) as u32, (x + w as i32).clamp(0, SIZE as i32) as u32);
    }
    for &(font, size, x, y, text, spacing) in parts {
        let mut glyphs = Vec::new();
        text_with(canvas, font, size, x, y, text, spacing, c, 1.0, &mut glyphs);
        for &(gx, _, gw, _) in &glyphs {
            live.changed(gx.max(0) as u32, (gx + gw as i32).clamp(0, SIZE as i32) as u32);
        }
        live.glyphs.extend(glyphs);
    }
    live.number = key;
}

/// Marks a Live screen drawn: from now on only what changes is.
pub fn settle(live: &mut Live) {
    live.fresh = false;
}

struct RingPixel {
    /// The pixel in each layout (Canvas).
    upright: u32,
    panel: u32,
    tick: u8,
    /// How much of it a tick covers, of 255: a short tick, a long one (the
    /// target's mark), and a wide one (the temperature now).
    short: u8,
    long: u8,
    wide: u8,
}

fn ring_pixels() -> &'static [RingPixel] {
    &ring_geometry().0
}

/// Each tick's pixels in ring_pixels.
fn tick_pixels(tick: usize) -> &'static [RingPixel] {
    let (pixels, starts) = ring_geometry();
    &pixels[starts[tick]..starts[tick + 1]]
}

/// Fledge's arc: its middle's radius, the band's half width, and how far
/// out a lit band's glow reaches.
const ARC: f32 = 212.0;
const BAND: f32 = 8.0;
const HALO: f32 = 24.0;
/// Fledge's knob (where the ring is set) and dot (the temperature now).
const KNOB: f32 = 13.0;
const DOT: f32 = 5.5;

/// Fledge's arc, as ring_geometry is the ticks': each pixel of the band
/// and its glow, by the tick (the slice of the arc) it's in; `short` the
/// band's cover, `long` the glow's.
fn arc_geometry() -> &'static (Vec<RingPixel>, Vec<usize>) {
    static GEOMETRY: std::sync::OnceLock<(Vec<RingPixel>, Vec<usize>)> = std::sync::OnceLock::new();
    GEOMETRY.get_or_init(|| {
        let mut pixels = Vec::new();
        let step = SPAN / (TICKS - 1) as f32;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (dx, dy) = (x as f32 + 0.5 - CENTRE, y as f32 + 0.5 - CENTRE);
                let off = ((dx * dx + dy * dy).sqrt() - ARC).abs();
                if off > HALO {
                    continue;
                }
                let angle = dx.atan2(-dy).to_degrees();
                let at = (angle + SPAN / 2.0) / step;
                if !(-0.5..=(TICKS - 1) as f32 + 0.5).contains(&at) {
                    continue; // the ends' round caps are drawn as extras
                }
                let tick = at.round().clamp(0.0, (TICKS - 1) as f32) as u8;
                let band = ((BAND + 0.5 - off).clamp(0.0, 1.0) * 255.0).round() as u8;
                let halo = ((1.0 - off / HALO).powi(2) * 255.0).round() as u8;
                pixels.push(RingPixel { upright: y * SIZE + x, panel: x * SIZE + (SIZE - 1 - y), tick, short: band, long: halo, wide: 0 });
            }
        }
        pixels.sort_by_key(|p| p.tick);
        let mut starts = vec![0; TICKS + 1];
        for p in &pixels {
            starts[p.tick as usize + 1] += 1;
        }
        for i in 0..TICKS {
            starts[i + 1] += starts[i];
        }
        (pixels, starts)
    })
}

/// A point on Fledge's arc at tick `at` (fractional), `radius` out.
fn on_arc(at: f32, radius: f32) -> (f32, f32) {
    let angle = (-SPAN / 2.0 + SPAN * at / (TICKS - 1) as f32).to_radians();
    (CENTRE + radius * angle.sin(), CENTRE - radius * angle.cos())
}

/// Whether a tick's colour is the ring's unlit one: the Nest's dim ticks,
/// or nothing.
fn unlit(c: Color) -> bool {
    let u = c.to_color_u8();
    u.alpha() < 8 || [u.red(), u.green(), u.blue()] == DIM
}

/// Fledge's ring: one arc on a faint track, each tick's slice in its
/// colour (lit slices glowing), rounded at its ends and where it's lit; a
/// long tick (the set value) a white knob, a wide one (the temperature now)
/// a dot. On a Live screen only the slices that changed, and what was
/// drawn over them, are drawn again.
fn arc(canvas: &mut Canvas, mut live: Option<&mut Live>, of: impl Fn(usize) -> (Color, Shape)) {
    let ticks: Vec<(Color, Shape)> = (0..TICKS).map(&of).collect();
    // Each slice's colour: a mark takes its neighbour's, so the arc runs on
    // under its knob or dot.
    let band = |i: usize| -> (Color, bool) {
        let pick = |j: usize| ticks.get(j).filter(|(_, shape)| *shape == Shape::Short).map(|(c, _)| *c);
        let c = match ticks[i] {
            (c, Shape::Short) => c,
            _ => pick(i.wrapping_sub(1)).or_else(|| pick(i + 1)).unwrap_or(colour(TRACK)),
        };
        if unlit(c) { (colour(TRACK), false) } else { (c, true) }
    };
    let slices: Vec<(Color, bool)> = (0..TICKS).map(band).collect();
    // What goes over the arc: its rounded ends, where it's lit, the knob and dot.
    let mut extras: Vec<(f32, f32, f32, Color)> = Vec::new();
    let ends = [(0usize, -0.5f32), (TICKS - 1, TICKS as f32 - 0.5)];
    for (tick, at) in ends {
        let (x, y) = on_arc(at, ARC);
        extras.push((x, y, BAND, slices[tick].0));
    }
    for i in 0..TICKS - 1 {
        if slices[i].1 != slices[i + 1].1 {
            let (lit, at) = if slices[i].1 { (i, i as f32 + 0.5) } else { (i + 1, i as f32 + 0.5) };
            let (x, y) = on_arc(at, ARC);
            extras.push((x, y, BAND, slices[lit].0));
        }
    }
    for (i, (c, shape)) in ticks.iter().enumerate() {
        let (x, y) = on_arc(i as f32, ARC);
        match shape {
            Shape::Long => {
                // The knob glows with what it's set to.
                let glow = if slices[i].1 { slices[i].0 } else { colour(WHITE) };
                let mut soft = glow;
                soft.set_alpha(0.22);
                extras.push((x, y, KNOB + 9.0, soft));
                extras.push((x, y, KNOB, colour(WHITE)));
                if slices[i].1 {
                    extras.push((x, y, 4.5, slices[i].0));
                }
            }
            Shape::Wide => extras.push((x, y, DOT, *c)),
            Shape::Short => {}
        }
    }
    let boxed = |&(x, y, r, _): &(f32, f32, f32, Color)| {
        let pad = r + 2.0;
        let rect = ((x - pad).floor() as i32, (y - pad).floor() as i32, (2.0 * pad).ceil() as usize + 1, (2.0 * pad).ceil() as usize + 1);
        let angle = (x - CENTRE).atan2(CENTRE - y).to_degrees();
        let at = (angle + SPAN / 2.0) / (SPAN / (TICKS - 1) as f32);
        let reach = (pad / ARC).to_degrees() / (SPAN / (TICKS - 1) as f32) + 1.0;
        let (low, high) = ((at - reach).floor().max(0.0) as usize, ((at + reach).ceil().max(0.0) as usize).min(TICKS - 1));
        (rect, low..=high)
    };
    let key = |&(x, y, r, c): &(f32, f32, f32, Color)| -> Extra {
        let u = c.to_color_u8();
        (x, y, r, [u.red(), u.green(), u.blue(), u.alpha()])
    };
    let new: Vec<(Extra, Drawn)> = extras.iter().map(|e| (key(e), boxed(e))).collect();
    // Which circles draw again: only those that moved or changed, or whose
    // slices or neighbours did; the rest (the arc's ends, a knob at rest)
    // stay as they are, so a frame shows only what moved.
    let mut redraw = vec![!live.as_deref().is_some_and(|l| !l.fresh); new.len()];
    if let Some(live) = live.as_deref_mut() {
        let mut changed: Vec<bool> = slices.iter().enumerate().map(|(tick, &(tint, lit))| {
            let u = canvas.colour(tint).to_color_u8();
            let colour = [u.red() as u16, u.green() as u16, u.blue() as u16, u.alpha() as u16];
            let shape = if lit { Shape::Wide } else { Shape::Short };
            live.fresh || live.drawn[tick] != colour || live.shapes[tick] != shape
        }).collect();
        let previous = std::mem::take(&mut live.extras);
        let overlaps = |a: &(i32, i32, usize, usize), b: &(i32, i32, usize, usize)| {
            a.0 < b.0 + b.2 as i32 && b.0 < a.0 + a.2 as i32 && a.1 < b.1 + b.3 as i32 && b.1 < a.1 + a.3 as i32
        };
        let mut cleared = vec![false; previous.len()];
        for (j, (extra, _)) in new.iter().enumerate() {
            redraw[j] = !previous.iter().any(|(old, _)| old == extra);
        }
        loop {
            let mut more = false;
            for (i, (old, (rect, under))) in previous.iter().enumerate() {
                if cleared[i] {
                    continue;
                }
                let gone = !new.iter().any(|(extra, _)| extra == old);
                let under_changed = under.clone().any(|t| changed[t]);
                let near = new.iter().zip(&redraw).any(|((_, (r, _)), &again)| again && overlaps(r, rect))
                    || previous.iter().zip(&cleared).any(|((_, (r, _)), &c)| c && overlaps(r, rect));
                if gone || under_changed || near {
                    cleared[i] = true;
                    more = true;
                    for t in under.clone() {
                        changed[t] = true;
                    }
                    for (j, (extra, _)) in new.iter().enumerate() {
                        if extra == old {
                            redraw[j] = true;
                        }
                    }
                }
            }
            if !more {
                break;
            }
        }
        for (i, (_, (rect, under))) in previous.into_iter().enumerate() {
            if cleared[i] {
                live.clear(canvas, rect, under);
            }
        }
    }
    let panel = canvas.panel;
    let (pixels, starts) = arc_geometry();
    for (tick, &(tint, lit)) in slices.iter().enumerate() {
        let u = canvas.colour(tint).to_color_u8();
        let colour = [u.red() as u16, u.green() as u16, u.blue() as u16, u.alpha() as u16];
        let shape = if lit { Shape::Wide } else { Shape::Short };
        if let Some(live) = live.as_deref_mut() {
            if !live.fresh && live.drawn[tick] == colour && live.shapes[tick] == shape {
                continue;
            }
            (live.drawn[tick], live.shapes[tick]) = (colour, shape);
        }
        let [r, g, b, a] = colour;
        let (mut low, mut high) = (u32::MAX, 0);
        for p in &pixels[starts[tick]..starts[tick + 1]] {
            let at = 4 * if panel { p.panel } else { p.upright } as usize;
            let band = p.short as u32 * a as u32 / 255;
            let halo = if lit { p.long as u32 * a as u32 * 140 / (255 * 255) } else { 0 };
            // The band over its glow, both of one colour.
            let cover = (255 - (255 - band) * (255 - halo) / 255) as u16;
            let from = match live.as_deref() {
                Some(live) => [live.still.pixmap.data()[at], live.still.pixmap.data()[at + 1], live.still.pixmap.data()[at + 2]],
                None => {
                    let data = canvas.pixmap.data();
                    [data[at], data[at + 1], data[at + 2]]
                }
            };
            let data = canvas.pixmap.data_mut();
            let blend = |d: u8, s: u16| ((s * cover + d as u16 * (255 - cover)) / 255) as u8;
            data[at] = blend(from[0], r);
            data[at + 1] = blend(from[1], g);
            data[at + 2] = blend(from[2], b);
            let column = p.upright % SIZE;
            (low, high) = (low.min(column), high.max(column));
        }
        if let (Some(live), true) = (live.as_deref_mut(), low <= high) {
            live.changed(low, high);
        }
    }
    for ((extra, (key, drawn)), again) in extras.iter().zip(new).zip(redraw) {
        let (x, y, r, c) = *extra;
        if again {
            circle(canvas, x, y, r, c);
        }
        if let Some(live) = live.as_deref_mut() {
            let (rect, _) = &drawn;
            if again {
                live.changed(rect.0.max(0) as u32, (rect.0 + rect.2 as i32).clamp(0, SIZE as i32) as u32);
            }
            live.extras.push((key, drawn));
        }
    }
}

/// Works out both rings' shapes ahead of their first frame, which would
/// otherwise wait for them (half a second on the unit).
pub fn warm() {
    let _ = (ring_geometry(), arc_geometry());
}

fn ring_geometry() -> &'static (Vec<RingPixel>, Vec<usize>) {
    static GEOMETRY: std::sync::OnceLock<(Vec<RingPixel>, Vec<usize>)> = std::sync::OnceLock::new();
    GEOMETRY.get_or_init(|| {
        let mut pixels = ring_pixels_unsorted();
        pixels.sort_by_key(|p| p.tick);
        let mut starts = vec![0; TICKS + 1];
        for p in &pixels {
            starts[p.tick as usize + 1] += 1;
        }
        for i in 0..TICKS {
            starts[i + 1] += starts[i];
        }
        (pixels, starts)
    })
}

fn ring_pixels_unsorted() -> Vec<RingPixel> {
    {
        let mut pixels = Vec::new();
        let (inner, outer) = (RING - MARK - 1.0, RING + 1.0);
        // Each tick a wedge, as the Nest's: `half` its half width at its
        // inner end and at the rim.
        let cover = |distance: f32, radius: f32, (inner, rim): (f32, f32), length: f32| {
            let along_tick = ((radius - (RING - length)) / length).clamp(0.0, 1.0);
            let half = inner + (rim - inner) * along_tick;
            let across = (half + 0.5 - distance).clamp(0.0, 1.0);
            let along = (radius - (RING - length) + 0.5).clamp(0.0, 1.0) * (RING + 0.5 - radius).clamp(0.0, 1.0);
            (across * along * 255.0).round() as u8
        };
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (dx, dy) = (x as f32 + 0.5 - CENTRE, y as f32 + 0.5 - CENTRE);
                let radius = (dx * dx + dy * dy).sqrt();
                if radius < inner || radius > outer {
                    continue;
                }
                let angle = dx.atan2(-dy).to_degrees();
                let tick = ((angle + SPAN / 2.0) / SPAN * (TICKS - 1) as f32).round();
                if !(0.0..TICKS as f32).contains(&tick) {
                    continue;
                }
                let distance = radius * (angle - tick_angle(tick as usize, TICKS)).to_radians().sin().abs();
                // The ring's ticks slim, as the Nest's; its marks wedges,
                // broadest at the rim.
                let (short, long) = (cover(distance, radius, (1.2, 1.2), TICK), cover(distance, radius, (2.3, 4.3), MARK));
                let wide = cover(distance, radius, (2.8, 3.3), TICK);
                if short > 0 || long > 0 || wide > 0 {
                    pixels.push(RingPixel { upright: y * SIZE + x, panel: x * SIZE + (SIZE - 1 - y),
                                            tick: tick as u8, short, long, wide });
                }
            }
        }
        pixels
    }
}

/// The tick that shows `fraction` (0..1) of the ring.
pub fn tick_at(fraction: f32) -> usize {
    ((fraction.clamp(0.0, 1.0) * (TICKS - 1) as f32).round()) as usize
}

/// A square RGB picture `size` wide, centred on (cx, cy), its corners
/// rounded to `radius`.
pub fn picture(canvas: &mut Canvas, rgb: &[u8], size: u32, cx: f32, cy: f32, radius: f32) {
    let (x0, y0) = ((cx - size as f32 / 2.0).round() as i32, (cy - size as f32 / 2.0).round() as i32);
    let edge = size as f32 - radius;
    for y in 0..size {
        let (py, fy) = (y0 + y as i32, y as f32 + 0.5);
        if !(0..SIZE as i32).contains(&py) {
            continue;
        }
        for x in 0..size {
            let (px, fx) = (x0 + x as i32, x as f32 + 0.5);
            if !(0..SIZE as i32).contains(&px) {
                continue;
            }
            // How much of the pixel is inside the rounded corners.
            let (ex, ey) = ((radius - fx).max(fx - edge).max(0.0), (radius - fy).max(fy - edge).max(0.0));
            let cover = if ex > 0.0 && ey > 0.0 { (radius - (ex * ex + ey * ey).sqrt() + 0.5).clamp(0.0, 1.0) } else { 1.0 };
            if cover <= 0.0 {
                continue;
            }
            let from = 3 * (y * size + x) as usize;
            let colour = canvas.rgb([rgb[from], rgb[from + 1], rgb[from + 2]]);
            let at = canvas.at(px as u32, py as u32);
            let data = canvas.pixmap.data_mut();
            if cover >= 1.0 {
                // Inside: the picture's own pixel, nothing to blend.
                data[at..at + 3].copy_from_slice(&colour);
                continue;
            }
            for (channel, value) in colour.into_iter().enumerate() {
                let old = data[at + channel] as f32;
                data[at + channel] = (old + (value as f32 - old) * cover).round() as u8;
            }
        }
    }
}

/// A square RGB picture `size` wide drawn `side` wide (smaller), each pixel
/// the average of those it covers, centred on (cx, cy), corners rounded.
pub fn picture_scaled(canvas: &mut Canvas, rgb: &[u8], size: u32, side: u32, cx: f32, cy: f32, radius: f32) {
    if side >= size {
        return picture(canvas, rgb, size, cx, cy, radius);
    }
    let mut small = Vec::with_capacity((side * side * 3) as usize);
    for y in 0..side {
        let (y0, y1) = (y * size / side, ((y + 1) * size / side).max(y * size / side + 1));
        for x in 0..side {
            let (x0, x1) = (x * size / side, ((x + 1) * size / side).max(x * size / side + 1));
            let mut sum = [0u32; 3];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let at = 3 * (sy * size + sx) as usize;
                    for k in 0..3 {
                        sum[k] += rgb[at + k] as u32;
                    }
                }
            }
            let n = (y1 - y0) * (x1 - x0);
            small.extend(sum.map(|v| (v / n) as u8));
        }
    }
    picture(canvas, &small, side, cx, cy, radius);
}

/// A filled circle, its edge softened over a pixel: drawn pixel by pixel
/// over its box, a hundred times cheaper than a path on the unit (where
/// the knob and its glow are drawn every frame the ring turns).
pub fn circle(canvas: &mut Canvas, cx: f32, cy: f32, r: f32, c: Color) {
    let u = c.to_color_u8();
    let [cr, cg, cb] = canvas.rgb([u.red(), u.green(), u.blue()]);
    let a = u.alpha() as f32 / 255.0;
    if a <= 0.0 || r <= 0.0 {
        return;
    }
    let (x0, x1) = (((cx - r - 1.0).floor() as i32).max(0), ((cx + r + 1.0).ceil() as i32).min(SIZE as i32));
    let (y0, y1) = (((cy - r - 1.0).floor() as i32).max(0), ((cy + r + 1.0).ceil() as i32).min(SIZE as i32));
    for y in y0..y1 {
        let dy = y as f32 + 0.5 - cy;
        for x in x0..x1 {
            let dx = x as f32 + 0.5 - cx;
            let cover = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0) * a;
            if cover <= 0.0 {
                continue;
            }
            let at = canvas.at(x as u32, y as u32);
            let data = canvas.pixmap.data_mut();
            for (k, v) in [cr, cg, cb].into_iter().enumerate() {
                let old = data[at + k] as f32;
                data[at + k] = (old + (v as f32 - old) * cover).round() as u8;
            }
        }
    }
}

/// A rectangle with its corners rounded to `r`.
pub fn rounded(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32, r: f32, c: Color) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut path = PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(x + w - r, y);
    path.quad_to(x + w, y, x + w, y + r);
    path.line_to(x + w, y + h - r);
    path.quad_to(x + w, y + h, x + w - r, y + h);
    path.line_to(x + r, y + h);
    path.quad_to(x, y + h, x, y + h - r);
    path.line_to(x, y + r);
    path.quad_to(x, y, x + r, y);
    path.close();
    if let Some(path) = path.finish() {
        canvas.fill_path(&path, c);
    }
}

/// A rounded bar, as the Nest's lists select with.
pub fn pill(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32, c: Color) {
    let r = h / 2.0;
    let mut path = PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(x + w - r, y);
    path.quad_to(x + w, y, x + w, y + r);
    path.quad_to(x + w, y + h, x + w - r, y + h);
    path.line_to(x + r, y + h);
    path.quad_to(x, y + h, x, y + r);
    path.quad_to(x, y, x + r, y);
    path.close();
    // Quadratics through the corners' control points round them a little
    // squarer than arcs; at a list row's size the Nest's look the same.
    if let Some(path) = path.finish() {
        canvas.fill_path(&path, c);
    }
}

/// The list's highlight where it glides: within the pill at (x, y, w, h),
/// `chosen` (the rows as the highlight shows them) over `out` (as they show
/// without it), the pill's rounded edge softened over a pixel.
pub fn pill_window(out: &mut Canvas, chosen: &Canvas, x: f32, y: f32, w: f32, h: f32) {
    let r = h / 2.0;
    let (y0, y1) = ((y.floor() as i32).max(0), ((y + h).ceil() as i32).min(SIZE as i32));
    let (x0, x1) = ((x.floor() as i32).max(0), ((x + w).ceil() as i32).min(SIZE as i32));
    let from = chosen.pixmap.data();
    for py in y0..y1 {
        let fy = py as f32 + 0.5;
        let along_y = (fy - y + 0.5).clamp(0.0, 1.0) * (y + h - fy + 0.5).clamp(0.0, 1.0);
        for px in x0..x1 {
            let fx = px as f32 + 0.5;
            // Distance outside the pill: its ends are half circles.
            let cx = fx.clamp(x + r, x + w - r);
            let cover = (r + 0.5 - ((fx - cx).powi(2) + (fy - (y + r)).powi(2)).sqrt()).clamp(0.0, 1.0) * along_y;
            if cover <= 0.0 {
                continue;
            }
            let at = out.at(px as u32, py as u32);
            let data = out.pixmap.data_mut();
            for k in 0..3 {
                let (old, new) = (data[at + k] as f32, from[at + k] as f32);
                data[at + k] = (old + (new - old) * cover).round() as u8;
            }
        }
    }
}

/// `text`'s width at `size`, `spacing` added between letters.
pub fn measure(font: &FontArc, size: f32, text: &str, spacing: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut width = 0.0;
    let mut previous = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(p) = previous {
            width += scaled.kern(p, id) + spacing;
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}

/// A glyph's coverage, rasterised once per font, size and quarter pixel:
/// outlines are slow to fill on the unit, and the same few glyphs repeat.
struct Glyph {
    left: i32,
    top: i32,
    width: usize,
    coverage: Vec<u8>,
}

type GlyphKey = (usize, u16, u32, u8);

thread_local! {
    static GLYPHS: std::cell::RefCell<std::collections::HashMap<GlyphKey, std::rc::Rc<Glyph>>> =
        Default::default();
}

fn glyph(font: &FontArc, id: ab_glyph::GlyphId, size: f32, quarter: u8) -> std::rc::Rc<Glyph> {
    let key = (font.font_data().as_ptr() as usize, id.0, size.to_bits(), quarter);
    if let Some(found) = GLYPHS.with(|g| g.borrow().get(&key).cloned()) {
        return found;
    }
    let scaled = font.as_scaled(PxScale::from(size));
    let positioned = id.with_scale_and_position(PxScale::from(size), point(quarter as f32 / 4.0, 0.0));
    let made = match scaled.outline_glyph(positioned) {
        Some(outline) => {
            let bounds = outline.px_bounds();
            let (width, height) = (bounds.width() as usize, bounds.height() as usize);
            let mut coverage = vec![0u8; width * height];
            outline.draw(|x, y, c| {
                if let Some(cell) = coverage.get_mut(y as usize * width + x as usize) {
                    *cell = (c.clamp(0.0, 1.0) * 255.0) as u8;
                }
            });
            Glyph { left: bounds.min.x as i32, top: bounds.min.y as i32, width, coverage }
        }
        None => Glyph { left: 0, top: 0, width: 0, coverage: Vec::new() },
    };
    let made = std::rc::Rc::new(made);
    GLYPHS.with(|g| g.borrow_mut().insert(key, made.clone()));
    made
}

/// Blends coverage `mask` (`width` wide) in colour `c` with its top left at
/// (x, y), each pixel at its coverage times `opacity`.
fn blend_mask(canvas: &mut Canvas, x: i32, y: i32, width: usize, mask: &[u8], c: [u8; 3], opacity: f32) {
    if width == 0 {
        return;
    }
    let c = canvas.rgb(c).map(|v| v as u16);
    let opacity = (opacity.clamp(0.0, 1.0) * 255.0) as u16;
    let side = SIZE as i32;
    for (row, line) in mask.chunks(width).enumerate() {
        let py = y + row as i32;
        if py < 0 || py >= side {
            continue;
        }
        for (column, &cover) in line.iter().enumerate() {
            let px = x + column as i32;
            if cover == 0 || px < 0 || px >= side {
                continue;
            }
            let a = cover as u16 * opacity / 255;
            let at = canvas.at(px as u32, py as u32);
            let data = canvas.pixmap.data_mut();
            for k in 0..3 {
                data[at + k] = ((c[k] * a + data[at + k] as u16 * (255 - a)) / 255) as u8;
            }
        }
    }
}

/// Draws `text` from `x` with its baseline at `y`; returns where it ends.
#[allow(clippy::too_many_arguments)] // a drawing primitive: where, what, how
pub fn text_at(canvas: &mut Canvas, font: &FontArc, size: f32, x: f32, y: f32, text: &str, spacing: f32,
               c: [u8; 3], opacity: f32) -> f32 {
    text_with(canvas, font, size, x, y, text, spacing, c, opacity, &mut Vec::new())
}

/// text_at, noting each glyph's rectangle (x, y, width, height).
#[allow(clippy::too_many_arguments)]
fn text_with(canvas: &mut Canvas, font: &FontArc, size: f32, x: f32, y: f32, text: &str, spacing: f32,
             c: [u8; 3], opacity: f32, glyphs: &mut Vec<(i32, i32, usize, usize)>) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut x = x;
    let mut previous = None;
    let baseline = y.round() as i32;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(p) = previous {
            x += scaled.kern(p, id) + spacing;
        }
        let whole = x.floor();
        let quarter = ((x - whole) * 4.0) as u8;
        let g = glyph(font, id, size, quarter);
        let (gx, gy) = (whole as i32 + g.left, baseline + g.top);
        blend_mask(canvas, gx, gy, g.width, &g.coverage, c, opacity);
        if g.width > 0 {
            glyphs.push((gx, gy, g.width, g.coverage.len() / g.width));
        }
        x += scaled.h_advance(id);
        previous = Some(id);
    }
    x
}

/// A line of words too long for its place, scrolling through it: `text`
/// (`words` wide) in the window `width` wide from `left`, `offset` pixels
/// along, and after `gap` the same again coming in behind it, so the line
/// goes round. Its ends fade into what's behind (`still`: the canvas as it
/// is without the words). Returns the upright columns it drew in.
pub struct Ticker<'a> {
    pub font: &'a FontArc,
    pub size: f32,
    pub text: &'a str,
    pub words: f32,
    pub left: f32,
    pub width: f32,
    pub baseline: f32,
    pub colour: [u8; 3],
    pub gap: f32,
}

pub fn ticker(canvas: &mut Canvas, still: &Canvas, line: &Ticker, offset: f32) -> (u32, u32) {
    const FADE: i32 = 22;
    let (x0, x1) = (line.left.floor() as i32, (line.left + line.width).ceil() as i32);
    let (top, height) = ((line.baseline - line.size * 1.05).floor() as i32, (line.size * 1.5).ceil() as usize);
    canvas.restore(still, 0, top, SIZE as usize, height);
    // At rest its first letter stands clear of the faded end.
    let origin = line.left + FADE as f32 - offset;
    for start in [origin, origin + line.words + line.gap] {
        if start < x1 as f32 && start + line.words > x0 as f32 {
            text_at(canvas, line.font, line.size, start, line.baseline, line.text, 0.0, line.colour, 1.0);
        }
    }
    // Only in its window: the rest as it was, and the ends fading out.
    canvas.restore(still, 0, top, x0.max(0) as usize, height);
    canvas.restore(still, x1, top, (SIZE as i32 - x1).max(0) as usize, height);
    let (y0, y1) = (top.max(0) as u32, (top + height as i32).clamp(0, SIZE as i32) as u32);
    for step in 0..FADE {
        let keep = (step as f32 + 0.5) / FADE as f32;
        for x in [x0 + step, x1 - 1 - step] {
            if !(0..SIZE as i32).contains(&x) {
                continue;
            }
            for y in y0..y1 {
                let at = canvas.at(x as u32, y);
                let (from, to) = (still.pixmap.data(), canvas.pixmap.data_mut());
                for k in 0..3 {
                    let (old, new) = (from[at + k] as f32, to[at + k] as f32);
                    to[at + k] = (old + (new - old) * keep).round() as u8;
                }
            }
        }
    }
    (x0.clamp(0, SIZE as i32 - 1) as u32, (x1 - 1).clamp(0, SIZE as i32 - 1) as u32)
}

/// Draws `text` centred on `cx`.
#[allow(clippy::too_many_arguments)]
pub fn text(canvas: &mut Canvas, font: &FontArc, size: f32, cx: f32, y: f32, text: &str, spacing: f32, c: [u8; 3]) {
    let width = measure(font, size, text, spacing);
    text_at(canvas, font, size, cx - width / 2.0, y, text, spacing, c, 1.0);
}

/// `text` shortened with an ellipsis to fit `width`.
pub fn fit(font: &FontArc, size: f32, text: &str, spacing: f32, width: f32) -> String {
    if measure(font, size, text, spacing) <= width {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>().trim_end().to_owned() + "…";
        if measure(font, size, &candidate, spacing) <= width {
            return candidate;
        }
    }
    String::new()
}

/// An icon's coverage at one size: its side and a byte per pixel.
type Mask = std::rc::Rc<(usize, Vec<u8>)>;

thread_local! {
    static ICONS: std::cell::RefCell<std::collections::HashMap<(usize, u32), Mask>> = Default::default();
}

/// Draws an icon `size` across, centred on (cx, cy), from its coverage at
/// that size, made once.
pub fn icon(canvas: &mut Canvas, icon: Icon, cx: f32, cy: f32, size: f32, c: Color) {
    let key = (icon.path.as_ptr() as usize, size.to_bits());
    let mask = ICONS.with(|cache| cache.borrow().get(&key).cloned()).unwrap_or_else(|| {
        let made = std::rc::Rc::new(icon_mask(icon, size));
        ICONS.with(|cache| cache.borrow_mut().insert(key, made.clone()));
        made
    });
    let side = mask.0;
    let u = c.to_color_u8();
    blend_mask(canvas, (cx - side as f32 / 2.0).round() as i32, (cy - side as f32 / 2.0).round() as i32, side,
               &mask.1, [u.red(), u.green(), u.blue()], u.alpha() as f32 / 255.0);
}

fn icon_mask(icon: Icon, size: f32) -> (usize, Vec<u8>) {
    let side = size.ceil() as u32;
    let Some(mut pixmap) = Pixmap::new(side.max(1), side.max(1)) else { return (0, Vec::new()) };
    let Ok(svg) = kurbo::BezPath::from_svg(icon.path) else { return (0, Vec::new()) };
    let mut path = PathBuilder::new();
    for element in svg.elements() {
        match *element {
            kurbo::PathEl::MoveTo(p) => path.move_to(p.x as f32, p.y as f32),
            kurbo::PathEl::LineTo(p) => path.line_to(p.x as f32, p.y as f32),
            kurbo::PathEl::QuadTo(a, p) => path.quad_to(a.x as f32, a.y as f32, p.x as f32, p.y as f32),
            kurbo::PathEl::CurveTo(a, b, p) => {
                path.cubic_to(a.x as f32, a.y as f32, b.x as f32, b.y as f32, p.x as f32, p.y as f32)
            }
            kurbo::PathEl::ClosePath => path.close(),
        }
    }
    let Some(path) = path.finish() else { return (0, Vec::new()) };
    let scale = size / icon.box_size;
    // Material Symbols' viewBox starts at y -960.
    let top = if icon.box_size == 960.0 { 960.0 * scale } else { 0.0 };
    let transform = Transform::from_row(scale, 0.0, 0.0, scale, 0.0, top);
    pixmap.fill_path(&path, &paint(colour(WHITE)), FillRule::Winding, transform, None);
    (side as usize, pixmap.data().as_chunks::<4>().0.iter().map(|p| p[3]).collect())
}

/// Dims the whole canvas to `keep` of its brightness: under a menu.
pub fn dim(canvas: &mut Canvas, keep: f32) {
    let keep = (keep.clamp(0.0, 1.0) * 256.0) as u16;
    for pixel in canvas.pixmap.data_mut().as_chunks_mut::<4>().0 {
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u16 * keep) >> 8) as u8;
        }
    }
}

/// A hint on a frame for the panel (device::Frame): a pill low on the
/// display, its text centred.
pub fn hint(frame: &mut Vec<u8>, fonts: &Fonts, text: &str) {
    let Some(pixmap) = Pixmap::from_vec(std::mem::take(frame), tiny_skia::IntSize::from_wh(SIZE, SIZE).expect("480")) else {
        return;
    };
    let mut canvas = Canvas { pixmap, panel: true };
    let width = measure(fonts.medium(), 15.0, text, 0.2) + 28.0;
    // Between the cards (their bottoms at 389) and the dots.
    pill(&mut canvas, CENTRE - width / 2.0, 399.0, width, 30.0, Color::from_rgba8(36, 38, 42, 255));
    self::text(&mut canvas, fonts.medium(), 15.0, CENTRE, 419.0, text, 0.2, [230, 233, 238]);
    *frame = canvas.pixmap.take();
}

/// Where the cards are: a dot each at height `y`, `active` white, all at
/// `fade` of their brightness.
pub fn dots(canvas: &mut Canvas, count: usize, active: usize, y: f32, fade: f32) {
    if count < 2 {
        return;
    }
    let gap = 15.0;
    if fledge() {
        // Fledge's: a lozenge where you are among dots.
        let width = gap * (count - 1) as f32 + 14.0;
        let mut x = CENTRE - width / 2.0;
        for i in 0..count {
            let (w, a) = if i == active { (22.0, 1.0) } else { (8.0, 0.32) };
            pill(canvas, x, y - 4.0, w, 8.0, alpha(WHITE, a * fade.clamp(0.0, 1.0)));
            x += w + 7.0;
        }
        return;
    }
    let start = CENTRE - gap * (count - 1) as f32 / 2.0;
    for i in 0..count {
        let a = if i == active { 1.0 } else { 0.35 };
        circle(canvas, start + gap * i as f32, y, 4.0, alpha(WHITE, a * fade.clamp(0.0, 1.0)));
    }
}

/// The chosen card's name above the zoomed-out cards (their tops at 91),
/// where the display is wide enough for a long one, and the dots below.
const TITLE_BASELINE: f32 = 68.0;
const DOTS_AT: f32 = 446.0;

/// How wide the round display is at height `y`, less a margin.
fn chord(y: f32) -> f32 {
    let from_centre = CENTRE - y;
    2.0 * (CENTRE * CENTRE - from_centre * from_centre).max(0.0).sqrt() - 12.0
}

/// The largest size a card's name fits at above the cards, where the
/// round display is narrow, and the width it has there; a name too long
/// even at the smallest is cut short to that width.
fn title_size(fonts: &Fonts, name: &str) -> (f32, f32) {
    let room = |size: f32| chord(TITLE_BASELINE - size * 0.72);
    for size in [32.0, 28.0, 25.0, 22.0, 19.0] {
        if measure(fonts.medium(), size, name, 0.0) <= room(size) {
            return (size, room(size));
        }
    }
    (19.0, room(19.0))
}

/// Around the cards zoomed out, on a frame for the panel (device::Frame):
/// the chosen one's name above at `title` of full opacity, and the dots
/// under them at `fade`.
pub fn around_cards(frame: &mut Vec<u8>, fonts: &Fonts, name: &str, title: f32, count: usize, active: usize,
                    fade: f32) {
    let Some(pixmap) = Pixmap::from_vec(std::mem::take(frame), tiny_skia::IntSize::from_wh(SIZE, SIZE).expect("480")) else {
        return;
    };
    let mut canvas = Canvas { pixmap, panel: true };
    if title > 0.01 {
        let (size, room) = title_size(fonts, name);
        let name = fit(fonts.medium(), size, name, 0.0, room);
        let width = measure(fonts.medium(), size, &name, 0.0);
        text_at(&mut canvas, fonts.medium(), size, CENTRE - width / 2.0, TITLE_BASELINE, &name, 0.0, WHITE, title.min(1.0));
    }
    dots(&mut canvas, count, active, DOTS_AT, fade);
    *frame = canvas.pixmap.take();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draws the same things on both canvases: the panel's must be the
    /// upright one turned, in BGR.
    #[test]
    fn the_panel_canvas_is_the_upright_one_turned() {
        let draw = |canvas: &mut Canvas| {
            ring(canvas, |i| if i < 60 { colour([255, 169, 87]) } else { colour(DIM) }, &[60]);
            pill(canvas, 62.0, 128.0, 356.0, 54.0, colour(SELECT));
            dots(canvas, 5, 1, 440.0, 1.0);
        };
        let (mut upright, mut panel) = (canvas_for([10, 20, 30], false), canvas_for([10, 20, 30], true));
        draw(&mut upright);
        draw(&mut panel);
        let (u, p) = (upright.pixmap.data(), panel.pixmap.data());
        let mut worst = 0;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let (a, b) = (upright.at(x, y), panel.at(x, y));
                for (k, l) in [(0, 2), (1, 1), (2, 0)] {
                    worst = worst.max((u[a + k] as i32 - p[b + l] as i32).abs());
                }
            }
        }
        // tiny-skia samples edges finer along one axis than the other.
        assert!(worst <= 16, "differs by up to {worst} (antialiasing of turned paths)");
        assert_eq!(&p[panel.at(0, 0)..panel.at(0, 0) + 3], &[30, 20, 10]);
    }

    /// A scrolling line stays in its window, and comes round to where it began.
    #[test]
    fn a_ticker_draws_only_in_its_window_and_comes_round() {
        let font = FontArc::try_from_slice(include_bytes!("fonts/DMSans-Medium.ttf")).unwrap();
        let text = "2 lights on · playing The Hitchhiker's Guide to the Galaxy";
        let words = measure(&font, 23.0, text, 0.0);
        for panel in [false, true] {
            let still = canvas_for([10, 20, 30], panel);
            let line = Ticker { font: &font, size: 23.0, text, words, left: 95.0, width: 290.0, baseline: 412.0,
                                colour: [255, 211, 154], gap: 72.0 };
            let draw = |offset: f32| {
                let mut canvas = still.clone();
                let columns = ticker(&mut canvas, &still, &line, offset);
                (canvas, columns)
            };
            let (start, columns) = draw(0.0);
            assert_eq!(columns, (95, 384));
            let differs = |canvas: &Canvas, x: u32, y: u32| {
                let at = canvas.at(x, y);
                canvas.pixmap.data()[at..at + 3] != still.pixmap.data()[at..at + 3]
            };
            let drawn = |canvas: &Canvas, columns: std::ops::Range<u32>| {
                columns.flat_map(|x| (380..430).map(move |y| (x, y))).filter(|&(x, y)| differs(canvas, x, y)).count()
            };
            assert!(drawn(&start, 95..385) > 500, "the words show in the window (panel {panel})");
            assert_eq!(drawn(&start, 0..95) + drawn(&start, 385..SIZE), 0, "and nowhere else");
            let (later, _) = draw(140.0);
            assert!(later.pixmap.data() != start.pixmap.data(), "it moves");
            let (round, _) = draw(words + 72.0);
            assert!(round.pixmap.data() == start.pixmap.data(), "and comes round to its start");
        }
    }

    /// A Live ring drawn through a sweep ends as a ring drawn whole.
    #[test]
    fn a_live_ring_ends_as_a_whole_one() {
        let lit = |top: usize| move |i: usize| if i <= top { colour([255, 169, 87]) } else { colour(DIM) };
        for panel in [false, true] {
            let still = canvas_for(BLACK, panel);
            let (mut canvas, mut live) = (still.clone(), Live::new(&still));
            for top in [40, 41, 45, 90, 60, 61] {
                live.columns = None;
                ring_live(&mut canvas, &mut live, lit(top), &[top]);
                settle(&mut live);
            }
            let mut whole = still.clone();
            ring(&mut whole, lit(61), &[61]);
            assert!(canvas.pixmap.data() == whole.pixmap.data(), "panel {panel}");
            live.columns = None;
            ring_live(&mut canvas, &mut live, lit(61), &[61]);
            assert_eq!(live.columns, None, "nothing changed, nothing drawn");
        }
    }
}
