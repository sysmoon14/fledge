//! Drawing in the Nest's own look: its font (Akkurat, from the unit), its
//! ring of ticks, and Home Assistant's icons.

use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use tiny_skia::{Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

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

pub fn canvas(background: [u8; 3]) -> Pixmap {
    let mut pixmap = Pixmap::new(SIZE, SIZE).expect("480x480");
    pixmap.fill(colour(background));
    pixmap
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

/// A tick of the ring at `angle`, `length` in from its edge.
pub fn tick(pixmap: &mut Pixmap, angle: f32, length: f32, width: f32, c: Color) {
    let (sin, cos) = angle.to_radians().sin_cos();
    let at = |r: f32| (CENTRE + r * sin, CENTRE - r * cos);
    let ((x0, y0), (x1, y1)) = (at(RING - length), at(RING));
    let mut path = PathBuilder::new();
    path.move_to(x0, y0);
    path.line_to(x1, y1);
    if let Some(path) = path.finish() {
        let stroke = Stroke { width, line_cap: LineCap::Butt, ..Stroke::default() };
        pixmap.stroke_path(&path, &paint(c), &stroke, Transform::identity(), None);
    }
}

/// The whole ring, each tick coloured by `of(i)`; `mark` ticks drawn long
/// and white, as the dial's target.
pub fn ring(pixmap: &mut Pixmap, of: impl Fn(usize) -> Color, marks: &[usize]) {
    for i in 0..TICKS {
        if !marks.contains(&i) {
            tick(pixmap, tick_angle(i, TICKS), TICK, 3.0, of(i));
        }
    }
    for &i in marks {
        tick(pixmap, tick_angle(i, TICKS), MARK, 5.0, colour(WHITE));
    }
}

/// The tick that shows `fraction` (0..1) of the ring.
pub fn tick_at(fraction: f32) -> usize {
    ((fraction.clamp(0.0, 1.0) * (TICKS - 1) as f32).round()) as usize
}

pub fn circle(pixmap: &mut Pixmap, cx: f32, cy: f32, r: f32, c: Color) {
    if let Some(path) = PathBuilder::from_circle(cx, cy, r) {
        pixmap.fill_path(&path, &paint(c), FillRule::Winding, Transform::identity(), None);
    }
}

/// A rounded bar, as the Nest's lists select with.
pub fn pill(pixmap: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Color) {
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
        pixmap.fill_path(&path, &paint(c), FillRule::Winding, Transform::identity(), None);
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

/// Draws `text` from `x` with its baseline at `y`; returns where it ends.
#[allow(clippy::too_many_arguments)] // a drawing primitive: where, what, how
pub fn text_at(pixmap: &mut Pixmap, font: &FontArc, size: f32, x: f32, y: f32, text: &str, spacing: f32,
               c: [u8; 3], opacity: f32) -> f32 {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let mut x = x;
    let mut previous = None;
    let (width, height) = (pixmap.width() as i32, pixmap.height() as i32);
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(p) = previous {
            x += scaled.kern(p, id) + spacing;
        }
        let glyph = id.with_scale_and_position(scale, point(x, y));
        x += scaled.h_advance(id);
        previous = Some(id);
        let Some(outline) = scaled.outline_glyph(glyph) else { continue };
        let bounds = outline.px_bounds();
        let pixels = pixmap.pixels_mut();
        outline.draw(|gx, gy, coverage| {
            let (px, py) = (bounds.min.x as i32 + gx as i32, bounds.min.y as i32 + gy as i32);
            if px < 0 || py < 0 || px >= width || py >= height {
                return;
            }
            let a = coverage.clamp(0.0, 1.0) * opacity;
            let pixel = &mut pixels[(py * width + px) as usize];
            let blend = |d: u8, s: u8| (s as f32 * a + d as f32 * (1.0 - a)).round() as u8;
            // The canvas is opaque, so premultiplied is plain colour here.
            if let Some(p) = tiny_skia::PremultipliedColorU8::from_rgba(
                blend(pixel.red(), c[0]), blend(pixel.green(), c[1]), blend(pixel.blue(), c[2]), 255) {
                *pixel = p;
            }
        });
    }
    x
}

/// Draws `text` centred on `cx`.
#[allow(clippy::too_many_arguments)]
pub fn text(pixmap: &mut Pixmap, font: &FontArc, size: f32, cx: f32, y: f32, text: &str, spacing: f32, c: [u8; 3]) {
    let width = measure(font, size, text, spacing);
    text_at(pixmap, font, size, cx - width / 2.0, y, text, spacing, c, 1.0);
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

/// Draws an icon `size` across, centred on (cx, cy).
pub fn icon(pixmap: &mut Pixmap, icon: Icon, cx: f32, cy: f32, size: f32, c: Color) {
    let Ok(svg) = kurbo::BezPath::from_svg(icon.path) else { return };
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
    let Some(path) = path.finish() else { return };
    let scale = size / icon.box_size;
    // Material Symbols' viewBox starts at y -960.
    let top = if icon.box_size == 960.0 { 960.0 * scale } else { 0.0 };
    let transform = Transform::from_row(scale, 0.0, 0.0, scale, cx - size / 2.0, cy - size / 2.0 + top);
    pixmap.fill_path(&path, &paint(c), FillRule::Winding, transform, None);
}

/// Where the cards are: a dot each along the bottom, `active` white.
pub fn dots(pixmap: &mut Pixmap, count: usize, active: usize, on: [u8; 3]) {
    if count < 2 {
        return;
    }
    let gap = 18.0;
    let start = CENTRE - gap * (count - 1) as f32 / 2.0;
    for i in 0..count {
        let c = if i == active { colour(on) } else { alpha(on, 0.35) };
        circle(pixmap, start + gap * i as f32, 440.0, 4.5, c);
    }
}
