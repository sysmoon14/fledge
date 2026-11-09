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


pub const OWNER: &str = "/var/run/unfr0zen/screen.owner";
const RING: &str = "2280000.adc";
const BUTTON: &str = "gpio-keys";
const BEEPER: &str = "pwm-beeper";
const EV_SND: u16 = 0x12;
const SND_TONE: u16 = 2;
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

/// Asks nlclient to light its display (custom_screen.c's keeper).
pub fn wake_display() {
    if let Err(e) = crate::settings_ui::publish("display.wake", b"") {
        crate::logf!("ui: display.wake: {e}");
    }
}

/// The Nest's click: the beeper's tone, at nlclient's own clickfrequency
/// and clickduration (/nestlabs/etc/client.config).
pub struct Beeper(Option<File>);

impl Beeper {
    pub fn open() -> Beeper {
        Beeper(node(BEEPER).map_err(|e| crate::logf!("ui: {e}: no clicks")).ok())
    }

    /// A notification's chime: two short rising notes.
    pub fn chime(&mut self) {
        let Some(file) = &mut self.0 else { return };
        for (tone, ms) in [(1568, 90), (0, 40), (2093, 150)] {
            let _ = file.write_all(&[encode(EV_SND, SND_TONE, tone), encode(0, 0, 0)].concat());
            std::thread::sleep(Duration::from_millis(ms));
        }
        let _ = file.write_all(&[encode(EV_SND, SND_TONE, 0), encode(0, 0, 0)].concat());
    }

    pub fn click(&mut self) {
        let Some(file) = &mut self.0 else { return };
        let _ = file.write_all(&[encode(EV_SND, SND_TONE, 2000), encode(0, 0, 0)].concat());
        std::thread::sleep(Duration::from_millis(3));
        let _ = file.write_all(&[encode(EV_SND, SND_TONE, 0), encode(0, 0, 0)].concat());
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
    file: File,
    map: *mut u8,
    len: usize,
    stride: usize,
    /// While the UI owns the display, it draws into the page that isn't
    /// shown and then pans to it, so no frame shows half drawn.
    flip: Option<Flip>,
}

/// The two pages, as nlclient's EGL flips between them: which one shows,
/// which one nlclient showed (its own again when the UI is done), and the
/// rows each lacks of the newest frame.
struct Flip {
    pages: [usize; 2],
    front: usize,
    home: usize,
    stale: [Option<(usize, usize)>; 2],
}

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const CAPTURE_REQUEST: &str = "/var/run/unfr0zen/capture.request";
const FBIOPAN_DISPLAY: libc::c_ulong = 0x4606;

fn union(a: Option<(usize, usize)>, b: Option<(usize, usize)>) -> Option<(usize, usize)> {
    match (a, b) {
        (Some((a0, a1)), Some((b0, b1))) => Some((a0.min(b0), a1.max(b1))),
        (a, None) => a,
        (None, b) => b,
    }
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
        Ok(Display { file, map: map.cast(), len, stride, flip: None })
    }

    /// The panning offset's row: the page shown.
    fn pan(&self) -> usize {
        sys("pan").ok().and_then(|s| s.split(',').nth(1)?.parse().ok()).unwrap_or(0)
    }

    /// The first byte of the visible page.
    fn page(&self) -> usize {
        match &self.flip {
            Some(flip) => flip.pages[flip.front] * self.stride,
            None => (self.pan() * self.stride).min(self.len.saturating_sub(self.stride * 480)),
        }
    }

    /// Pans the display to the page starting at row `y`.
    fn pan_to(&mut self, y: usize) -> std::io::Result<()> {
        let mut var = [0u32; 40];
        // SAFETY: fb_var_screeninfo is 160 bytes; yoffset its sixth word.
        unsafe {
            if libc::ioctl(self.file.as_raw_fd(), FBIOGET_VSCREENINFO as _, var.as_mut_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            var[5] = y as u32;
            if libc::ioctl(self.file.as_raw_fd(), FBIOPAN_DISPLAY as _, var.as_mut_ptr()) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }

    /// Whether the display shows a page other than the UI's own last frame:
    /// waking can pan it back to nlclient's.
    pub fn lost(&self) -> bool {
        match &self.flip {
            Some(flip) => self.pan() != flip.pages[flip.front],
            None => false,
        }
    }

    /// The UI takes the display: from now on frames are drawn unseen and
    /// flipped to. Without two pages it draws straight on the shown one.
    pub fn begin(&mut self) {
        let rows = self.len / self.stride;
        if rows < 2 * SIDE {
            return;
        }
        let pages = [0, rows / 2];
        let home = usize::from(self.pan() >= rows / 2);
        self.flip = Some(Flip { pages, front: home, home, stale: [Some((0, SIDE - 1)); 2] });
    }

    /// Gives the display back showing `frame` (nlclient's) on nlclient's
    /// own page, as its EGL left it.
    pub fn end(&mut self, frame: &[u8]) {
        let Some(flip) = self.flip.take() else { return self.show(frame) };
        let start = flip.pages[flip.home] * self.stride;
        let stride = self.stride;
        let page = &mut self.bytes()[start..];
        for y in 0..SIDE {
            page[y * stride..y * stride + ROW].copy_from_slice(&frame[y * ROW..(y + 1) * ROW]);
        }
        if let Err(e) = self.pan_to(flip.pages[flip.home]) {
            crate::logf!("ui: pan: {e}");
        }
    }

    /// Writes rows into the page to show next and flips to it: `rows`
    /// those changed (None, all), each given by `each`.
    fn present(&mut self, rows: Option<(usize, usize)>, mut each: impl FnMut(usize, &mut [u8])) {
        let stride = self.stride;
        let Some(flip) = self.flip.as_mut() else {
            let start = self.page();
            let (first, last) = rows.unwrap_or((0, SIDE - 1));
            let page = &mut self.bytes()[start..];
            for y in first..=last.min(SIDE - 1) {
                each(y, &mut page[y * stride..y * stride + ROW]);
            }
            return;
        };
        let back = 1 - flip.front;
        let all = Some((0, SIDE - 1));
        let changed = if rows.is_none() { all } else { rows };
        let (first, last) = union(changed, flip.stale[back]).unwrap_or((0, SIDE - 1));
        flip.stale[back] = None;
        flip.stale[flip.front] = union(flip.stale[flip.front], changed);
        flip.front = back;
        let y = flip.pages[back];
        let start = y * stride;
        let page = &mut self.bytes()[start..];
        for row in first..=last.min(SIDE - 1) {
            each(row, &mut page[row * stride..row * stride + ROW]);
        }
        // For tools (utils/nest-screen.py): the frame about to show, whole,
        // where reading the framebuffer could catch the next one being drawn.
        if std::fs::remove_file(CAPTURE_REQUEST).is_ok() {
            let frame: Vec<u8> = (0..SIDE).flat_map(|row| page[row * stride..row * stride + ROW].to_vec()).collect();
            let _ = crate::settings_ui::publish("capture.raw", &frame);
        }
        if let Err(e) = self.pan_to(y) {
            crate::logf!("ui: pan: {e}");
        }
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

    /// Shows a Frame.
    pub fn show(&mut self, frame: &[u8]) {
        self.present(None, |y, row| row.copy_from_slice(&frame[y * ROW..(y + 1) * ROW]));
    }

    /// Shows a Frame of which only rows `first` to `last` changed.
    pub fn show_rows(&mut self, frame: &[u8], first: usize, last: usize) {
        self.present(Some((first, last)), |y, row| row.copy_from_slice(&frame[y * ROW..(y + 1) * ROW]));
    }

    /// `from` sliding out and `to` in by `by` pixels of 480, sideways as the
    /// display stands: the framebuffer's rows, as it is mounted turned.
    pub fn slide(&mut self, from: &[u8], to: &[u8], by: usize, clockwise: bool) {
        let by = by.min(SIDE);
        self.present(None, |y, row| {
            let (source, at) = if clockwise {
                if y + by < SIDE { (from, y + by) } else { (to, y + by - SIDE) }
            } else if y >= by { (from, y - by) } else { (to, y + SIDE - by) };
            row.copy_from_slice(&source[at * ROW..(at + 1) * ROW]);
        });
    }

    /// `to` over `from` at `alpha` of 256.
    pub fn fade(&mut self, from: &[u8], to: &[u8], alpha: u16) {
        let alpha = alpha.min(256) as u32;
        let mut line = [0u8; ROW];
        self.present(None, |y, row| {
            let (a, b) = (&from[y * ROW..(y + 1) * ROW], &to[y * ROW..(y + 1) * ROW]);
            // A pixel at a time, two channels to a multiply (blue and red,
            // then green and the unused byte), into a line written to the
            // framebuffer whole: its memory is slow a byte at a time.
            for ((out, a), b) in line.as_chunks_mut::<4>().0.iter_mut().zip(a.as_chunks::<4>().0).zip(b.as_chunks::<4>().0) {
                let (a, b) = (u32::from_le_bytes(*a), u32::from_le_bytes(*b));
                let even = ((a & 0x00ff_00ff) * (256 - alpha) + (b & 0x00ff_00ff) * alpha) >> 8 & 0x00ff_00ff;
                let odd = (((a >> 8) & 0x00ff_00ff) * (256 - alpha) + ((b >> 8) & 0x00ff_00ff) * alpha) & 0xff00_ff00;
                *out = (even | odd).to_le_bytes();
            }
            row.copy_from_slice(&line);
        });
    }

}

/// The display's side, and a framebuffer row of it in bytes.
const SIDE: usize = 480;
const ROW: usize = SIDE * 4;

/// A screen as the framebuffer holds it: 480 rows of 480 BGRX pixels, the
/// picture turned a quarter anticlockwise, as the panel shows the
/// framebuffer turned a quarter clockwise.
pub type Frame = Vec<u8>;

/// A screen drawn for the panel (paint::for_panel) as a Frame: as it is.
pub fn frame(canvas: super::paint::Canvas) -> Frame {
    canvas.pixmap.take()
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

    /// A turn of the ring for nlclient, as a hand makes one: `reports`
    /// reports of `raw` counts, 25 ms apart (its menus take no other).
    pub fn hand_turn(&mut self, raw: i32, reports: usize) {
        let report: Vec<u8> = [encode(EV_REL, REL_DIAL, raw), encode(0, 0, 0)].concat();
        for _ in 0..reports {
            self.send(true, &report);
            std::thread::sleep(Duration::from_millis(25));
        }
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
