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
//!
//! The gen 2: its display is 320x320 (two pages of 320x640), each frame
//! scaled by 2/3 on its way to the panel and by 3/2 read back, so the UI
//! draws at 480 as on the gen 3. Its ring is an optical sensor ("avago-adbs-
//! a330", REL_X, clockwise up, about 4000 counts a turn) and its button the
//! power button ("twl4030_pwrbutton", KEY_POWER): read as REL_DIAL and
//! KEY_ENTER, and written back to nlclient as its own.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::Duration;

use super::prof;

pub const OWNER: &str = "/var/run/unfr0zen/screen.owner";
const REST: &str = "/var/run/unfr0zen/screen.rest";
const KEEPER_RESTS: &str = "/var/run/unfr0zen/keeper.rests";
const KEEPER: &str = "/var/run/unfr0zen/keeper.pid";
const FB_PRIVATE: &str = "/var/run/unfr0zen/fb.private";
const BEEPER: &str = "pwm-beeper";
const EV_SND: u16 = 0x12;
const SND_TONE: u16 = 2;
pub const EV_KEY: u16 = 1;
pub const EV_REL: u16 = 2;
pub const KEY_ENTER: u16 = 28;
pub const REL_DIAL: u16 = 7;
const REL_X: u16 = 0;
const KEY_POWER: u16 = 116;

/// A generation's ring and button: their input devices, and the codes they
/// report. `ring_sign` turns its counts to the gen 3's (anticlockwise up).
struct Controls {
    ring: &'static str,
    ring_code: u16,
    ring_sign: i32,
    button: &'static str,
    button_code: u16,
}

const CONTROLS: [Controls; 2] = [
    Controls { ring: "2280000.adc", ring_code: REL_DIAL, ring_sign: 1, button: "gpio-keys", button_code: KEY_ENTER },
    Controls { ring: "avago-adbs-a330", ring_code: REL_X, ring_sign: 1, button: "twl4030_pwrbutton", button_code: KEY_POWER },
];
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;

pub fn own(yes: bool) {
    let result = if yes {
        crate::settings_ui::publish("screen.owner", format!("{} 0", std::process::id()).as_bytes())
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

/// Whether the display is another process's: the owner file names a live
/// process that isn't this one, and has no heartbeat, which only the UI
/// writes (Doom's launcher, which may outlive a restart of nestlocal). A
/// file with a heartbeat is an earlier nestlocal's, whose pid may be reused.
pub fn owned_elsewhere() -> bool {
    foreign_owner(&std::fs::read_to_string(OWNER).unwrap_or_default())
        // SAFETY: signal 0 only asks whether the process exists.
        .is_some_and(|pid| unsafe { libc::kill(pid, 0) } == 0)
}

/// The pid an owner file names, when it has no heartbeat and isn't ours.
fn foreign_owner(text: &str) -> Option<i32> {
    let mut words = text.split_whitespace();
    let pid = words.next()?.parse::<i32>().ok().filter(|&pid| pid > 1 && pid as u32 != std::process::id())?;
    words.next().is_none().then_some(pid)
}

/// The owner's heartbeat: nlclient's patch (keeper.rests and on) takes the
/// display back from an owner whose beat stops changing.
pub fn beat(beat: u32) {
    if let Err(e) = crate::settings_ui::publish("screen.owner", format!("{} {beat}", std::process::id()).as_bytes()) {
        crate::logf!("ui: {OWNER}: {e}");
    }
}

/// Favours the calling thread (the UI's) over the rest of the unit's work
/// when both want its one core: a frame late shows as a stutter, and what
/// it holds up meanwhile (MQTT, nlclient's own housekeeping) is in no hurry.
/// It sleeps between frames and while nothing moves.
///
/// Not on the gen 2, whose frames take most of its slower core: nlclient
/// held up there misses its heartbeats, and nlheartbeatd restarts the unit.
pub fn favour() {
    let favoured = crate::device::current().generation != 2;
    // SAFETY: setpriority on the calling thread (0), which Linux takes a
    // nice value for apart from the process's other threads.
    #[cfg(target_os = "linux")]
    if favoured && unsafe { libc::setpriority(libc::PRIO_PROCESS as _, 0, -10) } != 0 {
        crate::logf!("ui: priority: {}", std::io::Error::last_os_error());
    }
    let _ = favoured;
}

/// Whether nlclient's patch can leave the display to go out while the UI
/// holds it (screen.rest), and takes it back from a hung owner.
pub fn rests() -> bool {
    Path::new(KEEPER_RESTS).exists()
}

/// Leaves the display to nlclient's own timing (true) though the UI holds
/// it, or has nlclient keep it lit while the UI owns it.
pub fn rest(yes: bool) {
    let result = if yes {
        crate::settings_ui::publish("screen.rest", b"")
    } else {
        match std::fs::remove_file(REST) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    };
    if let Err(e) = result {
        crate::logf!("ui: {REST}: {e}");
    }
}

/// Whether the patch's display keeper runs in the nlclient running now: it
/// starts at nlclient's first bucket commit, which on the gen 2 can be long
/// in coming (Sender::nudge_nlclient).
pub fn keeper_runs() -> bool {
    let Ok(pid) = std::fs::read_to_string(KEEPER) else { return false };
    std::fs::read_to_string(format!("/proc/{}/comm", pid.trim())).is_ok_and(|comm| comm.trim() == "nlclient")
}

/// Whether the gen 2's nlclient draws into private memory while the UI
/// owns the display (custom_screen.c's custom_present): FB_PRIVATE names a
/// running nlclient. Not KEEPER's: its display keeper starts long after
/// nlclient does on a gen 2.
fn nlclient_private() -> bool {
    let Some(pid) = std::fs::read_to_string(FB_PRIVATE).ok().and_then(|s| s.trim().parse::<u32>().ok()) else {
        return false;
    };
    std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|comm| comm.trim() == "nlclient")
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
        self.tick(2000, 3);
    }

    /// A softer, lower click: a light turned off by the ring.
    pub fn click_low(&mut self) {
        self.tick(900, 5);
    }

    fn tick(&mut self, hertz: i32, ms: u64) {
        let Some(file) = &mut self.0 else { return };
        let _ = file.write_all(&[encode(EV_SND, SND_TONE, hertz), encode(0, 0, 0)].concat());
        std::thread::sleep(Duration::from_millis(ms));
        let _ = file.write_all(&[encode(EV_SND, SND_TONE, 0), encode(0, 0, 0)].concat());
    }
}

/// Switches the display's backlight on or off (bl_power), leaving its
/// brightness, which nlclient sets and lit() reads, as it is: off, a lit
/// display is truly dark, where black pixels still glow.
pub fn backlight(on: bool) {
    let Ok(dir) = std::fs::read_dir("/sys/class/backlight") else { return };
    for entry in dir.flatten() {
        // FB_BLANK_UNBLANK, FB_BLANK_POWERDOWN.
        if let Err(e) = std::fs::write(entry.path().join("bl_power"), if on { "0" } else { "4" }) {
            crate::logf!("ui: backlight {}: {e}", entry.path().display());
        }
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
    /// The panel's side in pixels, as a Frame has it (paint::side).
    side: usize,
    /// Whether the panel shows the framebuffer turned a quarter clockwise,
    /// as a Frame is laid out for (the gen 3's). The gen 2's shows it as it
    /// is, so a Frame is turned back a quarter on its way.
    turned: bool,
    /// While the UI owns the display, it draws into the page that isn't
    /// shown and then pans to it, so no frame shows half drawn.
    flip: Option<Flip>,
    /// The UI's last frame, as it went to the display: a transition starts
    /// from it, as reading the display back is slow (its memory isn't cached).
    shown: Frame,
    /// When the gen 2's nlclient was last seen drawing aside (recheck).
    checked: std::time::Instant,
    /// Pans in a row that didn't land, on the gen 2 (check_pan).
    missed_pans: u8,
    /// When a capture was last left for tools (CAPTURE).
    captured: Option<std::time::Instant>,
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
const CAPTURE: &str = "/var/run/unfr0zen/capture.raw";
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
        let side: usize = size.split(',').next().and_then(|s| s.parse().ok()).ok_or("fb0: virtual_size")?;
        if side != SIDE && side != SMALL {
            return Err(format!("fb0: a {side} pixel display"));
        }
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
        Ok(Display { file, map: map.cast(), len, stride, side, turned: side == SIDE, flip: None,
                     shown: vec![0; side * side * 4], checked: std::time::Instant::now(), missed_pans: 0, captured: None })
    }

    /// The panel's side in pixels.
    pub fn side(&self) -> usize {
        self.side
    }

    /// Writes the frame's rows `first` to `last` into a page: as they are,
    /// or on the gen 2 the panel's columns they make, turned back a quarter.
    fn put(page: &mut [u8], stride: usize, side: usize, turned: bool, frame: &[u8], first: usize, last: usize) {
        let row = side * 4;
        let last = last.min(side - 1);
        if turned {
            for y in first..=last {
                page[y * stride..y * stride + row].copy_from_slice(&frame[y * row..(y + 1) * row]);
            }
            return;
        }
        fledge_pixels::turn::back(page, stride, side, frame, first, last);
    }

    /// A page as a Frame, from a copy read in whole rows (its memory is slow
    /// a pixel at a time): on the gen 2 turned a quarter, as put() undoes.
    fn take(page: &[u8], stride: usize, side: usize, turned: bool) -> Frame {
        let row = side * 4;
        let mut panel = Vec::with_capacity(side * row);
        for y in 0..side {
            panel.extend_from_slice(&page[y * stride..y * stride + row]);
        }
        if turned {
            return panel;
        }
        let mut frame = Vec::with_capacity(side * row);
        for x in 0..side {
            for c in 0..side {
                let at = (side - 1 - c) * row + x * 4;
                frame.extend_from_slice(&panel[at..at + 4]);
            }
        }
        frame
    }

    /// The panning offset's row: the page shown.
    fn pan(&self) -> usize {
        sys("pan").ok().and_then(|s| s.split(',').nth(1)?.parse().ok()).unwrap_or(0)
    }

    /// The first byte of the visible page.
    fn page(&self) -> usize {
        match &self.flip {
            Some(flip) => flip.pages[flip.front] * self.stride,
            None => (self.pan() * self.stride).min(self.len.saturating_sub(self.stride * self.side)),
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

    /// Maps fb0 again if it has grown to two pages since it was mapped: the
    /// gen 2's nlclient makes its second page as it sets up its display,
    /// which can come after this process opened it. Mapped at one page, the
    /// UI could neither flip nor reach the page nlclient left shown.
    fn grow(&mut self) {
        if self.len >= 2 * self.side * self.stride {
            return;
        }
        let Some(rows) = sys("virtual_size").ok().and_then(|s| s.split(',').nth(1)?.parse::<usize>().ok()) else {
            return;
        };
        let Some(len) = grown(self.len, self.stride, rows) else { return };
        // SAFETY: a shared mapping of the framebuffer device, len its size now.
        let map = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED,
                       self.file.as_raw_fd(), 0)
        };
        if map == libc::MAP_FAILED {
            crate::logf!("ui: /dev/fb0: mmap of {rows} rows: {}", std::io::Error::last_os_error());
            return;
        }
        // SAFETY: unmaps what was mapped before; nothing borrows it past a call.
        unsafe { libc::munmap(self.map.cast(), self.len) };
        crate::logf!("ui: fb0 grew to {rows} rows: mapped again");
        (self.map, self.len) = (map.cast(), len);
    }

    /// The UI takes the display: from now on frames are drawn unseen and
    /// flipped to. Without two pages it draws straight on the shown one.
    pub fn begin(&mut self) {
        self.grow();
        let rows = self.len / self.stride;
        if rows < 2 * self.side {
            return;
        }
        // The gen 2's nlclient goes on drawing while the UI owns the display
        // (its present is skipped, not its drawing): into private memory once
        // its patch has mapped some over its pages (nlclient_private), and
        // until then into the page it doesn't show, so the UI keeps to the
        // one shown, drawing straight on it. Asked to draw, nlclient presents,
        // which is when its patch goes private.
        let home = usize::from(self.pan() >= rows / 2);
        let pages = if self.turned || nlclient_private() { [0, rows / 2] } else { [self.pan(); 2] };
        let home = if pages[0] == pages[1] { 0 } else { home };
        self.flip = Some(Flip { pages, front: home, home, stale: [Some((0, self.side - 1)); 2] });
        if !self.turned && pages[0] == pages[1] {
            wake_display();
        }
        self.checked = std::time::Instant::now();
        self.missed_pans = 0;
    }

    /// On the gen 2: both pages to flip between once nlclient draws into
    /// private memory, or back to the one shown if it no longer does (a new
    /// nlclient). Checked a few times a second.
    fn recheck(&mut self) {
        if self.turned || self.checked.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.checked = std::time::Instant::now();
        let rows = self.len / self.stride;
        let private = nlclient_private();
        let pan = self.pan();
        let Some(flip) = self.flip.as_mut() else { return };
        let single = flip.pages[0] == flip.pages[1];
        if private && single {
            let home = usize::from(pan >= rows / 2);
            *flip = Flip { pages: [0, rows / 2], front: home, home, stale: [Some((0, self.side - 1)); 2] };
            crate::logf!("ui: nlclient draws aside: flipping between both pages");
        } else if !private && !single {
            let home = flip.pages[flip.home];
            *flip = Flip { pages: [home; 2], front: 0, home: 0, stale: [Some((0, self.side - 1)); 2] };
            crate::logf!("ui: nlclient draws on fb0 again: keeping to one page");
        }
    }

    /// Gives the display back showing `frame` (nlclient's) on nlclient's
    /// own page, as its EGL left it.
    pub fn end(&mut self, frame: &[u8]) {
        let Some(flip) = self.flip.take() else { return self.show(frame) };
        let start = flip.pages[flip.home] * self.stride;
        let (stride, side, turned) = (self.stride, self.side, self.turned);
        Self::put(&mut self.bytes()[start..], stride, side, turned, frame, 0, side - 1);
        if let Err(e) = self.pan_to(flip.pages[flip.home]) {
            crate::logf!("ui: pan: {e}");
        }
    }

    /// Writes rows into the page to show next and flips to it: `rows`
    /// those changed (None, all), each given by `each`.
    fn present(&mut self, rows: Option<(usize, usize)>, mut each: impl FnMut(usize, &mut [u8])) {
        self.recheck();
        let (stride, side, turned) = (self.stride, self.side, self.turned);
        let row = side * 4;
        let Some(flip) = self.flip.as_mut() else {
            let start = self.page();
            let (first, last) = rows.unwrap_or((0, side - 1));
            if !turned {
                for y in first..=last.min(side - 1) {
                    each(y, &mut self.shown[y * row..(y + 1) * row]);
                }
                // SAFETY: map is len bytes, mapped for this Display's life; shown is a separate field.
                let page = unsafe { std::slice::from_raw_parts_mut(self.map.add(start), self.len - start) };
                return Self::put(page, stride, side, turned, &self.shown, first, last);
            }
            let page = &mut self.bytes()[start..];
            for y in first..=last.min(side - 1) {
                each(y, &mut page[y * stride..y * stride + row]);
            }
            return;
        };
        let back = 1 - flip.front;
        let all = Some((0, side - 1));
        let changed = if rows.is_none() { all } else { rows };
        let (first, last) = union(changed, flip.stale[back]).unwrap_or((0, side - 1));
        flip.stale[back] = None;
        flip.stale[flip.front] = union(flip.stale[flip.front], changed);
        flip.front = back;
        let y = flip.pages[back];
        let start = y * stride;
        // SAFETY: map is len bytes, mapped for this Display's life; shown is a separate field.
        let page = unsafe { std::slice::from_raw_parts_mut(self.map.add(start), self.len - start) };
        let shown = &mut self.shown;
        prof::time(prof::Part::Copy, || {
            for r in first..=last.min(side - 1) {
                let line = &mut shown[r * row..(r + 1) * row];
                each(r, line);
                if turned {
                    page[r * stride..r * stride + row].copy_from_slice(line);
                }
            }
            if !turned {
                Self::put(page, stride, side, turned, shown, first, last);
            }
        });
        // For tools (utils/nest-screen.py): the frame about to show, whole,
        // where reading the framebuffer could catch the next one being drawn.
        if std::fs::remove_file(CAPTURE_REQUEST).is_ok() {
            let frame = if turned {
                (0..side).flat_map(|r| page[r * stride..r * stride + row].to_vec()).collect()
            } else {
                shown.clone()
            };
            let _ = crate::settings_ui::publish("capture.raw", &frame);
            self.captured = Some(std::time::Instant::now());
        }
        // On the gen 2 /var/run is memory it can't spare (400 KB): a
        // capture is for a tool to read at once, and gone half a minute on.
        if !turned && self.captured.is_some_and(|at| at.elapsed() > Duration::from_secs(30)) {
            self.captured = None;
            let _ = std::fs::remove_file(CAPTURE);
        }
        if let Err(e) = prof::time(prof::Part::Pan, || self.pan_to(y)) {
            crate::logf!("ui: pan: {e}");
        }
        if !turned {
            self.check_pan(y);
        }
    }

    /// On the gen 2: whether the pan to `y` landed. Twice in a row not, the
    /// UI draws on the page that does show (and says so), rather than go on
    /// drawing where nothing is seen: a blank display under a live ring.
    fn check_pan(&mut self, y: usize) {
        let shown = self.pan();
        let give_up;
        (self.missed_pans, give_up) = missed_pan(self.missed_pans, shown, y);
        if !give_up {
            return;
        }
        crate::logf!("ui: panned to {y} but {shown} shows, twice: drawing on {shown}");
        if let Some(flip) = self.flip.as_mut() {
            *flip = Flip { pages: [shown; 2], front: 0, home: 0, stale: [Some((0, self.side - 1)); 2] };
        }
        // Not flipping between both again until nlclient is seen drawing
        // aside afresh (recheck), a little later.
        self.checked = std::time::Instant::now() + Duration::from_secs(5);
    }

    fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: map is len bytes, mapped for this Display's life.
        unsafe { std::slice::from_raw_parts_mut(self.map, self.len) }
    }

    /// The visible page as a Frame: what nlclient shows.
    pub fn save(&mut self) -> Frame {
        self.grow();
        let (start, stride, side, turned) = (self.page(), self.stride, self.side, self.turned);
        let page = &self.bytes()[start..];
        prof::time(prof::Part::Read, || Self::take(page, stride, side, turned))
    }

    /// What shows: the UI's own last frame while it has the display (and
    /// it wasn't panned away), else the display read back.
    pub fn current(&mut self) -> Frame {
        if self.flip.is_some() && !self.lost() {
            return self.shown.clone();
        }
        self.save()
    }

    /// Shows a Frame.
    pub fn show(&mut self, frame: &[u8]) {
        let row = self.side * 4;
        self.present(None, |y, line| line.copy_from_slice(&frame[y * row..(y + 1) * row]));
    }

    /// Shows a Frame of which only rows `first` to `last` changed.
    pub fn show_rows(&mut self, frame: &[u8], first: usize, last: usize) {
        let row = self.side * 4;
        self.present(Some((first, last)), |y, line| line.copy_from_slice(&frame[y * row..(y + 1) * row]));
    }

    /// `from` sliding out and `to` in by `by` pixels, sideways as the
    /// display stands: the framebuffer's rows, as it is mounted turned.
    pub fn slide(&mut self, from: &[u8], to: &[u8], by: usize, clockwise: bool) {
        let (side, row) = (self.side, self.side * 4);
        let by = by.min(side);
        self.present(None, |y, line| line.copy_from_slice(slid(from, to, by, clockwise, side, row, y)));
    }
}

/// Pans in a row that didn't land, after one to `y` with `shown` showing,
/// and whether that is the second (counting starts again).
fn missed_pan(missed: u8, shown: usize, y: usize) -> (u8, bool) {
    match (shown == y, missed + 1) {
        (true, _) => (0, false),
        (false, 2..) => (0, true),
        (false, n) => (n, false),
    }
}

/// The length to map fb0 at again, when its `rows` of `stride` bytes are
/// more than the `len` mapped (Display::grow).
fn grown(len: usize, stride: usize, rows: usize) -> Option<usize> {
    let now = rows.checked_mul(stride)?;
    (now > len).then_some(now)
}

/// Frame row `y` of `from` slid `by` rows towards `to` (Display::slide).
fn slid<'a>(from: &'a [u8], to: &'a [u8], by: usize, clockwise: bool, side: usize, row: usize, y: usize) -> &'a [u8] {
    let (source, at) = if clockwise {
        if y + by < side { (from, y + by) } else { (to, y + by - side) }
    } else if y >= by { (from, y - by) } else { (to, y + side - by) };
    &source[at * row..(at + 1) * row]
}

/// `from` slid `by` rows towards `to`, into `out`, as Display::slide shows it.
pub fn slide_into(out: &mut [u8], from: &[u8], to: &[u8], by: usize, clockwise: bool) {
    let side = super::paint::side() as usize;
    let row = side * 4;
    let by = by.min(side);
    for (y, line) in out.chunks_exact_mut(row).enumerate().take(side) {
        line.copy_from_slice(slid(from, to, by, clockwise, side, row, y));
    }
}

impl Display {
    /// `to` over `from` at `alpha` of 256.
    pub fn fade(&mut self, from: &[u8], to: &[u8], alpha: u16) {
        let row = self.side * 4;
        let mut blended = vec![0u8; row];
        self.present(None, |y, line| {
            // Into a line written to the framebuffer whole: its memory is
            // slow a byte at a time.
            fledge_pixels::blend::mix(&mut blended, &from[y * row..(y + 1) * row], &to[y * row..(y + 1) * row], alpha);
            line.copy_from_slice(&blended);
        });
    }

}

/// The gen 3's display side; the gen 2's.
const SIDE: usize = 480;
const SMALL: usize = 320;

/// A screen as the framebuffer holds it: the panel's side (paint::side) in
/// rows of as many BGRX pixels, the picture turned a quarter anticlockwise,
/// as the gen 3's panel shows the framebuffer turned a quarter clockwise.
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
    controls: &'static Controls,
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
        let mut missing = Vec::new();
        for controls in &CONTROLS {
            match (node(controls.ring), node(controls.button)) {
                (Ok(ring), Ok(button)) => return Ok(Input { ring, button, grabbed: false, controls }),
                (Err(e), _) | (_, Err(e)) => missing.push(e),
            }
        }
        Err(missing.join(", "))
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
                    let c = self.controls;
                    match (kind, code) {
                        (EV_REL, code) if code == c.ring_code => {
                            events.push(Event { kind, code: REL_DIAL, value: value * c.ring_sign });
                        }
                        (EV_KEY, code) if code == c.button_code => events.push(Event { kind, code: KEY_ENTER, value }),
                        (EV_REL | EV_KEY, _) => events.push(Event { kind, code, value }),
                        _ => {}
                    }
                }
            }
        }
        events
    }

    /// A turn of the ring for nlclient, as a hand makes one: `reports`
    /// reports of `raw` counts, 25 ms apart (its menus take no other).
    pub fn hand_turn(&mut self, raw: i32, reports: usize) {
        let c = self.controls;
        let report: Vec<u8> = [encode(EV_REL, c.ring_code, raw * c.ring_sign), encode(0, 0, 0)].concat();
        for _ in 0..reports {
            self.send(true, &report);
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// A press and release of the button, for nlclient.
    pub fn press(&mut self) {
        let key = self.controls.button_code;
        let down: Vec<u8> = [encode(EV_KEY, key, 1), encode(0, 0, 0)].concat();
        let up: Vec<u8> = [encode(EV_KEY, key, 0), encode(0, 0, 0)].concat();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gen_2_page_is_the_frame_turned_back_a_quarter() {
        let (side, row) = (SMALL, SMALL * 4);
        let mut frame = vec![0u8; side * row];
        // Frame row 10 (upright column 10), column 20 (upright y 299).
        frame[10 * row + 20 * 4] = 9;
        let mut page = vec![0u8; side * row];
        Display::put(&mut page, row, side, false, &frame, 0, side - 1);
        // Shown upright at x 10, y 299: the panel's row 299, column 10.
        assert_eq!(page[299 * row + 10 * 4], 9);
        assert_eq!(Display::take(&page, row, side, false), frame, "read back as it was");
    }

    #[test]
    fn a_gen_2_page_turned_in_bands_is_every_pixel_turned() {
        let (side, row) = (SMALL, SMALL * 4);
        let frame: Vec<u8> = (0..side * row).map(|i| (i * 7 % 251) as u8).collect();
        for (first, last) in [(0, side - 1), (37, 121), (300, 319), (5, 5)] {
            let mut page = vec![0u8; side * row];
            let mut want = page.clone();
            Display::put(&mut page, row, side, false, &frame, first, last);
            for y in 0..side {
                for x in first..=last {
                    let at = x * row + (side - 1 - y) * 4;
                    want[y * row + x * 4..y * row + x * 4 + 4].copy_from_slice(&frame[at..at + 4]);
                }
            }
            assert!(page == want, "rows {first}..={last}");
        }
    }

    #[test]
    fn two_pans_in_a_row_that_dont_land_give_up_flipping() {
        assert_eq!(missed_pan(0, 320, 320), (0, false), "landed");
        assert_eq!(missed_pan(0, 320, 0), (1, false), "once may be a page still turning");
        assert_eq!(missed_pan(1, 320, 0), (0, true), "twice: draw where it shows");
        assert_eq!(missed_pan(1, 0, 0), (0, false), "a landing in between starts again");
    }

    #[test]
    fn a_framebuffer_grown_after_opening_is_mapped_again() {
        let stride = 320 * 4;
        assert_eq!(grown(320 * stride, stride, 640), Some(640 * stride), "nlclient made its second page");
        assert_eq!(grown(640 * stride, stride, 640), None, "both pages mapped already");
        assert_eq!(grown(320 * stride, stride, 320), None, "still one page");
    }

    #[test]
    fn a_gen_3_page_is_the_frame() {
        let (side, row) = (SIDE, SIDE * 4);
        let mut frame = vec![0u8; side * row];
        frame[10 * row + 20 * 4] = 9;
        let mut page = vec![0u8; side * row];
        Display::put(&mut page, row, side, true, &frame, 0, side - 1);
        assert_eq!(page, frame);
        assert_eq!(Display::take(&page, row, side, true), frame);
    }
}

#[cfg(test)]
mod owner_tests {
    use super::foreign_owner;

    #[test]
    fn only_an_owner_without_a_heartbeat_that_is_not_us_is_another_s() {
        assert_eq!(foreign_owner("4321\n"), Some(4321), "Doom's launcher");
        assert_eq!(foreign_owner("4321 17"), None, "an earlier nestlocal's, with its heartbeat");
        assert_eq!(foreign_owner(&std::process::id().to_string()), None, "ours");
        assert_eq!(foreign_owner(""), None);
        assert_eq!(foreign_owner("1"), None);
        assert_eq!(foreign_owner("x"), None);
    }
}
