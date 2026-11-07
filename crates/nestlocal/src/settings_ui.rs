//! Small local file protocol with the injected native settings screen.
//! /var/run is ephemeral: requests and pairing credentials never survive boot.
use std::path::Path;

pub const DIR: &str = "/var/run/unfr0zen";

pub fn publish(name: &str, data: &[u8]) -> std::io::Result<()> {
    let path = Path::new(DIR).join(name);
    let temporary = Path::new(DIR).join(format!("{name}.partial"));
    // nlclient renders this on the physical display; pairing credentials
    // and commands remain accessible only to root.
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().create(true).truncate(true).write(true).mode(0o600).open(&temporary)?;
    file.write_all(data)?;
    drop(file);
    std::fs::rename(temporary, path)
}

pub fn status(protocol: &str, text: &str) {
    if let Err(e) = publish(&format!("{protocol}.status"), text.as_bytes()) {
        crate::logf!("settings UI: {e}");
    }
}

pub fn take_request(protocol: &str) -> bool { take_request_age(protocol).is_some() }

pub fn take_request_age(protocol: &str) -> Option<std::time::Duration> {
    let path = Path::new(DIR).join(format!("{protocol}.request"));
    let age = std::fs::metadata(&path).ok()?.modified().ok()
        .and_then(|time| time.elapsed().ok()).unwrap_or_default();
    std::fs::remove_file(path).ok()?;
    Some(age)
}

/// A request's contents, taken atomically: one the screen leaves while this
/// reads is a new request, not lost.
pub fn take_request_text(protocol: &str) -> Option<String> {
    let path = Path::new(DIR).join(format!("{protocol}.request"));
    let taken = Path::new(DIR).join(format!("{protocol}.taken"));
    std::fs::rename(path, &taken).ok()?;
    let text = std::fs::read_to_string(&taken).ok();
    let _ = std::fs::remove_file(taken);
    text
}

pub fn init(matter: bool, zigbee: bool, doom: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(DIR)?;
    std::fs::set_permissions(DIR, std::fs::Permissions::from_mode(0o700))?;
    // Publish configured features independently of service/radio readiness.
    let features = (matter as u8) | ((zigbee as u8) << 1) | ((doom as u8) << 2);
    publish("features", &[b'0' + features])?;
    publish_activity_icons()?;
    publish_light_ring()?;
    for name in ["display.request", "display.taken", "lights.request", "lights.taken", "matter.request", "matter-reset.request", "zigbee.request", "doom.request", "matter.code", "matter.png"] {
        let _ = std::fs::remove_file(Path::new(DIR).join(name));
    }
    status("matter", if matter { "Starting Matter..." } else { "Matter not enabled" });
    status("zigbee", if zigbee { "Starting Zigbee..." } else { "Zigbee not enabled" });
    status("doom", if doom { "Ready" } else { "Unavailable" });
    Ok(())
}

pub fn matter_codes(comm: &rs_matter::BasicCommData, capabilities: rs_matter::pairing::DiscoveryCapabilities) -> Result<(), String> {
    let (png_bytes, code) = encode_matter_codes(comm, capabilities)?;
    publish("matter.png", &png_bytes).map_err(|e| e.to_string())?;
    publish("matter.code", code.as_bytes()).map_err(|e| e.to_string())
}

/// A length of the gen 3's 480-pixel settings screen, on this unit's display.
fn scaled(length: usize) -> usize {
    length * crate::device::current().display / 480
}

// Eight small frames rendered through nlclient's native PNG API. Keeping the
// animation in the view avoids coupling UI refresh to radio/commissioning waits.
/// Ticks of a light's brightness ring, one per 2%, and the arc they span:
/// a dimmer knob's 300°, open at the bottom, lit clockwise from its left end.
pub const RING_TICKS: usize = 50;
const RING_ARC: f64 = 300.0;

/// The light page's brightness ring: each tick lit (light-ring-NN-on.png)
/// and unlit (-off), and light-ring.layout, "<count>\n" then "<x> <y>\n" per
/// tick, the top left of its image relative to the display's centre.
fn publish_light_ring() -> std::io::Result<()> {
    let (layout, images) = light_ring(crate::device::current().display)?;
    for (name, png) in images {
        publish(&name, &png)?;
    }
    publish("light-ring.layout", layout.as_bytes())
}

/// PNG images by file name.
type Images = Vec<(String, Vec<u8>)>;

/// The ring for a display this many pixels across: its layout, and each
/// tick's images.
fn light_ring(display: usize) -> std::io::Result<(String, Images)> {
    let display = display as f64;
    let mut images = Vec::new();
    // At the gen 3's 480 pixels: 22-pixel ticks, 4 wide, inside the bezel.
    let (outer, inner, width) = (display * 226.0 / 480.0, display * 204.0 / 480.0, display * 4.0 / 480.0);
    let size = ((outer - inner) + width * 2.0).ceil() as usize + 2;
    let mut layout = format!("{RING_TICKS}\n");
    for tick in 0..RING_TICKS {
        // Clockwise from the top; the arc starts at its lower left.
        let degrees = -RING_ARC / 2.0 + RING_ARC * tick as f64 / (RING_TICKS - 1) as f64;
        let (sin, cos) = degrees.to_radians().sin_cos();
        let middle = (outer + inner) / 2.0;
        let (mx, my) = (sin * middle, -cos * middle);
        let left = (mx - size as f64 / 2.0).round();
        let top = (my - size as f64 / 2.0).round();
        layout += &format!("{} {}\n", left as i32, top as i32);
        // The tick's two ends, in its image's pixels.
        let (ax, ay) = (sin * inner - left, -cos * inner - top);
        let (bx, by) = (sin * outer - left, -cos * outer - top);
        for (lit, colour) in [(true, [255u8, 180, 60]), (false, [80u8, 80, 80])] {
            let mut rgba = vec![0u8; size * size * 4];
            for y in 0..size { for x in 0..size {
                let distance = segment_distance((x as f64 + 0.5, y as f64 + 0.5), (ax, ay), (bx, by));
                let coverage = (width / 2.0 + 0.5 - distance).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    let at = (y * size + x) * 4;
                    rgba[at..at + 3].copy_from_slice(&colour);
                    rgba[at + 3] = (coverage * 255.0).round() as u8;
                }
            } }
            images.push((format!("light-ring-{tick:02}-{}.png", if lit { "on" } else { "off" }), encode_rgba(size, &rgba)?));
        }
    }
    Ok((layout, images))
}

fn segment_distance((px, py): (f64, f64), (ax, ay): (f64, f64), (bx, by): (f64, f64)) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

fn encode_rgba(size: usize, rgba: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut data, size as u32, size as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(data)
}

fn publish_activity_icons() -> std::io::Result<()> {
    let size = scaled(56);
    let (centre, orbit, dot_radius) = (size as f64 / 2.0, size as f64 * 20.0 / 56.0, size as f64 * 4.0 / 56.0);
    for frame in 0..8 {
        let mut rgba = vec![0u8; size * size * 4];
        for dot in 0..8 {
            let angle = dot as f64 * std::f64::consts::TAU / 8.0;
            let cx = centre + angle.sin() * orbit;
            let cy = centre - angle.cos() * orbit;
            let alpha = 64 + ((dot + 8 - frame) % 8) * 27;
            for y in 0..size { for x in 0..size {
                if (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2) <= dot_radius.powi(2) {
                    rgba[(y * size + x) * 4..(y * size + x) * 4 + 4]
                        .copy_from_slice(&[0, 174, 239, alpha as u8]);
                }
            } }
        }
        let mut data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut data, size as u32, size as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&rgba)?;
        }
        publish(&format!("activity-{frame}.png"), &data)?;
    }
    Ok(())
}

fn encode_matter_codes(comm: &rs_matter::BasicCommData, capabilities: rs_matter::pairing::DiscoveryCapabilities) -> Result<(Vec<u8>, String), String> {
    use rs_matter::pairing::qr::{Qr, QrPayload, CommFlowType, no_optional_data};
    use rs_matter::dm::devices::test::{TEST_VID, TEST_PID};
    let payload = QrPayload::new(capabilities, CommFlowType::Standard, comm.clone(), TEST_VID, TEST_PID, "", no_optional_data);
    let mut text_buf = [0u8; 256];
    let (text, _) = payload.as_str(&mut text_buf).map_err(|e| e.to_string())?;
    let mut temporary = [0u8; 4096];
    let mut encoded = [0u8; 4096];
    let qr = Qr::compute(text, &mut temporary, &mut encoded).map_err(|e| e.to_string())?;
    // Four modules of quiet zone, integral scaling, no interpolation, in
    // the slot the gen 3's 192 pixels are on this display.
    let modules = qr.size() + 8;
    let scale = scaled(192) as u32 / modules;
    if scale == 0 { return Err("Matter QR exceeds display slot".into()); }
    let size = modules * scale;
    let pixels: Vec<u8> = (0..size).flat_map(|y| {
        let qr = &qr;
        (0..size).map(move |x| {
            let mx = (x / scale) as i32 - 4;
            let my = (y / scale) as i32 - 4;
            if mx >= 0 && my >= 0 && mx < qr.size() as i32 && my < qr.size() as i32 && qr.get_module(mx, my) { 0 } else { 255 }
        })
    }).collect();
    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, size, size);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&pixels).map_err(|e| e.to_string())?;
    }
    Ok((png_bytes, comm.compute_pretty_pairing_code().to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_light_ring_spans_a_dimmer_arc_inside_the_display() {
        for display in [480usize, 320] {
            let (layout, images) = super::light_ring(display).unwrap();
            let mut lines = layout.lines();
            assert_eq!(lines.next(), Some(super::RING_TICKS.to_string().as_str()));
            let ticks: Vec<(i32, i32)> = lines.map(|l| {
                let mut xy = l.split(' ').map(|v| v.parse::<i32>().unwrap());
                (xy.next().unwrap(), xy.next().unwrap())
            }).collect();
            assert_eq!(ticks.len(), super::RING_TICKS);
            assert_eq!(images.len(), super::RING_TICKS * 2);
            let half = display as i32 / 2;
            for &(x, y) in &ticks {
                assert!(x >= -half && y >= -half && x < half && y < half, "{display}: {x} {y}");
            }
            // Mirror images about the vertical, open at the bottom, starting lower left.
            let (first, last) = (ticks[0], ticks[super::RING_TICKS - 1]);
            assert!(first.0 < 0 && first.1 > 0 && last.0 > 0 && last.1 > 0);
            assert!((first.1 - last.1).abs() <= 1);
            let middle = ticks[super::RING_TICKS / 2];
            assert!(middle.1 < -half / 2, "the middle tick is at the top");
            // Every image is a PNG of the same small square.
            let mut reader = png::Decoder::new(images[0].1.as_slice()).read_info().unwrap();
            let mut buffer = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut buffer).unwrap();
            assert!(info.width == info.height && (info.width as usize) < display / 8);
            assert!(buffer.chunks(4).any(|p| p[3] == 255), "a tick is drawn");
        }
    }

    use super::*;
    use rs_matter::{BasicCommData, sc::pase::{Spake2pVerifierPassword, Spake2pVerifierPasswordRef}};

    #[test]
    fn qr_png_and_manual_code_follow_configured_credentials() {
        for (pin, discriminator) in [(20202021u32, 0u16), (34567890, 3840)] {
            let comm = BasicCommData {
                password: Spake2pVerifierPassword::new_from_ref(Spake2pVerifierPasswordRef::new(&pin.to_le_bytes())),
                discriminator,
            };
            let (png, code) = encode_matter_codes(&comm, rs_matter::pairing::DiscoveryCapabilities::IP).unwrap();
            assert_eq!(code, comm.compute_pretty_pairing_code().as_str());
            let mut reader = png::Decoder::new(png.as_slice()).read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!(info.width, info.height);
            assert!(info.width <= 192);
            assert_eq!(info.color_type, png::ColorType::Grayscale);
            assert!(pixels.contains(&0));
            assert!(pixels[..info.width as usize * 4].iter().all(|p| *p == 255));
            // Finder patterns seven modules wide in three corners, inside a
            // quiet zone of four modules on every side.
            let width = info.width as usize;
            let dark = |x: usize, y: usize| pixels[y * width + x] == 0;
            let top = (0..width).find(|&y| (0..width).any(|x| dark(x, y))).unwrap();
            let bottom = (0..width).rev().find(|&y| (0..width).any(|x| dark(x, y))).unwrap();
            let left = (0..width).find(|&x| dark(x, top)).unwrap();
            let right = (0..width).rev().find(|&x| dark(x, top)).unwrap();
            let finder = (left..width).take_while(|&x| dark(x, top)).count();
            assert_eq!(top, left);
            assert_eq!(finder * 4, left * 7, "finder {finder} px, quiet zone {left} px");
            assert_eq!((width - 1 - right, width - 1 - bottom), (left, left));
            assert_eq!(width % (left / 4), 0);
            if let Ok(dir) = std::env::var("NLCLIENT_TEST_ARTIFACTS") {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(Path::new(&dir).join("matter.png"), &png).unwrap();
                std::fs::write(Path::new(&dir).join("matter.code"), &code).unwrap();
            }
        }
    }
}
