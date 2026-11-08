//! Drawing in the Nest's own look: its font (Akkurat, from the unit), its
//! ring of ticks, and Home Assistant's icons.

use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

use super::icons::Icon;

/// The round display's side, in pixels.
pub const SIZE: u32 = 480;
const CENTRE: f32 = SIZE as f32 / 2.0;
/// The ring's outer edge, and how long its ticks are: as nlclient's dial.
pub const RING: f32 = 236.0;
pub const TICK: f32 = 46.0;
pub const MARK: f32 = 64.0;
/// The ring's ticks span 300°, open at the bottom.
pub const TICKS: usize = 150;
const SPAN: f32 = 300.0;

/// The unit's fonts.
pub const FONT_DIR: &str = "/nestlabs/share/fonts";

pub struct Fonts {
    pub regular: FontArc,
    pub medium: FontArc,
}

impl Fonts {
    pub fn load(dir: &str) -> Result<Fonts, String> {
        let load = |name: &str| -> Result<FontArc, String> {
            let path = format!("{dir}/{name}");
            let data = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
            FontArc::try_from_vec(data).map_err(|e| format!("{path}: {e}"))
        };
        Ok(Fonts { regular: load("AkkuratNest-Regular.ttf")?, medium: load("AkkuratNest-Medium.ttf")? })
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

    fn fill_path(&mut self, path: &tiny_skia::Path, c: Color) {
        let (c, transform) = (self.colour(c), self.transform());
        self.pixmap.fill_path(path, &paint(c), FillRule::Winding, transform, None);
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

/// The whole ring, each tick coloured by `of(i)`; `mark` ticks drawn long
/// and white, as the dial's target.
///
/// Drawn from the ring's coverage, worked out once (ring_pixels): every
/// pixel near a tick, which tick, and how much of it a tick covers, short
/// or long. A frame then only blends those, where stroking 150 lines took
/// most of a frame on the unit.
pub fn ring(canvas: &mut Canvas, of: impl Fn(usize) -> Color, marks: &[usize]) {
    let mut colours = [[0u16; 4]; TICKS];
    for (i, c) in colours.iter_mut().enumerate() {
        let u = canvas.colour(if marks.contains(&i) { colour(WHITE) } else { of(i) }).to_color_u8();
        *c = [u.red() as u16, u.green() as u16, u.blue() as u16, u.alpha() as u16];
    }
    let mut marked = [false; TICKS];
    for &m in marks {
        if m < TICKS {
            marked[m] = true;
        }
    }
    let panel = canvas.panel;
    let data = canvas.pixmap.data_mut();
    for p in ring_pixels() {
        let tick = p.tick as usize;
        let cover = if marked[tick] { p.long } else { p.short } as u16;
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

struct RingPixel {
    /// The pixel in each layout (Canvas).
    upright: u32,
    panel: u32,
    tick: u8,
    /// How much of it a tick covers, of 255: a short tick, and a long one
    /// (the target's mark).
    short: u8,
    long: u8,
}

fn ring_pixels() -> &'static [RingPixel] {
    static PIXELS: std::sync::OnceLock<Vec<RingPixel>> = std::sync::OnceLock::new();
    PIXELS.get_or_init(|| {
        let mut pixels = Vec::new();
        let (inner, outer) = (RING - MARK - 1.0, RING + 1.0);
        let cover = |distance: f32, radius: f32, half: f32, length: f32| {
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
                let (short, long) = (cover(distance, radius, 1.5, TICK), cover(distance, radius, 2.5, MARK));
                if short > 0 || long > 0 {
                    pixels.push(RingPixel { upright: y * SIZE + x, panel: x * SIZE + (SIZE - 1 - y),
                                            tick: tick as u8, short, long });
                }
            }
        }
        pixels
    })
}

/// The tick that shows `fraction` (0..1) of the ring.
pub fn tick_at(fraction: f32) -> usize {
    ((fraction.clamp(0.0, 1.0) * (TICKS - 1) as f32).round()) as usize
}

pub fn circle(canvas: &mut Canvas, cx: f32, cy: f32, r: f32, c: Color) {
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
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
        blend_mask(canvas, whole as i32 + g.left, baseline + g.top, g.width, &g.coverage, c, opacity);
        x += scaled.h_advance(id);
        previous = Some(id);
    }
    x
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

/// Where the cards are: a dot each along the bottom, `active` white.
pub fn dots(canvas: &mut Canvas, count: usize, active: usize, on: [u8; 3]) {
    if count < 2 {
        return;
    }
    let gap = 18.0;
    let start = CENTRE - gap * (count - 1) as f32 / 2.0;
    for i in 0..count {
        let c = if i == active { colour(on) } else { alpha(on, 0.35) };
        circle(canvas, start + gap * i as f32, 440.0, 4.5, c);
    }
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
            dots(canvas, 5, 1, WHITE);
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
}
