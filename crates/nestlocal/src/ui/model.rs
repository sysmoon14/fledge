//! Home Assistant's cards, as the Fledge integration keeps them retained on
//! <base>/set/ui, pictures (a media player's album art as PNG, a camera's
//! still as JPEG) on <base>/set/art/<key>,
//! and the commands the cards send back on <base>/ui/command.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Cards {
    #[serde(default)]
    pub cards: Vec<Card>,
    /// What the thermostat wakes to: "chooser" (the default), "thermostat",
    /// or a card's place in `cards`, opened.
    #[serde(default)]
    wake: Option<serde_json::Value>,
    /// Where the thermostat goes among the cards: before `cards[n]` (0,
    /// the default, first; past the end, last before settings).
    #[serde(default)]
    thermostat: Option<usize>,
    /// Whether the thermostat has a card at all (protocol 10); with false,
    /// Home Assistant controls the heating and the Nest shows only its cards.
    #[serde(default)]
    show_thermostat: Option<bool>,
    /// Shown as the display lights by itself (someone near), if Home
    /// Assistant asks for it: the time, the weather, what's on.
    #[serde(default)]
    pub glance: Option<Glance>,
    /// What someone across the room and someone coming up to the
    /// thermostat do (protocol 6); without it, as before.
    #[serde(default)]
    presence: Option<Presence>,
    /// Seconds the screen stays untouched before it times out (protocol 6).
    #[serde(default)]
    card_idle: Option<u64>,
    /// What this thermostat is called in Home Assistant, for its own card
    /// (protocol 7); "Thermostat" without.
    #[serde(default)]
    thermostat_name: Option<String>,
    /// Whether turning a dimmer down past its lowest turns the light off,
    /// and a switch's ring turns it off and on (protocol 8); on without.
    #[serde(default)]
    dim_off: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct Presence {
    #[serde(default)]
    far: Option<Far>,
    #[serde(default)]
    near: Option<Near>,
}

/// Someone across the room (the display lit by Farsight).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Far {
    /// The glance.
    Glance,
    /// What waking shows, as a turn would.
    Wake,
    /// Nothing: the display stays black until someone touches it.
    Dark,
}

/// Someone coming up to the thermostat (its approach reading).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Near {
    /// What waking shows, lit if it isn't.
    Wake,
    /// The glance, lit if it isn't.
    Glance,
    /// Nothing more than across the room.
    None,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Glance {
    /// Home Assistant's time zone, seconds east of UTC.
    #[serde(default)]
    pub offset: i64,
    #[serde(default)]
    pub weather: Option<Weather>,
    /// How it's laid out (protocol 11); the clock without.
    #[serde(default)]
    pub layout: Layout,
    /// The clock's hours (protocol 11): 24 without.
    #[serde(default)]
    pub clock: Clock,
    /// The lines under the big element, top to bottom (protocol 11);
    /// without, the date, the weather and what's on, as before.
    #[serde(default)]
    pub lines: Option<Vec<Line>>,
    /// The big number's (Layout::Number).
    #[serde(default)]
    pub big: Option<Big>,
    /// The next event (Layout::Agenda).
    #[serde(default)]
    pub event: Option<Event>,
    /// Whether the temperature layout colours the screen while heating or
    /// cooling (on without).
    #[serde(default)]
    pub colour: Option<bool>,
    /// The night's glance (Home Assistant decides when), drawn dimmer and
    /// warmer with `dim`.
    #[serde(default)]
    pub night: bool,
    #[serde(default)]
    pub dim: bool,
    /// How bright the dimmed glance stays, 0.1 to 1 (protocol 12).
    #[serde(default)]
    pub dim_level: Option<f32>,
    /// The date's words in Home Assistant's language (protocol 12);
    /// English without, or where they aren't all there.
    #[serde(default)]
    pub words: Option<Words>,
    /// The date's order: "mdy" for November 8, else 8 November.
    #[serde(default)]
    pub date_order: Option<String>,
}

/// The names of the days (Sunday first) and months, long and short.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Words {
    #[serde(default)]
    pub days: Vec<String>,
    #[serde(default)]
    pub days_short: Vec<String>,
    #[serde(default)]
    pub months: Vec<String>,
    #[serde(default)]
    pub months_short: Vec<String>,
}

impl Words {
    /// Whether every name is there.
    fn whole(&self) -> bool {
        let named = |names: &[String], n: usize| names.len() == n && names.iter().all(|name| !name.trim().is_empty());
        named(&self.days, 7) && named(&self.days_short, 7) && named(&self.months, 12) && named(&self.months_short, 12)
    }
}

/// The glance's layouts: something large, and lines under it.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// The time large (the glance as it was).
    #[default]
    Clock,
    /// The room's temperature large.
    Temp,
    /// Outside: the weather's icon and temperature.
    Weather,
    /// A clock face with hands.
    Analog,
    /// One value from Home Assistant.
    Number,
    /// The next event from a calendar.
    Agenda,
    /// A layout this build doesn't know: the clock.
    #[serde(other)]
    Unknown,
}

impl Layout {
    /// The lines it has room for.
    pub fn slots(self) -> usize {
        match self {
            Layout::Clock | Layout::Unknown => 3,
            Layout::Temp | Layout::Weather | Layout::Number => 2,
            Layout::Analog | Layout::Agenda => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct Clock {
    /// 12 or 24.
    #[serde(default = "twenty_four")]
    pub hours: u8,
    /// AM / PM beside a 12-hour time.
    #[serde(default = "yes")]
    pub ampm: bool,
}

impl Default for Clock {
    fn default() -> Clock {
        Clock { hours: 24, ampm: true }
    }
}

fn twenty_four() -> u8 {
    24
}

fn yes() -> bool {
    true
}

impl Clock {
    pub fn twelve(&self) -> bool {
        self.hours == 12
    }

    /// "14:05", or "2:05" with " PM" where `ampm` asks and the clock shows it.
    pub fn time(&self, minutes: i64, ampm: bool) -> String {
        let (h, m) = (minutes.rem_euclid(1440) / 60, minutes.rem_euclid(60));
        if !self.twelve() {
            return format!("{h:02}:{m:02}");
        }
        let twelve = (h + 11) % 12 + 1;
        if ampm && self.ampm { format!("{twelve}:{m:02} {}", Clock::half(h)) } else { format!("{twelve}:{m:02}") }
    }

    /// "AM" or "PM" for the hour `h` (0..24).
    pub fn half(h: i64) -> &'static str {
        if h < 12 { "AM" } else { "PM" }
    }
}

/// A line of the glance: one the Nest works out itself (`kind`), or
/// Home Assistant's words with an icon.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Line {
    /// date, date_short, time, inside, heating, whats_on, weather; or none
    /// for Home Assistant's `text`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
}

impl Line {
    pub fn kind(kind: &str) -> Line {
        Line { kind: Some(kind.to_owned()), ..Line::default() }
    }
}

/// The big number: what it is, its value and unit as Home Assistant shows
/// them.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Big {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub unit: String,
}

/// A calendar's next event.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Event {
    #[serde(default)]
    pub title: String,
    /// When it starts (Unix seconds).
    #[serde(default)]
    pub start: i64,
    #[serde(default)]
    pub all_day: bool,
}

impl Glance {
    /// The date's words, if Home Assistant sent them all.
    pub fn words(&self) -> Option<&Words> {
        self.words.as_ref().filter(|words| words.whole())
    }

    /// Whether the month comes before the day's number.
    pub fn month_first(&self) -> bool {
        self.date_order.as_deref() == Some("mdy")
    }

    /// Its lines, as many as its layout has room for: the date, the
    /// weather and what's on where Home Assistant sent none.
    pub fn lines(&self) -> Vec<Line> {
        let mut lines = match &self.lines {
            Some(lines) => lines.clone(),
            None => ["date", "weather", "whats_on"].map(Line::kind).to_vec(),
        };
        lines.truncate(self.layout.slots());
        lines
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Weather {
    /// cloudy, sunny, rainy, ... (Home Assistant's conditions).
    #[serde(default)]
    pub condition: String,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub unit: String,
}

/// What waking shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wake {
    /// The cards, zoomed out on the thermostat.
    Chooser,
    /// The thermostat, its ring setting the temperature: the Nest as it was.
    Thermostat,
    /// A card, opened.
    Card(usize),
}

impl Cards {
    /// Whether turning down past a dimmer's lowest turns the light off.
    pub fn dim_off(&self) -> bool {
        self.dim_off.unwrap_or(true)
    }

    /// Whether the thermostat has a card among the others.
    pub fn show_thermostat(&self) -> bool {
        self.show_thermostat.unwrap_or(true)
    }

    /// The thermostat's place among the cards.
    pub fn thermostat(&self) -> usize {
        self.thermostat.unwrap_or(0)
    }

    /// Whether Home Assistant said what someone across the room or close
    /// brings (protocol 6 on).
    pub fn has_presence(&self) -> bool {
        self.presence.is_some()
    }

    /// What someone across the room brings: the glance if Home Assistant
    /// sent one, else waking, unless it says.
    pub fn far(&self) -> Far {
        match self.presence.as_ref().and_then(|p| p.far) {
            Some(Far::Glance) | None if self.glance.is_some() => Far::Glance,
            Some(Far::Dark) => Far::Dark,
            _ => Far::Wake,
        }
    }

    /// What someone coming up to it brings: nothing more, unless it says.
    pub fn near(&self) -> Near {
        match self.presence.as_ref().and_then(|p| p.near) {
            Some(Near::Glance) if self.glance.is_none() => Near::Wake,
            Some(near) => near,
            None => Near::None,
        }
    }

    /// This thermostat's name in Home Assistant, if it sent one.
    pub fn thermostat_name(&self) -> Option<&str> {
        self.thermostat_name.as_deref().map(str::trim).filter(|n| !n.is_empty())
    }

    /// How long the screen stays untouched, if Home Assistant says.
    pub fn card_idle(&self) -> Option<std::time::Duration> {
        self.card_idle.map(|s| std::time::Duration::from_secs(s.clamp(5, 600)))
    }

    pub fn wake(&self) -> Wake {
        match &self.wake {
            Some(Value::String(s)) if s == "thermostat" => Wake::Thermostat,
            Some(Value::Number(n)) => match n.as_u64() {
                Some(i) if (i as usize) < self.cards.len() => Wake::Card(i as usize),
                _ => Wake::Chooser,
            },
            _ => Wake::Chooser,
        }
    }
}

// A climate card's lists make it the largest; a deck holds a handful.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Card {
    Lights(LightsCard),
    Climate(ClimateCard),
    Media(MediaCard),
    /// A scene, script or button: pressed, it runs.
    Action(ActionCard),
    /// A camera's latest still: pressed, a new one.
    Camera(CameraCard),
    /// A choice of options (a select: hot water boost): pressed, the list.
    Options(OptionsCard),
    /// Blinds, curtains, a garage door: the ring sets how open.
    Cover(CoverCard),
    /// An alarm panel: armed from its modes, disarmed with its code.
    Alarm(AlarmCard),
    /// A glance face (protocol 13): something to look at, drawn as the
    /// glance is, with actions round its ring.
    Face(FaceCard),
    /// A kind from a newer Fledge: left out rather than failing the rest.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ActionCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
}

/// A glance face as a card: its own layout and lines (Home Assistant sends
/// them built, as it does the glance), and up to FACE_ACTIONS actions,
/// listed after Back when it's pressed open; `hold`, one of them, is what
/// a long press does while it's open.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct FaceCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub glance: Glance,
    #[serde(default)]
    pub actions: Vec<FaceAction>,
    #[serde(default)]
    hold: Option<usize>,
}

/// The most actions a face has round its ring, after Back.
pub const FACE_ACTIONS: usize = 4;

impl FaceCard {
    /// What a hold does: one of its actions, by place.
    pub fn hold(&self) -> Option<usize> {
        self.hold.filter(|&i| i < self.actions.len())
    }
}

/// One of a face's actions: a scene, script or button run, or a light or
/// switch turned on and off.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct FaceAction {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    /// "run", or "switch".
    #[serde(default)]
    pub kind: String,
    /// A switch's: whether it's on now.
    #[serde(default)]
    pub on: Option<bool>,
}

impl FaceAction {
    pub fn switched(&self) -> bool {
        self.kind == "switch"
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct OptionsCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub current: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CoverCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    /// open, closed, opening, closing.
    #[serde(default)]
    pub state: String,
    /// Percent open, if it knows.
    #[serde(default)]
    pub position: Option<u8>,
    /// Whether it can be set part open (else only opened and closed).
    #[serde(default)]
    pub positionable: bool,
}

/// An alarm panel (protocol 9), as any Home Assistant alarm gives it; Alarmo
/// adds its delay's length, the sensors that set it off, and arming anyway.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AlarmCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    /// disarmed, arming, armed_home, armed_away, armed_night,
    /// armed_vacation, armed_custom_bypass, pending, disarming, triggered.
    #[serde(default)]
    pub state: String,
    /// The modes it arms to, in the ring's order, each with its name.
    #[serde(default)]
    pub modes: Vec<AlarmMode>,
    /// The code it wants for what it would do next ("number", "text"), as
    /// its code_format says in this state; None: none.
    #[serde(default)]
    pub code: Option<String>,
    /// How many digits the code has, if Fledge was told: the last sends it.
    #[serde(default)]
    pub code_length: Option<u8>,
    /// The exit or entry delay running, if the panel says how long.
    #[serde(default)]
    pub delay: Option<AlarmDelay>,
    /// What set it off, or stops it arming, by name.
    #[serde(default)]
    pub sensors: Vec<String>,
    /// Whether it can be armed with a door open (Alarmo's force).
    #[serde(default)]
    pub can_force: bool,
    /// Whether it takes the screen when the entry delay starts or it goes
    /// off (the default), or waits on its card.
    #[serde(default = "takes_over")]
    pub takeover: bool,
}

fn takes_over() -> bool { true }

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AlarmMode {
    /// home, away, night, vacation, custom_bypass.
    pub mode: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct AlarmDelay {
    /// When it ends (Unix seconds), and how long it is.
    pub ends: f64,
    pub seconds: f64,
}

impl AlarmCard {
    /// Whether the code it wants now is digits the ring can enter.
    pub fn wants_digits(&self) -> bool {
        self.code.as_deref() == Some("number")
    }

    /// Whether it wants a code the ring can't enter (letters).
    pub fn wants_text(&self) -> bool {
        self.code.as_ref().is_some_and(|c| c != "number")
    }

    /// The mode it's armed in, if it is.
    pub fn armed(&self) -> Option<&str> {
        self.state.strip_prefix("armed_")
    }

    /// Whether it needs whoever is there: coming in, or going off.
    pub fn urgent(&self) -> bool {
        matches!(self.state.as_str(), "pending" | "triggered")
    }

    /// A mode's name on the Nest: as Fledge was told, else Home Assistant's.
    pub fn mode_name(&self, mode: &str) -> String {
        let named = self.modes.iter().find(|m| m.mode == mode).map(|m| m.name.trim()).filter(|n| !n.is_empty());
        named.map(str::to_owned).unwrap_or_else(|| match mode {
            "home" => "Home".into(),
            "away" => "Away".into(),
            "night" => "Night".into(),
            "vacation" => "Vacation".into(),
            "custom_bypass" => "Custom".into(),
            other => other.replace('_', " "),
        })
    }

    /// How much of its delay is left at `now` (Unix seconds), 0..1, and the
    /// whole seconds left; None if it doesn't say.
    pub fn left(&self, now: f64) -> Option<(f32, u64)> {
        let delay = self.delay.filter(|d| d.seconds > 0.0)?;
        let left = (delay.ends - now).max(0.0);
        Some(((left / delay.seconds).clamp(0.0, 1.0) as f32, left.ceil() as u64))
    }
}

/// What Home Assistant says of an alarm command (<base>/set/alarm_result).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct AlarmResult {
    /// The alarm's entity.
    pub alarm: String,
    #[serde(default)]
    pub ok: bool,
    /// rejected (the code, or the panel said no), locked (too many tries),
    /// open_sensors (a door or window open), failed.
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub sensors: Vec<String>,
    /// Tries left before the code is locked out, and for how long it is.
    #[serde(default)]
    pub tries: Option<u8>,
    #[serde(default)]
    pub locked_for: Option<u64>,
}

impl AlarmResult {
    pub fn parse(payload: &[u8]) -> Result<AlarmResult, String> {
        let result: AlarmResult = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
        if result.alarm.is_empty() {
            return Err("an alarm result needs its alarm".into());
        }
        Ok(result)
    }
}

/// An alarm code, as entered: its digits never printed (logs, ui.mode).
#[derive(Clone, Default, PartialEq)]
pub struct Code(pub String);

impl std::fmt::Debug for Code {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Code({} digits)", self.0.len())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct CameraCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    /// Its latest still, from <base>/set/art/<entity id>.
    #[serde(skip)]
    pub art: Option<Arc<Art>>,
}

/// A notification from Home Assistant (fledge.notify, on <base>/set/alert):
/// over whatever shows, until pressed, dismissed or `timeout` seconds pass.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Alert {
    /// Its tag, in what the thermostat says back of it.
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default = "alert_timeout")]
    pub timeout: f64,
    /// The key of its picture among the art (a camera's still), if any.
    #[serde(default)]
    pub picture: Option<String>,
    /// Home Assistant took it back (fledge.dismiss).
    #[serde(default)]
    pub dismiss: bool,
    #[serde(skip)]
    pub art: Option<Arc<Art>>,
}

fn alert_timeout() -> f64 { 30.0 }

impl Alert {
    /// <base>/set/alert's payload.
    pub fn parse(payload: &[u8]) -> Result<Alert, String> {
        let alert: Alert = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
        if alert.id.is_empty() {
            return Err("an alert needs its id".into());
        }
        Ok(alert)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct LightsCard {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub lights: Vec<Light>,
    /// A light group's own lights, each switched from its menu.
    #[serde(default)]
    pub members: Vec<Light>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Light {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub on: bool,
    #[serde(default)]
    pub dimmable: bool,
    /// Percent; None while Home Assistant doesn't know (an off light).
    #[serde(default)]
    pub brightness: Option<u8>,
    #[serde(default)]
    pub rgb: Option<[u8; 3]>,
    /// Its white's warmth, in kelvin, and the range it takes: None, it has
    /// no warmth to set.
    #[serde(default)]
    pub kelvin: Option<u16>,
    #[serde(default)]
    pub kelvins: Option<[u16; 2]>,
    /// Whether it takes colours, and its hue (0..360) when it shows one.
    #[serde(default)]
    pub hues: bool,
    #[serde(default)]
    pub hue: Option<u16>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct ClimateCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    /// heat, cool, heat_cool, off, ...
    #[serde(default)]
    pub mode: String,
    /// heating, cooling, idle, off, ...
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub target: Option<f64>,
    #[serde(default)]
    pub current: Option<f64>,
    #[serde(default = "min_temp")]
    pub min: f64,
    #[serde(default = "max_temp")]
    pub max: f64,
    #[serde(default = "temp_step")]
    pub step: f64,
    /// Its presets (eco, away, ...) but none, and the one it's in. Home
    /// Assistant's whole list, none included, is `preset_modes` (Fledge
    /// 0.18.0 on); a list of its own off its menu.
    #[serde(default)]
    pub presets: Vec<String>,
    #[serde(default)]
    pub preset_modes: Vec<String>,
    #[serde(default)]
    pub preset: Option<String>,
    /// Its modes (heat, cool, dry, fan_only, off, ...) and fan speeds, each
    /// chosen from a list off its menu.
    #[serde(default)]
    pub modes: Vec<String>,
    #[serde(default)]
    pub fan_modes: Vec<String>,
    #[serde(default)]
    pub fan_mode: Option<String>,
    /// Its swing (up and down) and horizontal swing, as fan speeds are.
    #[serde(default)]
    pub swing_modes: Vec<String>,
    #[serde(default)]
    pub swing_mode: Option<String>,
    #[serde(default)]
    pub swing_horizontal_modes: Vec<String>,
    #[serde(default)]
    pub swing_horizontal_mode: Option<String>,
}

/// What a climate card's menu chooses from a list: its mode, fan speed,
/// preset, swing or horizontal swing, in the menu's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClimateChoice {
    Mode,
    Fan,
    Preset,
    Swing,
    SwingHorizontal,
    /// This thermostat's hot water boost, for how long (off, or minutes):
    /// its own, beyond what a climate has (not in ALL).
    HotWater,
}

impl ClimateChoice {
    pub const ALL: [ClimateChoice; 5] = [ClimateChoice::Mode, ClimateChoice::Fan, ClimateChoice::Preset,
                                         ClimateChoice::Swing, ClimateChoice::SwingHorizontal];

    /// Its command's action, as Fledge reads it.
    pub fn action(self) -> &'static str {
        match self {
            ClimateChoice::Mode => "hvac_mode",
            ClimateChoice::Fan => "fan_mode",
            ClimateChoice::Preset => "preset",
            ClimateChoice::Swing => "swing_mode",
            ClimateChoice::SwingHorizontal => "swing_horizontal_mode",
            ClimateChoice::HotWater => "hot_water_boost",
        }
    }

    /// Its name, as Home Assistant's more-info shows it.
    pub fn title(self) -> &'static str {
        match self {
            ClimateChoice::Mode => "Mode",
            ClimateChoice::Fan => "Fan",
            ClimateChoice::Preset => "Preset",
            ClimateChoice::Swing => "Swing",
            ClimateChoice::SwingHorizontal => "Horizontal swing",
            ClimateChoice::HotWater => "Hot water boost",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            ClimateChoice::Mode => "mdi:thermostat",
            ClimateChoice::Fan => "mdi:fan",
            ClimateChoice::Preset => "mdi:tune-variant",
            ClimateChoice::Swing => "mdi:arrow-oscillating",
            ClimateChoice::SwingHorizontal => "mdi:arrow-left-right",
            ClimateChoice::HotWater => "mdi:water-boiler",
        }
    }
}

impl ClimateCard {
    /// Its action as its mode allows: off while off, and idle if it says
    /// it's cooling in heat (heating in cool), as an integration can leave
    /// a stale action behind. Every view of it then glows alike.
    pub fn settle_action(&mut self) {
        let can = |modes: &[&str]| modes.contains(&self.mode.as_str());
        let fits = match self.action.as_str() {
            _ if self.mode == "off" => false,
            "heating" | "preheating" => can(&["heat", "heat_cool", "auto"]),
            "cooling" => can(&["cool", "heat_cool", "auto"]),
            _ => true,
        };
        if !fits {
            self.action = if self.mode == "off" { "off" } else { "idle" }.into();
        }
    }

    /// Whether its menu lists `choice`: modes to change between, else any.
    pub fn offers(&self, choice: ClimateChoice) -> bool {
        let (list, _) = self.choices(choice);
        if choice == ClimateChoice::Mode { list.len() > 1 } else { !list.is_empty() }
    }

    /// Its options for `choice`, and the one it's in. Presets are Home
    /// Assistant's whole list where Fledge sends it (none, to leave one),
    /// else the ones it has.
    pub fn choices(&self, choice: ClimateChoice) -> (&[String], Option<&str>) {
        match choice {
            ClimateChoice::Mode => (&self.modes, Some(self.mode.as_str())),
            ClimateChoice::Fan => (&self.fan_modes, self.fan_mode.as_deref()),
            ClimateChoice::Preset if !self.preset_modes.is_empty() => {
                (&self.preset_modes, Some(self.preset.as_deref().unwrap_or("none")))
            }
            ClimateChoice::Preset => (&self.presets, self.preset.as_deref()),
            ClimateChoice::Swing => (&self.swing_modes, self.swing_mode.as_deref()),
            ClimateChoice::SwingHorizontal => (&self.swing_horizontal_modes, self.swing_horizontal_mode.as_deref()),
            ClimateChoice::HotWater => (&[], None),
        }
    }

    /// The preset it's in, if one (not none): its dial says so.
    pub fn active_preset(&self) -> Option<&str> {
        self.preset.as_deref().filter(|p| !p.is_empty() && *p != "none")
    }

    /// `choice` set to `option`, as Home Assistant will soon say.
    pub fn set(&mut self, choice: ClimateChoice, option: &str) {
        let option = option.to_owned();
        match choice {
            ClimateChoice::Mode => self.mode = option,
            ClimateChoice::Fan => self.fan_mode = Some(option),
            ClimateChoice::Preset => self.preset = Some(option),
            ClimateChoice::Swing => self.swing_mode = Some(option),
            ClimateChoice::SwingHorizontal => self.swing_horizontal_mode = Some(option),
            ClimateChoice::HotWater => {}
        }
    }
}

/// This thermostat, from its own buckets (bridge::state, with its equipment
/// and software): its card, as a climate card is drawn, and its settings.
/// Temperatures are as it shows them (°C, or °F with `fahrenheit`).
#[derive(Clone, Debug, PartialEq)]
pub struct Nest {
    pub card: ClimateCard,
    pub eco: bool,
    /// When it expects to reach its target (Unix seconds).
    pub time_to_target: Option<f64>,
    pub humidity: Option<f64>,
    pub can_heat: bool,
    pub can_cool: bool,
    pub fahrenheit: bool,
    pub hot_water: bool,
    /// Minutes of hot water boost asked for, while one runs.
    pub hot_water_boost: Option<u32>,
    pub hot_water_schedule: bool,
    pub eco_low: Option<f64>,
    pub eco_high: Option<f64>,
    pub learning: bool,
    pub home_away: bool,
    pub early_on: bool,
    pub sunblock: bool,
    /// This build of nestlocal, and a newer release to install, if any.
    pub version: String,
    pub update: Option<String>,
    /// When its schedule next changes the target ("HH:MM", its own clock).
    pub next_change: Option<String>,
    /// Whether it acts as a thermostat; not, its heating is off and it has
    /// no card or settings of its own (role.rs).
    pub acting: bool,
}

impl Nest {
    /// From the state Home Assistant's entities read (bridge::state), plus
    /// can_heat, can_cool, has_hot_water_control, has_fan, version and update.
    pub fn from_state(s: &serde_json::Map<String, Value>) -> Nest {
        let number = |k: &str| s.get(k).and_then(Value::as_f64);
        let on = |k: &str| s.get(k).and_then(Value::as_bool).unwrap_or(false);
        let text = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let fahrenheit = text("temperature_scale") == "F";
        let (min, max, step) = if fahrenheit { (48.0, 90.0, 1.0) } else { (9.0, 32.0, 0.5) };
        let eco = text("preset") == "eco";
        let (can_heat, can_cool) = (on("can_heat"), on("can_cool"));
        // Its modes, presets and fan as its Home Assistant entity lists them
        // (bridge.rs), for the menu every thermostat card shares.
        let mut modes = vec!["off".to_owned()];
        modes.extend([("heat", can_heat), ("cool", can_cool), ("heat_cool", can_heat && can_cool)]
            .into_iter().filter(|(_, can)| *can).map(|(mode, _)| mode.to_owned()));
        // The bridge always says a fan mode (auto); only one with a fan has one.
        let fan_mode = s.get("fan_mode").and_then(Value::as_str).map(str::to_owned).filter(|_| on("has_fan"));
        let card = ClimateCard {
            id: "nest".into(),
            name: "Thermostat".into(),
            icon: String::new(),
            mode: text("mode"),
            action: text("action"),
            target: number("target"),
            current: number("current_temperature"),
            min, max, step,
            preset: Some(if eco { "eco" } else { "none" }.to_owned()),
            preset_modes: vec!["none".into(), "eco".into()],
            modes,
            fan_modes: if fan_mode.is_some() { vec!["auto".into(), "on".into()] } else { Vec::new() },
            fan_mode,
            ..ClimateCard::default()
        };
        let mut card = card;
        card.settle_action();
        let boost = s.get("hot_water_boost").and_then(|v| match v {
            Value::String(m) => m.parse().ok(),
            Value::Number(m) => m.as_u64().map(|m| m as u32),
            _ => None,
        });
        let time_to_target = s.get("time_to_target").and_then(Value::as_str)
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.timestamp() as f64);
        Nest {
            card, eco, time_to_target,
            humidity: number("current_humidity"),
            can_heat,
            can_cool,
            fahrenheit,
            hot_water: on("has_hot_water_control"),
            hot_water_boost: boost,
            hot_water_schedule: on("hot_water_schedule"),
            eco_low: number("away_temperature_low"),
            eco_high: number("away_temperature_high"),
            learning: on("learning_mode"),
            home_away: on("auto_away_enable"),
            early_on: on("preconditioning_enabled"),
            sunblock: on("sunlight_correction_enabled"),
            version: text("version"),
            update: s.get("update").and_then(Value::as_str).map(str::to_owned),
            next_change: s.get("next_change").and_then(Value::as_str).map(str::to_owned),
            acting: s.get("acting").and_then(Value::as_bool).unwrap_or(true),
        }
    }

    /// A Home Assistant climate's card drawn as this thermostat's is: its
    /// own, without what only the Nest knows (its schedule, when it'll get
    /// there, hot water, its own Eco).
    pub fn of_card(card: &ClimateCard) -> Nest {
        let can = |mode: &str| card.modes.iter().any(|m| m == mode || m == "heat_cool");
        Nest {
            card: card.clone(), eco: false, time_to_target: None, humidity: None,
            can_heat: can("heat"), can_cool: can("cool"), fahrenheit: false,
            hot_water: false, hot_water_boost: None, hot_water_schedule: false, eco_low: None, eco_high: None,
            learning: false, home_away: false, early_on: false, sunblock: false,
            version: String::new(), update: None, next_change: None, acting: true,
        }
    }

    /// A temperature as shown, in °C for its commands.
    pub fn celsius(&self, shown: f64) -> f64 {
        if self.fahrenheit { ((shown - 32.0) / 1.8 * 2.0).round() / 2.0 } else { shown }
    }

    /// Its heating mode's name for the mode command: heat, else cool.
    pub fn on_mode(&self) -> &str {
        if self.can_heat || !self.can_cool {
            // One that can neither (a fan, a dehumidifier): its first mode.
            if !self.can_heat {
                if let Some(mode) = self.card.modes.iter().find(|m| *m != "off") {
                    return mode;
                }
            }
            "heat"
        } else {
            "cool"
        }
    }
}

fn min_temp() -> f64 { 7.0 }
fn max_temp() -> f64 { 30.0 }
fn temp_step() -> f64 { 0.5 }

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct MediaCard {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: String,
    /// playing, paused, idle, off, ...
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    /// Percent.
    #[serde(default)]
    pub volume: Option<u8>,
    /// Its album art, from its own topic.
    #[serde(skip)]
    pub art: Option<Arc<Art>>,
    /// Where the track was (seconds) when Home Assistant last said, and when
    /// that was (Unix seconds), and how long it is.
    #[serde(default)]
    pub position: Option<f64>,
    #[serde(default)]
    pub position_at: Option<f64>,
    #[serde(default)]
    pub duration: Option<f64>,
}

impl MediaCard {
    /// How far through the track it is at `now` (Unix seconds), 0..1: on
    /// from where it was while it plays. None without a length.
    pub fn progress(&self, now: f64) -> Option<f32> {
        let duration = self.duration.filter(|d| *d > 0.0)?;
        let mut at = self.position?;
        if self.state == "playing" {
            at += (now - self.position_at.unwrap_or(now)).max(0.0);
        }
        Some((at / duration).clamp(0.0, 1.0) as f32)
    }
}

/// Album art: a square picture, RGB, at most ART_MAX wide.
#[derive(Clone, Debug, PartialEq)]
pub struct Art {
    pub size: u32,
    pub rgb: Vec<u8>,
}

/// A camera's still fills the display.
pub const ART_MAX: u32 = 480;
/// The largest JPEG taken (cropped and scaled down to ART_MAX).
const JPEG_MAX: usize = 1920;

impl Art {
    /// From a JPEG (a camera's picture) or a PNG (album art).
    pub fn decode(data: &[u8]) -> Result<Art, String> {
        if data.starts_with(&[0xFF, 0xD8]) {
            Art::decode_jpeg(data)
        } else {
            Art::decode_png(data)
        }
    }

    /// A JPEG, cropped square from its middle and scaled down to fit the
    /// display if it's larger.
    fn decode_jpeg(data: &[u8]) -> Result<Art, String> {
        use zune_core::colorspace::ColorSpace;
        use zune_core::options::DecoderOptions;
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
        let mut decoder = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(data), options);
        // Its size from its header before decoding: a camera's is display
        // sized; a huge one mustn't take the thermostat's memory.
        decoder.decode_headers().map_err(|e| format!("art: {e:?}"))?;
        let (width, height) = decoder.dimensions().ok_or("art: no size")?;
        if width > JPEG_MAX || height > JPEG_MAX {
            return Err(format!("art: {width}x{height}: at most {JPEG_MAX}"));
        }
        let pixels = decoder.decode().map_err(|e| format!("art: {e:?}"))?;
        let (width, height) = decoder.dimensions().ok_or("art: no size")?;
        if pixels.len() < width * height * 3 {
            return Err("art: short".into());
        }
        let side = width.min(height);
        let (left, top) = ((width - side) / 2, (height - side) / 2);
        let size = side.min(ART_MAX as usize);
        let mut rgb = Vec::with_capacity(size * size * 3);
        for y in 0..size {
            let row = top + y * side / size;
            for x in 0..size {
                let at = (row * width + left + x * side / size) * 3;
                rgb.extend_from_slice(&pixels[at..at + 3]);
            }
        }
        Ok(Art { size: size as u32, rgb })
    }

    /// A PNG, square and 8-bit RGB or RGBA (alpha dropped).
    fn decode_png(png_data: &[u8]) -> Result<Art, String> {
        let mut reader = png::Decoder::new(png_data).read_info().map_err(|e| format!("art: {e}"))?;
        // Its size checked before anything is allocated for it: a huge one
        // from anyone on the broker mustn't take the thermostat's memory.
        let header = reader.info();
        if header.width > ART_MAX || header.height > ART_MAX {
            return Err(format!("art: {}x{}: at most {ART_MAX}", header.width, header.height));
        }
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).map_err(|e| format!("art: {e}"))?;
        if info.width != info.height || info.width == 0 || info.width > ART_MAX
            || info.bit_depth != png::BitDepth::Eight {
            return Err(format!("art: {}x{} at {:?} bits: wants square 8-bit, at most {ART_MAX}",
                               info.width, info.height, info.bit_depth));
        }
        let pixels = &buffer[..info.buffer_size()];
        let rgb = match info.color_type {
            png::ColorType::Rgb => pixels.to_vec(),
            png::ColorType::Rgba => pixels.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2]]).collect(),
            other => return Err(format!("art: {other:?}: wants RGB")),
        };
        Ok(Art { size: info.width, rgb })
    }
}

impl Cards {
    /// Home Assistant's payload; an empty one (the retained cards cleared)
    /// is no cards, which leaves the thermostat as it is.
    pub fn parse(payload: &[u8]) -> Result<Cards, String> {
        if payload.iter().all(u8::is_ascii_whitespace) {
            return Ok(Cards::default());
        }
        let mut cards: Cards = serde_json::from_slice(payload).map_err(|e| e.to_string())?;
        cards.cards.retain(|card| match card {
            Card::Lights(lights) => !lights.lights.is_empty(),
            Card::Unknown => false,
            _ => true,
        });
        for card in &mut cards.cards {
            match card {
                Card::Climate(card) => card.settle_action(),
                Card::Face(face) => face.actions.truncate(FACE_ACTIONS),
                _ => {}
            }
        }
        Ok(cards)
    }
}

impl Cards {
    /// Lights Home Assistant reports no colour for (an off one: it only
    /// knows rgb_color while on) keep the colour they last had, so the ring
    /// doesn't change colour as a light turns on or off. `last` holds each
    /// light's, by entity.
    pub fn keep_colours(&mut self, last: &mut std::collections::HashMap<String, [u8; 3]>) {
        for card in &mut self.cards {
            let Card::Lights(card) = card else { continue };
            for light in &mut card.lights {
                match light.rgb {
                    Some(rgb) => {
                        last.insert(light.id.clone(), rgb);
                    }
                    None => light.rgb = last.get(&light.id).copied(),
                }
            }
        }
    }
}

impl Light {
    /// The colour the light shows in: its own while on, Home Assistant's
    /// rgb_color (which it gives colour-temperature lights too), lifted so a
    /// dim colour still reads on black; warm white when it has none.
    pub fn colour(&self) -> [u8; 3] {
        let [r, g, b] = self.rgb.unwrap_or(WARM);
        let top = r.max(g).max(b).max(1) as u32;
        if top >= 170 {
            return [r, g, b];
        }
        let lift = |c: u8| ((c as u32 * 200 / top).min(255)) as u8;
        [lift(r), lift(g), lift(b)]
    }
}

/// A 2700 K lamp's colour, for lights that report none.
pub const WARM: [u8; 3] = [255, 169, 87];

/// One change the thermostat asks of Home Assistant.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    On(String),
    Off(String),
    Brightness(String, u8),
    Temperature(String, f64),
    Volume(String, u8),
    PlayPause(String),
    /// A light's white, in kelvin.
    Kelvin(String, u16),
    /// A light's colour, by hue (0..360).
    Hue(String, u16),
    /// Runs a scene, script or button.
    Run(String),
    /// Asks for a camera's still, now.
    Snapshot(String),
    /// A notification, by its id: pressed, or dismissed (a long press, its
    /// time up).
    AlertPressed(String),
    AlertDismissed(String),
    Next(String),
    Previous(String),
    /// A climate's mode (cool, dry, ...), fan speed, preset or swing, by
    /// Home Assistant's names.
    Climate(String, ClimateChoice, String),
    /// A select's option.
    Select(String, String),
    /// A cover's position, percent open; or open or closed whole.
    Position(String, u8),
    Open(String),
    Close(String),
    /// An alarm armed to a mode (home, away, ...), with its code if it
    /// wants one; `force`, with a door or window open.
    AlarmArm { id: String, mode: String, code: Option<Code>, force: bool },
    AlarmDisarm { id: String, code: Option<Code> },
    /// One of this thermostat's own commands (<base>/set/<name>), done here
    /// rather than sent to Home Assistant: its target, Eco, its settings.
    Nest(String, String),
}

impl Command {
    pub fn payload(&self) -> Value {
        match self {
            Command::Nest(name, arg) => json!({"id": "nest", "action": name, "value": arg}),
            Command::On(id) => json!({"id": id, "action": "on"}),
            Command::Off(id) => json!({"id": id, "action": "off"}),
            Command::Brightness(id, b) => json!({"id": id, "action": "brightness", "value": b}),
            Command::Temperature(id, t) => json!({"id": id, "action": "temperature", "value": t}),
            Command::Volume(id, v) => json!({"id": id, "action": "volume", "value": v}),
            Command::PlayPause(id) => json!({"id": id, "action": "play_pause"}),
            Command::Kelvin(id, k) => json!({"id": id, "action": "color_temp", "value": k}),
            Command::Hue(id, h) => json!({"id": id, "action": "hue", "value": h}),
            Command::Run(id) => json!({"id": id, "action": "run"}),
            Command::Snapshot(id) => json!({"id": id, "action": "snapshot"}),
            Command::AlertPressed(id) => json!({"id": id, "action": "alert_pressed"}),
            Command::AlertDismissed(id) => json!({"id": id, "action": "alert_dismissed"}),
            Command::Next(id) => json!({"id": id, "action": "next"}),
            Command::Previous(id) => json!({"id": id, "action": "previous"}),
            Command::Climate(id, choice, o) => json!({"id": id, "action": choice.action(), "option": o}),
            Command::Select(id, o) => json!({"id": id, "action": "select", "option": o}),
            Command::Position(id, p) => json!({"id": id, "action": "position", "value": p}),
            Command::Open(id) => json!({"id": id, "action": "open"}),
            Command::Close(id) => json!({"id": id, "action": "close"}),
            Command::AlarmArm { id, mode, code, force } => {
                let mut out = json!({"id": id, "action": "arm", "mode": mode, "force": force});
                if let Some(code) = code {
                    out["code"] = json!(code.0);
                }
                out
            }
            Command::AlarmDisarm { id, code } => {
                let mut out = json!({"id": id, "action": "disarm"});
                if let Some(code) = code {
                    out["code"] = json!(code.0);
                }
                out
            }
        }
    }

    /// The payload as the log may show it: an alarm code hidden.
    pub fn logged(&self) -> Value {
        let mut out = self.payload();
        if let Some(code) = out.get_mut("code") {
            *code = json!("…");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_clock_tells_twelve_and_twenty_four_hours() {
        let clock = |json: &str| serde_json::from_str::<super::Clock>(json).unwrap();
        assert_eq!(clock("{}").time(9 * 60 + 5, true), "09:05", "24 hours without");
        let twelve = clock(r#"{"hours":12}"#);
        assert_eq!(twelve.time(0, true), "12:00 AM");
        assert_eq!(twelve.time(12 * 60 + 30, true), "12:30 PM");
        assert_eq!(twelve.time(21 * 60 + 13, false), "9:13", "the big time's AM / PM is drawn apart");
        assert_eq!(clock(r#"{"hours":12,"ampm":false}"#).time(21 * 60 + 13, true), "9:13");
    }
    use super::*;

    #[test]
    fn a_climate_s_action_that_its_mode_rules_out_reads_as_idle() {
        let card = |mode: &str, action: &str| {
            let payload = format!(r#"{{"cards":[{{"kind":"climate","id":"climate.ac","mode":"{mode}","action":"{action}"}}]}}"#);
            match &Cards::parse(payload.as_bytes()).unwrap().cards[0] {
                Card::Climate(card) => card.action.clone(),
                _ => panic!(),
            }
        };
        assert_eq!(card("heat", "cooling"), "idle", "a stale cooling in heat");
        assert_eq!(card("cool", "heating"), "idle");
        assert_eq!(card("off", "heating"), "off");
        assert_eq!(card("heat", "heating"), "heating");
        assert_eq!(card("heat_cool", "cooling"), "cooling");
        assert_eq!(card("dry", "drying"), "drying");
    }

    #[test]
    fn the_nest_s_own_card_lists_a_fan_only_when_it_has_one() {
        let state = |fan: bool| serde_json::json!({"mode": "heat", "fan_mode": "auto", "can_heat": true, "has_fan": fan});
        let without = Nest::from_state(state(false).as_object().unwrap());
        assert!(!without.card.offers(ClimateChoice::Fan), "the bridge says auto even without a fan");
        let with = Nest::from_state(state(true).as_object().unwrap());
        assert_eq!(with.card.choices(ClimateChoice::Fan), (&["auto".to_owned(), "on".to_owned()][..], Some("auto")));
        assert_eq!(with.card.choices(ClimateChoice::Preset).0, ["none", "eco"]);
    }

    #[test]
    fn parses_the_blueprints_cards() {
        let payload = br#"{"cards":[
            {"kind":"lights","name":"Bedroom","icon":"mdi:lightbulb-group","lights":[
                {"id":"light.reading_lamp","name":"Reading Lamp","icon":"m3s:table-lamp","on":true,
                 "dimmable":true,"brightness":60,"rgb":[255,169,87]},
                {"id":"switch.fan","name":"Fan","icon":"","on":false,"dimmable":false,"brightness":null,"rgb":null}]},
            {"kind":"climate","id":"climate.living","name":"Living room","icon":"","mode":"heat",
             "action":"idle","target":19,"current":19.5,"min":7,"max":30,"step":0.5},
            {"kind":"media","id":"media_player.kitchen","name":"Kitchen","icon":"","state":"playing",
             "title":"Mix","artist":"","volume":35},
            {"kind":"lights","name":"Empty","icon":"","lights":[]}]}"#;
        let cards = Cards::parse(payload).unwrap();
        assert_eq!(cards.cards.len(), 3, "a lights card with no lights is dropped");
        let Card::Lights(lights) = &cards.cards[0] else { panic!() };
        assert_eq!(lights.lights[0].brightness, Some(60));
        assert_eq!(lights.lights[1].rgb, None);
        let Card::Climate(climate) = &cards.cards[1] else { panic!() };
        assert_eq!(climate.target, Some(19.0));
        assert_eq!(Cards::parse(b"").unwrap(), Cards::default());
        assert_eq!(cards.wake(), Wake::Chooser, "the chooser by default");
        assert_eq!((cards.far(), cards.near(), cards.card_idle()), (Far::Wake, Near::None, None), "as before protocol 6");
        assert_eq!(cards.thermostat_name(), None, "\"Thermostat\" before protocol 7");
        let named = Cards::parse(br#"{"cards":[],"thermostat_name":" Hallway "}"#).unwrap();
        assert_eq!(named.thermostat_name(), Some("Hallway"));
        let wake = |w: &str| Cards::parse(format!(r#"{{"cards":[{{"kind":"climate","id":"climate.x"}}],"wake":{w}}}"#).as_bytes()).unwrap().wake();
        assert_eq!((wake(r#""thermostat""#), wake("0"), wake("5"), wake(r#""chooser""#)),
                   (Wake::Thermostat, Wake::Card(0), Wake::Chooser, Wake::Chooser));
        assert!(Cards::parse(b"{").is_err());
    }

    #[test]
    fn a_glance_face_is_a_card_with_its_actions_and_hold() {
        let cards = Cards::parse(br#"{"cards":[{"kind":"face","id":"fledge_face.a1","name":"Weather",
            "glance":{"layout":"weather","lines":[{"kind":"date"}]},
            "actions":[{"id":"scene.movie","name":"Movie Night","kind":"run"},
                       {"id":"light.hall","name":"Hall","kind":"switch","on":true},
                       {"id":"scene.c","kind":"run"},{"id":"scene.d","kind":"run"},{"id":"scene.e","kind":"run"}],
            "hold":1}]}"#).unwrap();
        let Card::Face(face) = &cards.cards[0] else { panic!() };
        assert_eq!((face.glance.layout, face.actions.len(), face.hold()), (Layout::Weather, FACE_ACTIONS, Some(1)));
        assert!(face.actions[1].switched() && face.actions[1].on == Some(true) && !face.actions[0].switched());
        let far = Cards::parse(br#"{"cards":[{"kind":"face","id":"f","actions":[],"hold":3}]}"#).unwrap();
        let Card::Face(face) = &far.cards[0] else { panic!() };
        assert_eq!(face.hold(), None, "a hold that isn't one of its actions does nothing");
    }

    #[test]
    fn a_dim_colour_is_lifted_and_none_is_warm() {
        let mut light = Light { id: "light.x".into(), name: String::new(), icon: String::new(), on: true,
                                dimmable: true, brightness: Some(10), rgb: Some([40, 0, 20]), ..Default::default() };
        assert_eq!(light.colour(), [200, 0, 100]);
        light.rgb = Some([255, 249, 253]);
        assert_eq!(light.colour(), [255, 249, 253]);
        light.rgb = None;
        assert_eq!(light.colour(), WARM);
        // Off, it keeps the colour it last had on.
        let lights = |on: bool, rgb: &str| Cards::parse(format!(r#"{{"cards":[{{"kind":"lights","name":"R","icon":"","lights":[
            {{"id":"light.a","name":"A","on":{on},"rgb":{rgb}}}]}}]}}"#).as_bytes()).unwrap();
        let colour = |cards: &Cards| match &cards.cards[0] { Card::Lights(c) => c.lights[0].colour(), _ => unreachable!() };
        let mut last = std::collections::HashMap::new();
        let mut on = lights(true, "[200,220,255]");
        on.keep_colours(&mut last);
        let mut off = lights(false, "null");
        off.keep_colours(&mut last);
        assert_eq!(colour(&off), [200, 220, 255], "not warm white while it's off");
        let mut other = lights(false, "null");
        other.keep_colours(&mut std::collections::HashMap::new());
        assert_eq!(colour(&other), WARM, "never seen on: warm white");
    }

    #[test]
    fn commands_name_the_entity_and_action() {
        assert_eq!(Command::Brightness("light.x".into(), 40).payload(),
                   json!({"id": "light.x", "action": "brightness", "value": 40}));
        assert_eq!(Command::PlayPause("media_player.k".into()).payload()["action"], "play_pause");
    }
}
