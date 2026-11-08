//! The unit's display, ring and button, for the UI.
//!
//! Display: /dev/fb0, two 480x640 pages of 32-bit BGRX that nlclient flips
//! between; the visible one is the page the panning offset names. While the
//! UI owns the display, nlclient's patched eglSwapBuffers (custom_swap in
//! custom_screen.c) presents nothing, so the UI paints the visible page.
//! nlclient sees ownership through OWNER: this process's pid, checked live,
//! so the display returns to nlclient the moment this process is gone.
//!
//! Input: the ring (REL_DIAL on "2280000.adc") and the button (KEY_ENTER on
//! "gpio-keys"). Grabbed (EVIOCGRAB), only this process reads them. Events
//! written to a node go to its readers, or only to the grabber while it is
//! grabbed: `nudge` lets go for the length of one write to reach nlclient.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;

use tiny_skia::Pixmap;

pub const OWNER: &str = "/var/run/unfr0zen/screen.owner";
const RING: &str = "2280000.adc";
const BUTTON: &str = "gpio-keys";
pub const EV_KEY: u16 = 1;
pub const EV_REL: u16 = 2;
pub const KEY_ENTER: u16 = 28;
pub const REL_DIAL: u16 = 7;
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;

pub fn own(yes: bool) {
    let result = if yes {
        crate::settings_ui::publish("screen.owner", std::process::id().to_string().as_bytes())
    } else {
        match std::fs::remove_file(OWNER) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    };
    if let Err(e) = result {
        crate::logf!("ui: {OWNER}: {e}");
    }
}

/// Whether the display is lit.
pub fn lit() -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/backlight") else { return true };
    dir.flatten().any(|entry| {
        std::fs::read_to_string(entry.path().join("brightness"))
            .ok().and_then(|s| s.trim().parse::<u32>().ok()).unwrap_or(1) > 0
    })
}

pub struct Display {
    _file: File,
    map: *mut u8,
    len: usize,
    stride: usize,
}

// SAFETY: the mapping is only touched by the UI thread that owns Display.
unsafe impl Send for Display {}

fn sys(name: &str) -> Result<String, String> {
    let path = format!("/sys/class/graphics/fb0/{name}");
    std::fs::read_to_string(&path).map(|s| s.trim().to_owned()).map_err(|e| format!("{path}: {e}"))
}

impl Display {
    pub fn open() -> Result<Display, String> {
        let stride: usize = sys("stride")?.parse().map_err(|_| "fb0: stride".to_string())?;
        let size = sys("virtual_size")?;
        let rows: usize = size.split(',').nth(1).and_then(|s| s.parse().ok()).ok_or("fb0: virtual_size")?;
        let file = std::fs::OpenOptions::new().read(true).write(true).open("/dev/fb0")
            .map_err(|e| format!("/dev/fb0: {e}"))?;
        let len = stride * rows;
        // SAFETY: a shared mapping of the framebuffer device, len its size.
        let map = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED,
                       file.as_raw_fd(), 0)
        };
        if map == libc::MAP_FAILED {
            return Err(format!("/dev/fb0: mmap: {}", std::io::Error::last_os_error()));
        }
        Ok(Display { _file: file, map: map.cast(), len, stride })
    }

    /// The first byte of the visible page.
    fn page(&self) -> usize {
        let y: usize = sys("pan").ok().and_then(|s| s.split(',').nth(1)?.parse().ok()).unwrap_or(0);
        (y * self.stride).min(self.len.saturating_sub(self.stride * 480))
    }

    fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: map is len bytes, mapped for this Display's life.
        unsafe { std::slice::from_raw_parts_mut(self.map, self.len) }
    }

    /// The visible page's top 480 rows as a Frame: what nlclient shows.
    pub fn save(&mut self) -> Frame {
        let (start, stride) = (self.page(), self.stride);
        let page = &self.bytes()[start..];
        let mut frame = Vec::with_capacity(SIDE * ROW);
        for y in 0..SIDE {
            frame.extend_from_slice(&page[y * stride..y * stride + ROW]);
        }
        frame
    }

    /// Shows a Frame on the visible page.
    pub fn show(&mut self, frame: &[u8]) {
        self.rows(|y, row| row.copy_from_slice(&frame[y * ROW..(y + 1) * ROW]));
    }

    /// `from` sliding out and `to` in by `by` pixels of 480, sideways as the
    /// display stands: the framebuffer's rows, as it is mounted turned.
    pub fn slide(&mut self, from: &[u8], to: &[u8], by: usize, clockwise: bool) {
        let by = by.min(SIDE);
        self.rows(|y, row| {
            let (source, at) = if clockwise {
                if y + by < SIDE { (from, y + by) } else { (to, y + by - SIDE) }
            } else if y >= by { (from, y - by) } else { (to, y + SIDE - by) };
            row.copy_from_slice(&source[at * ROW..(at + 1) * ROW]);
        });
    }

    /// `to` over `from` at `alpha` of 256.
    pub fn fade(&mut self, from: &[u8], to: &[u8], alpha: u16) {
        let alpha = alpha.min(256);
        self.rows(|y, row| {
            let (a, b) = (&from[y * ROW..(y + 1) * ROW], &to[y * ROW..(y + 1) * ROW]);
            for ((out, &a), &b) in row.iter_mut().zip(a).zip(b) {
                *out = ((a as u16 * (256 - alpha) + b as u16 * alpha) >> 8) as u8;
            }
        });
    }

    fn rows(&mut self, mut each: impl FnMut(usize, &mut [u8])) {
        let (start, stride) = (self.page(), self.stride);
        let page = &mut self.bytes()[start..];
        for y in 0..SIDE {
            each(y, &mut page[y * stride..y * stride + ROW]);
        }
    }
}

/// The display's side, and a framebuffer row of it in bytes.
const SIDE: usize = 480;
const ROW: usize = SIDE * 4;

/// A screen as the framebuffer holds it: 480 rows of 480 BGRX pixels, the
/// picture turned a quarter anticlockwise, as the panel shows the
/// framebuffer turned a quarter clockwise.
pub type Frame = Vec<u8>;

/// An upright picture as a Frame: upright row r, column c is framebuffer
/// row c, column 479-r.
pub fn frame(pixmap: &Pixmap) -> Frame {
    let source = pixmap.data();
    let mut frame = vec![0u8; SIDE * ROW];
    for (y, row) in frame.as_chunks_mut::<ROW>().0.iter_mut().enumerate() {
        for (x, out) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let s = &source[((SIDE - 1 - x) * SIDE + y) * 4..];
            *out = [s[2], s[1], s[0], 0xff];
        }
    }
    frame
}

impl Drop for Display {
    fn drop(&mut self) {
        // SAFETY: unmaps what open mapped.
        unsafe { libc::munmap(self.map.cast(), self.len) };
    }
}

pub struct Event {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

pub struct Input {
    ring: File,
    button: File,
    grabbed: bool,
}

fn node(name: &str) -> Result<File, String> {
    let dir = std::fs::read_dir("/sys/class/input").map_err(|e| format!("/sys/class/input: {e}"))?;
    for entry in dir.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if !file.starts_with("event") {
            continue;
        }
        let device = std::fs::read_to_string(entry.path().join("device/name")).unwrap_or_default();
        if device.trim() == name {
            let path = Path::new("/dev/input").join(&file);
            return std::fs::OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK)
                .open(&path).map_err(|e| format!("{}: {e}", path.display()));
        }
    }
    Err(format!("no input device {name}"))
}

use std::os::unix::fs::OpenOptionsExt;

fn grab(file: &File, yes: bool) -> std::io::Result<()> {
    // SAFETY: EVIOCGRAB takes an int by value.
    if unsafe { libc::ioctl(file.as_raw_fd(), EVIOCGRAB as _, yes as libc::c_int) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// One input_event as the unit's 32-bit kernel lays it out.
fn encode(kind: u16, code: u16, value: i32) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[8..10].copy_from_slice(&kind.to_ne_bytes());
    out[10..12].copy_from_slice(&code.to_ne_bytes());
    out[12..16].copy_from_slice(&value.to_ne_bytes());
    out
}

impl Input {
    pub fn open() -> Result<Input, String> {
        Ok(Input { ring: node(RING)?, button: node(BUTTON)?, grabbed: false })
    }

    pub fn grab(&mut self, yes: bool) {
        if self.grabbed == yes {
            return;
        }
        for file in [&self.ring, &self.button] {
            if let Err(e) = grab(file, yes) {
                crate::logf!("ui: grab {yes}: {e}");
            }
        }
        self.grabbed = yes;
    }

    /// Waits up to `timeout` for input; returns what arrived.
    pub fn wait(&mut self, timeout: Duration) -> Vec<Event> {
        let mut fds = [
            libc::pollfd { fd: self.ring.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: self.button.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: fds is two valid pollfds.
        unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout.as_millis() as libc::c_int) };
        let mut events = Vec::new();
        for file in [&mut self.ring, &mut self.button] {
            let mut buffer = [0u8; 16 * 32];
            while let Ok(read) = file.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                for raw in buffer[..read].as_chunks::<16>().0 {
                    let kind = u16::from_ne_bytes([raw[8], raw[9]]);
                    let code = u16::from_ne_bytes([raw[10], raw[11]]);
                    let value = i32::from_ne_bytes([raw[12], raw[13], raw[14], raw[15]]);
                    if kind == EV_REL || kind == EV_KEY {
                        events.push(Event { kind, code, value });
                    }
                }
            }
        }
        events
    }

    /// Activity nlclient sees, that changes nothing: the ring a count each
    /// way in one report. It keeps nlclient's display awake, or wakes it.
    pub fn nudge(&mut self) {
        let mut report = Vec::with_capacity(48);
        report.extend(encode(EV_REL, REL_DIAL, 1));
        report.extend(encode(EV_REL, REL_DIAL, -1));
        report.extend(encode(0, 0, 0));
        self.send(true, &report);
    }

    /// A press and release of the button, for nlclient.
    pub fn press(&mut self) {
        let down: Vec<u8> = [encode(EV_KEY, KEY_ENTER, 1), encode(0, 0, 0)].concat();
        let up: Vec<u8> = [encode(EV_KEY, KEY_ENTER, 0), encode(0, 0, 0)].concat();
        self.send(false, &down);
        std::thread::sleep(Duration::from_millis(120));
        self.send(false, &up);
    }

    fn send(&mut self, ring: bool, report: &[u8]) {
        let was = self.grabbed;
        self.grab(false);
        let file = if ring { &mut self.ring } else { &mut self.button };
        if let Err(e) = file.write_all(report) {
            crate::logf!("ui: inject: {e}");
        }
        // The write reached this process's reader too: it is not input.
        let mut discard = [0u8; 16 * 8];
        while matches!(file.read(&mut discard), Ok(n) if n > 0) {}
        self.grab(was);
    }
}
