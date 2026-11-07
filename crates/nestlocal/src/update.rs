//! What Home Assistant's update entity offers: this build's version, and the
//! latest release of the GitHub repository -update-repo names (owner/name).
//!
//! With -update-key, the entity installs a newer release
//! (utils/release-nestlocal.sh publishes them): its nestlocal-armv7 must match
//! GitHub's SHA-256 for it and the Ed25519 signature nestlocal-armv7.sig under
//! that key. It replaces /nestlabs/sbin/nestlocal, the old one kept as
//! nestlocal.prev, and PENDING is left for nestlocal.init, which counts starts
//! until the new build clears it once nlclient is talking to it, and after
//! three puts nestlocal.prev back, recording the version in FAILED.
//!
//! The latest release is asked for when the broker connects, when Home
//! Assistant presses the entity's check button, and once a day. A day is
//! counted by the wall clock: tokio's timers stop while the unit sleeps.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use http_body_util::BodyExt;
use serde_json::Value;

/// This build: UNFR0ZEN_BUILD_VERSION, else `git describe` (build.rs).
pub const VERSION: &str = env!("NESTLOCAL_VERSION");
/// How often the latest release is asked for.
pub const RECHECK: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the daily check looks at the wall clock.
pub const TICK: Duration = Duration::from_secs(10 * 60);
/// A request that has not finished by then failed (the unit may have slept).
const TIMEOUT: Duration = Duration::from_secs(30);
/// A download that has not finished by then failed.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// Larger than any nestlocal: a release asset beyond it is refused.
const MAX_DOWNLOAD: usize = 16 << 20;
const API_HOST: &str = "api.github.com";
/// The release's binary and its signature.
pub const ASSET: &str = "nestlocal-armv7";
pub const SIGNATURE: &str = "nestlocal-armv7.sig";
/// Left by an install until the new build is healthy (nestlocal.init).
pub const PENDING: &str = "/etc/nestlocal/update.pending";
/// The version nestlocal.init rolled back from.
pub const FAILED: &str = "/etc/nestlocal/update.failed";
pub const DAEMON: &str = "/nestlabs/sbin/nestlocal";
/// Room left beside the new binary on its filesystem.
const SPARE: u64 = 1 << 20;

/// A file of a release.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    /// GitHub's digest of it, `sha256:<hex>`.
    pub digest: String,
}

/// A repository's latest release.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    pub url: String,
    pub notes: String,
    pub assets: Vec<Asset>,
}

impl Release {
    fn asset(&self, name: &str) -> Result<&Asset, String> {
        self.assets.iter().find(|a| a.name == name).ok_or_else(|| format!("{} has no {name}", self.tag))
    }
}

/// The release GitHub's /releases/latest answered with.
pub fn parse_release(body: &[u8]) -> Result<Release, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("release: {e}"))?;
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let tag = text("tag_name");
    if tag.is_empty() {
        return Err("release: no tag_name".into());
    }
    let assets = v.get("assets").and_then(Value::as_array).into_iter().flatten().map(|a| {
        let text = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        Asset { name: text("name"), url: text("browser_download_url"), digest: text("digest") }
    }).collect();
    Ok(Release { tag, url: text("html_url"), notes: text("body"), assets })
}

/// What orders versions: vX.Y.Z, then -ben.N (utils/release-nestlocal.sh),
/// then git describe's -K-gHASH commits after it. -dirty is ignored.
fn version_key(version: &str) -> Option<(u32, u32, u32, u32, u32)> {
    let rest = version.strip_prefix('v')?;
    let (core, mut rest) = rest.split_once('-').unwrap_or((rest, ""));
    let mut parts = core.split('.').map(|p| p.parse::<u32>().ok());
    let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() {
        return None;
    }
    let mut ben = 0;
    if let Some(after) = rest.strip_prefix("ben.") {
        let (n, tail) = after.split_once('-').unwrap_or((after, ""));
        ben = n.parse().ok()?;
        rest = tail;
    }
    let mut commits = 0;
    if let Some((k, hash)) = rest.split_once('-') {
        if hash.starts_with('g') {
            commits = k.parse().ok()?;
        }
    }
    Some((major, minor, patch, ben, commits))
}

/// The release a build is on: git describe without -K-gHASH and -dirty.
pub fn release_of(version: &str) -> &str {
    let version = version.strip_suffix("-dirty").unwrap_or(version);
    match version.rsplit_once("-g") {
        Some((head, hash)) if !hash.is_empty() && hash.chars().all(|c| c.is_ascii_hexdigit()) => {
            match head.rsplit_once('-') {
                Some((base, k)) if !k.is_empty() && k.chars().all(|c| c.is_ascii_digit()) => base,
                _ => version,
            }
        }
        _ => version,
    }
}

/// Whether `latest` is newer than `installed`; neither readable is not.
pub fn newer(latest: &str, installed: &str) -> bool {
    matches!((version_key(latest), version_key(installed)), (Some(l), Some(i)) if l > i)
}

/// An Ed25519 public key, base64 as utils/release-nestlocal.sh prints it.
pub fn parse_key(text: &str) -> Result<[u8; 32], String> {
    base64(text.trim())?.try_into().map_err(|_| "expected a 32-byte Ed25519 public key".to_string())
}

fn base64(text: &str) -> Result<Vec<u8>, String> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let text = text.trim_end_matches('=');
    let (mut out, mut bits, mut n) = (Vec::new(), 0u32, 0);
    for c in text.bytes() {
        bits = bits << 6 | u32::from(value(c).ok_or("not base64")?);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    Ok(out)
}

/// Fails unless `data` hashes to `digest` (`sha256:<hex>`) and `signature`
/// is `key`'s Ed25519 signature of it.
pub fn check(data: &[u8], digest: &str, signature: &[u8], key: &[u8; 32]) -> Result<(), String> {
    let expected = digest.strip_prefix("sha256:").ok_or_else(|| format!("no SHA-256 digest ({digest:?})"))?;
    let actual: String = ring::digest::digest(&ring::digest::SHA256, data).as_ref().iter().map(|b| format!("{b:02x}")).collect();
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!("SHA-256 {actual} is not GitHub's {expected}"));
    }
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key).verify(data, signature)
        .map_err(|_| "the signature does not verify under -update-key".to_string())
}

/// Where an install puts things: DAEMON, PENDING (tests use a directory).
pub struct Paths {
    pub daemon: PathBuf,
    pub pending: PathBuf,
}

impl Paths {
    pub fn unit() -> Paths {
        Paths { daemon: DAEMON.into(), pending: PENDING.into() }
    }

    fn with(&self, suffix: &str) -> PathBuf {
        let mut path = self.daemon.clone().into_os_string();
        path.push(suffix);
        path.into()
    }

    /// Puts `binary` in place of the daemon, keeping the old one as .prev (a
    /// hard link: no space, and the running process keeps its path),
    /// and leaves PENDING (0 starts, the version) for nestlocal.init.
    pub fn install(&self, binary: &[u8], version: &str) -> Result<(), String> {
        use std::io::Write;
        let err = |what: &str, path: &Path, e: std::io::Error| format!("{what} {}: {e}", path.display());
        if self.pending.exists() {
            return Err("an earlier update has not finished starting".into());
        }
        let (prev, new) = (self.with(".prev"), self.with(".new"));
        let _ = std::fs::remove_file(&prev);
        let _ = std::fs::remove_file(&new);
        let dir = self.daemon.parent().unwrap_or(Path::new("/"));
        let free = free_space(dir).map_err(|e| err("free space of", dir, e))?;
        if free < binary.len() as u64 + SPARE {
            return Err(format!("{} has {} KiB free, the update needs {} KiB", dir.display(), free >> 10,
                               (binary.len() as u64 + SPARE) >> 10));
        }
        let mut file = std::fs::File::create(&new).map_err(|e| err("create", &new, e))?;
        file.write_all(binary).and_then(|()| file.sync_all()).map_err(|e| err("write", &new, e))?;
        drop(file);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).map_err(|e| err("chmod", &new, e))?;
        std::fs::write(&self.pending, format!("0\n{version}\n")).map_err(|e| err("write", &self.pending, e))?;
        // A second name for the running build, not a rename: start-stop-daemon
        // finds the running nestlocal by its executable's path, which a
        // rename would change, and stopping it would then miss it.
        std::fs::hard_link(&self.daemon, &prev).map_err(|e| err("keep", &self.daemon, e))?;
        if let Err(e) = std::fs::rename(&new, &self.daemon) {
            let _ = std::fs::remove_file(&prev);
            let _ = std::fs::remove_file(&self.pending);
            return Err(err("install", &new, e));
        }
        sync();
        Ok(())
    }
}

#[cfg(unix)]
fn free_space(dir: &Path) -> std::io::Result<u64> {
    let path = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).map_err(std::io::Error::other)?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    #[allow(clippy::unnecessary_cast)] // the fields' widths differ by platform
    Ok(st.f_bavail as u64 * st.f_frsize as u64)
}

fn sync() {
    unsafe { libc::sync() };
}

/// The version an install left pending, if the new build has not yet
/// cleared it.
pub fn pending() -> Option<String> {
    let text = std::fs::read_to_string(PENDING).ok()?;
    Some(text.lines().nth(1).unwrap_or("?").to_string())
}

/// The new build is healthy: nestlocal.init stops counting.
pub fn commit() {
    let _ = std::fs::remove_file(PENDING);
    sync();
}

/// The version nestlocal.init last rolled back from.
pub fn failed() -> Option<String> {
    std::fs::read_to_string(FAILED).ok().map(|t| t.trim().to_string()).filter(|v| !v.is_empty())
}

/// The version the entity offers: `tag`, or this build itself when it is
/// that release or was built on top of it (`git describe` puts the tag
/// first), so that a development build does not offer its own base.
pub fn offered(installed: &str, tag: &str) -> String {
    let built_on = installed.strip_prefix(tag).is_some_and(|rest| rest.is_empty() || rest.starts_with('-'));
    if built_on { installed.to_string() } else { tag.to_string() }
}

/// Whether owner/name is a repository path GitHub can have.
pub fn valid_repo(repo: &str) -> bool {
    let ok = |s: &str| !s.is_empty() && s != "." && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    repo.split_once('/').is_some_and(|(owner, name)| ok(owner) && ok(name))
}

/// What an install is doing, for the entity.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Progress {
    #[default]
    Idle,
    /// Downloading, percent done.
    Downloading(u8),
    /// Installed; nestlocal restarts into it.
    Restarting,
    Failed(String),
}

/// The entity's state: what the MQTT update entity reads from its state topic.
pub fn state(latest: Option<&Release>, progress: &Progress, failed: Option<&str>) -> Value {
    let mut state = serde_json::json!({ "installed_version": VERSION, "title": "nestlocal" });
    state["in_progress"] = matches!(progress, Progress::Downloading(_) | Progress::Restarting).into();
    if let Progress::Downloading(percent) = progress {
        state["update_percentage"] = (*percent).into();
    }
    let note = match (progress, failed) {
        (Progress::Failed(e), _) => Some(format!("install failed: {e}")),
        (_, Some(v)) => Some(format!("{v} did not start and was rolled back")),
        _ => None,
    };
    if let Some(note) = note {
        state["title"] = format!("nestlocal ({note})").into();
    }
    if let Some(r) = latest {
        state["latest_version"] = offered(VERSION, &r.tag).into();
        // Home Assistant ranks a development build's -K-gHASH above a plain
        // -ben.N, so it would not offer a newer release: show it the release
        // the build is on (the title keeps the build).
        if newer(&r.tag, VERSION) {
            let base = release_of(VERSION);
            if base != VERSION {
                state["installed_version"] = base.into();
                state["title"] = format!("nestlocal {VERSION}").into();
            }
        }
        state["release_url"] = r.url.clone().into();
        // Home Assistant shows at most 255 characters of the summary.
        state["release_summary"] = r.notes.chars().take(255).collect::<String>().into();
    }
    state
}

/// Whether the last check, at `last`, is a day old at `now`. A check dated
/// ahead of the clock (it was set back) counts as old.
pub fn due(last: Option<SystemTime>, now: SystemTime) -> bool {
    match last.map(|last| now.duration_since(last)) {
        Some(Ok(age)) => age >= RECHECK,
        None | Some(Err(_)) => true,
    }
}

/// The releases of one repository, as last asked for.
pub struct Updates {
    pub repo: String,
    /// -update-key: with it, newer releases install.
    pub key: Option<[u8; 32]>,
    tls: Arc<rustls::ClientConfig>,
    pub progress: Mutex<Progress>,
    pub latest: Mutex<Option<Release>>,
    /// When the latest release was last asked for, by the wall clock.
    pub checked: Mutex<Option<SystemTime>>,
    /// Home Assistant pressed the check button.
    pub check: tokio::sync::Notify,
}

impl Updates {
    /// Mozilla's roots (webpki-roots), not the unit's own bundle: that is
    /// Nest's, from the firmware's day.
    pub fn new(repo: &str, key: Option<[u8; 32]>) -> Updates {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let tls = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        Updates { repo: repo.into(), key, tls: Arc::new(tls), progress: Mutex::new(Progress::Idle),
                  latest: Mutex::new(None), checked: Mutex::new(None), check: tokio::sync::Notify::new() }
    }

    /// Downloads, checks and installs the latest release if it is newer;
    /// its version. `downloaded` hears the percentage as it comes.
    pub async fn install(&self, paths: &Paths, downloaded: impl Fn(u8)) -> Result<String, String> {
        let key = self.key.ok_or("no -update-key")?;
        let release = self.refresh().await?.ok_or_else(|| format!("{} has no release", self.repo))?;
        if !newer(&release.tag, VERSION) {
            return Err(format!("{} is not newer than {VERSION}", release.tag));
        }
        if failed().as_deref() == Some(release.tag.as_str()) {
            return Err(format!("{} was rolled back before; remove {FAILED} to try it again", release.tag));
        }
        let (asset, signature) = (release.asset(ASSET)?, release.asset(SIGNATURE)?);
        let signature = get(&self.tls, &signature.url, 1024, |_| {}).await?;
        let binary = tokio::time::timeout(DOWNLOAD_TIMEOUT, get(&self.tls, &asset.url, MAX_DOWNLOAD, &downloaded)).await
            .map_err(|_| "the download timed out".to_string())??;
        check(&binary, &asset.digest, &signature, &key)?;
        paths.install(&binary, &release.tag)?;
        Ok(release.tag)
    }

    /// Asks GitHub for the latest release and keeps it. None: the
    /// repository has no release yet.
    pub async fn refresh(&self) -> Result<Option<Release>, String> {
        *self.checked.lock().unwrap() = Some(SystemTime::now());
        // Asleep, the unit would drop the request half way.
        crate::wake::keep_awake_for(TIMEOUT.as_secs() + 5);
        let latest = tokio::time::timeout(TIMEOUT, fetch(&self.repo, self.tls.clone())).await
            .map_err(|_| "timed out".to_string())??;
        *self.latest.lock().unwrap() = latest.clone();
        Ok(latest)
    }
}

async fn fetch(repo: &str, tls: Arc<rustls::ClientConfig>) -> Result<Option<Release>, String> {
    let url = format!("https://{API_HOST}/repos/{repo}/releases/latest");
    match request(&tls, &url, 1 << 20, |_| {}).await? {
        (200, body) => parse_release(&body).map(Some),
        (404, _) => Ok(None),
        (code, _) => Err(format!("{repo}: HTTP {code}")),
    }
}

/// GETs an https URL that must answer 200, following GitHub's redirects
/// to its download host; `progress` hears the percentage received.
async fn get(tls: &Arc<rustls::ClientConfig>, url: &str, limit: usize, progress: impl Fn(u8)) -> Result<Vec<u8>, String> {
    let mut url = url.to_string();
    for _ in 0..5 {
        let response = send(tls, &url).await?;
        let status = response.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let location = response.headers().get("location").and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("{url}: redirect without a location"))?;
            url = location.to_string();
            continue;
        }
        if status != 200 {
            return Err(format!("{url}: HTTP {status}"));
        }
        return read(response, limit, &progress).await;
    }
    Err(format!("{url}: too many redirects"))
}

async fn request(tls: &Arc<rustls::ClientConfig>, url: &str, limit: usize, progress: impl Fn(u8)) -> Result<(u16, Vec<u8>), String> {
    let response = send(tls, url).await?;
    let status = response.status().as_u16();
    Ok((status, read(response, limit, &progress).await?))
}

async fn send(tls: &Arc<rustls::ClientConfig>, url: &str) -> Result<hyper::Response<hyper::body::Incoming>, String> {
    let rest = url.strip_prefix("https://").ok_or_else(|| format!("{url}: not https"))?;
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let path = if path.is_empty() { "/" } else { path };
    let tcp = tokio::net::TcpStream::connect((host, 443)).await.map_err(|e| format!("{host}: {e}"))?;
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
    let stream = tokio_rustls::TlsConnector::from(tls.clone()).connect(name, tcp).await.map_err(|e| format!("{host}: {e}"))?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
        .await.map_err(|e| chain(&e))?;
    tokio::spawn(conn);
    let request = hyper::Request::get(path)
        .header("host", host)
        .header("user-agent", format!("nestlocal/{VERSION}"))
        .header("accept", if host == API_HOST { "application/vnd.github+json" } else { "application/octet-stream" })
        .body(http_body_util::Empty::<bytes::Bytes>::new())
        .map_err(|e| e.to_string())?;
    sender.send_request(request).await.map_err(|e| format!("{host}: {}", chain(&e)))
}

/// An error and its causes: hyper's own says only "connection error".
fn chain(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut cause = e.source();
    while let Some(c) = cause {
        text = format!("{text}: {c}");
        cause = c.source();
    }
    text
}

async fn read(response: hyper::Response<hyper::body::Incoming>, limit: usize, progress: &impl Fn(u8)) -> Result<Vec<u8>, String> {
    let length = response.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse::<usize>().ok());
    if length.is_some_and(|n| n > limit) {
        return Err(format!("{} bytes is more than {limit}", length.unwrap_or(0)));
    }
    let mut body = response.into_body();
    let mut data = Vec::with_capacity(length.unwrap_or(0));
    let mut shown = None;
    while let Some(frame) = body.frame().await {
        if let Ok(chunk) = frame.map_err(|e| chain(&e))?.into_data() {
            data.extend_from_slice(&chunk);
            if data.len() > limit {
                return Err(format!("more than {limit} bytes"));
            }
            if let Some(total) = length.filter(|n| *n > 0) {
                let percent = (data.len() * 100 / total).min(100) as u8;
                if shown != Some(percent) {
                    shown = Some(percent);
                    progress(percent);
                }
            }
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tag_page_and_notes_of_the_latest_release() {
        let body = br#"{"tag_name":"v0.3.2","html_url":"https://github.com/o/r/releases/tag/v0.3.2","body":"Fixes","draft":false}"#;
        assert_eq!(parse_release(body).unwrap(), Release {
            tag: "v0.3.2".into(), url: "https://github.com/o/r/releases/tag/v0.3.2".into(), notes: "Fixes".into(),
            assets: vec![] });
        let body = br#"{"tag_name":"v0.3.2-ben.2","assets":[{"name":"nestlocal-armv7",
            "browser_download_url":"https://github.com/o/r/releases/download/v0.3.2-ben.2/nestlocal-armv7","digest":"sha256:ab"}]}"#;
        let release = parse_release(body).unwrap();
        assert_eq!(release.asset(ASSET).unwrap().digest, "sha256:ab");
        assert!(release.asset(SIGNATURE).is_err());
        assert!(parse_release(br#"{"message":"Not Found"}"#).is_err());
        assert!(parse_release(b"<html>").is_err());
    }

    #[test]
    fn a_build_on_top_of_the_latest_release_offers_itself() {
        assert_eq!(offered("v0.3.1-11-g9dbe5ec", "v0.3.1"), "v0.3.1-11-g9dbe5ec");
        assert_eq!(offered("v0.3.1", "v0.3.1"), "v0.3.1");
        assert_eq!(offered("v0.3.1-dirty", "v0.3.1"), "v0.3.1-dirty");
        assert_eq!(offered("v0.3.1-11-g9dbe5ec", "v0.3.2"), "v0.3.2");
        // v0.3.10 is not built on v0.3.1.
        assert_eq!(offered("v0.3.10", "v0.3.1"), "v0.3.1");
    }

    #[test]
    fn only_owner_slash_name_is_a_repository() {
        assert!(valid_repo("sysmoon14/unfr0zen"));
        assert!(valid_repo("a-b/c.d_e"));
        for repo in ["", "unfr0zen", "a/b/c", "/r", "o/", "o/..", "o/r?x=1", "o/r#x", "o r/x"] {
            assert!(!valid_repo(repo), "{repo}");
        }
    }

    #[test]
    fn the_state_offers_a_release_only_once_one_is_known() {
        let s = state(None, &Progress::Idle, None);
        assert_eq!(s["installed_version"], VERSION);
        assert!(s.get("latest_version").is_none());
        let notes = "x".repeat(300);
        let s = state(Some(&Release { tag: "v99.0.0".into(), url: "u".into(), notes, assets: vec![] }), &Progress::Idle, None);
        assert_eq!(s["latest_version"], "v99.0.0");
        assert_eq!(s["release_url"], "u");
        assert_eq!(s["release_summary"].as_str().unwrap().len(), 255);
    }

    /// Asks GitHub itself: `cargo test -p nestlocal -- --ignored update`.
    #[tokio::test]
    #[ignore = "needs the network"]
    async fn asks_github_for_the_latest_release() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let release = Updates::new("unfr0zen/unfr0zen", None).refresh().await.unwrap().expect("a release");
        assert!(release.tag.starts_with('v'), "{release:?}");
        assert!(Updates::new("unfr0zen/no-such-repository-here", None).refresh().await.unwrap().is_none());
    }

    #[test]
    fn the_state_shows_an_install_and_a_rollback() {
        let s = state(None, &Progress::Downloading(40), None);
        assert_eq!((s["in_progress"].clone(), s["update_percentage"].clone()), (true.into(), 40.into()));
        assert_eq!(state(None, &Progress::Restarting, None)["in_progress"], true);
        let s = state(None, &Progress::Idle, Some("v9.9.9-ben.1"));
        assert_eq!(s["in_progress"], false);
        assert!(s["title"].as_str().unwrap().contains("v9.9.9-ben.1 did not start"));
        assert!(state(None, &Progress::Failed("no room".into()), None)["title"].as_str().unwrap().contains("no room"));
    }

    #[test]
    fn a_build_s_release_is_its_tag() {
        assert_eq!(release_of("v0.3.2-ben.1-1-g51fc988"), "v0.3.2-ben.1");
        assert_eq!(release_of("v0.3.2-16-gffb3d93-dirty"), "v0.3.2");
        assert_eq!(release_of("v0.3.2-ben.2"), "v0.3.2-ben.2");
        assert_eq!(release_of("f290a39"), "f290a39");
    }

    #[test]
    fn home_assistant_sees_a_development_build_as_its_release_when_one_is_newer() {
        let r = Release { tag: "v99.0.0-ben.1".into(), url: "u".into(), notes: String::new(), assets: vec![] };
        let s = state(Some(&r), &Progress::Idle, None);
        assert_eq!(s["installed_version"], release_of(VERSION));
        assert_eq!(s["latest_version"], "v99.0.0-ben.1");
        assert_eq!(state(None, &Progress::Idle, None)["title"], "nestlocal");
    }

    #[test]
    fn versions_order_by_release_then_fork_release_then_commits() {
        assert!(newer("v0.3.2-ben.2", "v0.3.2-ben.1"));
        assert!(newer("v0.3.2-ben.2", "v0.3.2-ben.1-5-gabc1234"));
        assert!(newer("v0.3.3", "v0.3.2-ben.9"));
        assert!(newer("v0.3.3-ben.1", "v0.3.2-16-gffb3d93"));
        assert!(newer("v0.10.0", "v0.9.9"));
        assert!(!newer("v0.3.2-ben.1", "v0.3.2-ben.1"));
        assert!(!newer("v0.3.2-ben.1", "v0.3.2-ben.1-1-gabc1234-dirty"));
        assert!(!newer("v0.3.2-ben.1", "v0.3.2-ben.2"));
        assert!(!newer("v0.3.1", "v0.3.2-ben.1"));
        // Unreadable either side: never newer.
        assert!(!newer("latest", "v0.3.2"));
        assert!(!newer("v0.3.3", "f290a39"));
    }

    // A signature utils/release-nestlocal.sh's OpenSSL made, with a throwaway key.
    const KEY: &str = "s7Kavx6B57kBQRZzXkMumhYJCyBoQ08oLDTy2BBPpH4=";
    const SIGNED: &[u8] = b"nestlocal test binary";
    const DIGEST: &str = "sha256:ae52a2321030927686802e55ed29f4b153c23a609375e0bf2fbf9246cd1f216c";
    const SIG: &str = "d3d5037678d94b5669194b733593aa02c74cb354637a09653b82d116c29cb5e463e08704d6d371f8ebd09c7367e11c384c7c626d86013b09b9feca7378199400";

    fn sig() -> Vec<u8> {
        (0..SIG.len()).step_by(2).map(|i| u8::from_str_radix(&SIG[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn checks_github_s_digest_and_an_openssl_signature() {
        let key = parse_key(KEY).unwrap();
        check(SIGNED, DIGEST, &sig(), &key).unwrap();
        assert!(check(b"nestlocal test binarY", DIGEST, &sig(), &key).unwrap_err().contains("SHA-256"));
        let mut bad = sig();
        bad[0] ^= 1;
        assert!(check(SIGNED, DIGEST, &bad, &key).unwrap_err().contains("signature"));
        let other = parse_key("1mD83Vorls2abCO0i4u8gqDj69XNxM7UyWl8csxch9Y=").unwrap();
        assert!(check(SIGNED, DIGEST, &sig(), &other).is_err());
        assert!(check(SIGNED, "", &sig(), &key).is_err());
    }

    #[test]
    fn a_key_is_32_bytes_of_base64() {
        assert!(parse_key("7eIg9OAPKjpPeuXdvCXlOy+VAZfnSHDGOJY4HUIdgy8=").is_ok());
        assert!(parse_key("7eIg9OAPKjpPeuXdvCXlOy+VAZfnSHDGOJY4HUId").is_err());
        assert!(parse_key("not a key!").is_err());
    }

    #[test]
    fn an_install_keeps_the_old_binary_and_leaves_pending() {
        let dir = std::env::temp_dir().join(format!("nestlocal-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths { daemon: dir.join("nestlocal"), pending: dir.join("update.pending") };
        std::fs::write(&paths.daemon, b"old").unwrap();
        paths.install(b"new", "v0.3.2-ben.2").unwrap();
        assert_eq!(std::fs::read(&paths.daemon).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("nestlocal.prev")).unwrap(), b"old");
        assert!(!dir.join("nestlocal.new").exists());
        assert_eq!(std::fs::read_to_string(&paths.pending).unwrap(), "0\nv0.3.2-ben.2\n");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&paths.daemon).unwrap().permissions().mode() & 0o777, 0o755);
        // Not over a build still pending.
        assert!(paths.install(b"newer", "v0.3.2-ben.3").is_err());
        assert_eq!(std::fs::read(&paths.daemon).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// nestlocal.init, run by sh with its paths moved into a directory and
    /// start-stop-daemon a stub.
    #[test]
    fn the_init_script_rolls_back_after_three_starts_of_a_pending_update() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("nestlocal-init-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let d = dir.display().to_string();
        let exe = |path: &std::path::Path, text: &str| {
            std::fs::write(path, text).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        exe(&dir.join("bin/start-stop-daemon"), "#!/bin/sh\nexit 0\n");
        exe(&dir.join("nestlocal"), "#!/bin/sh\n# new\nexit 0\n");
        exe(&dir.join("nestlocal.prev"), "#!/bin/sh\n# old\nexit 0\n");
        std::fs::write(dir.join("options"), "-mqtt tcp://broker:1883\n").unwrap();
        std::fs::write(dir.join("update.pending"), "0\nv0.3.2-ben.3\n").unwrap();
        let init = include_str!("../rootfs/nestlocal.init")
            .replace("/nestlabs/sbin/nestlocal", &format!("{d}/nestlocal"))
            .replace("/var/run/nestlocal.pid", &format!("{d}/pid"))
            .replace("/etc/nestlocal/options", &format!("{d}/options"))
            .replace("/etc/nestlocal/update.", &format!("{d}/update."))
            .replace("/var/run/unfr0zen/", &format!("{d}/"));
        exe(&dir.join("init"), &init);
        let start = || {
            let path = format!("{d}/bin:{}", std::env::var("PATH").unwrap_or_default());
            let out = std::process::Command::new("sh").arg(dir.join("init")).arg("start").env("PATH", path).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        };
        for n in 1..=3 {
            start();
            assert_eq!(std::fs::read_to_string(dir.join("update.pending")).unwrap(), format!("{n}\nv0.3.2-ben.3\n"));
            assert!(std::fs::read_to_string(dir.join("nestlocal")).unwrap().contains("new"));
        }
        start();
        assert!(!dir.join("update.pending").exists());
        assert!(!dir.join("nestlocal.prev").exists());
        assert!(std::fs::read_to_string(dir.join("nestlocal")).unwrap().contains("old"));
        assert_eq!(std::fs::read_to_string(dir.join("update.failed")).unwrap(), "v0.3.2-ben.3\n");
        // Committed (no PENDING): starts are not counted.
        start();
        // stop kills what pidof still finds; here nothing is left.
        exe(&dir.join("bin/pidof"), "#!/bin/sh\nexit 1\n");
        let path = format!("{d}/bin:{}", std::env::var("PATH").unwrap_or_default());
        let out = std::process::Command::new("sh").arg(dir.join("init")).arg("stop").env("PATH", path).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(std::fs::read_to_string(dir.join("nestlocal")).unwrap().contains("old"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_check_is_due_daily_by_the_wall_clock() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
        assert!(due(None, now));
        assert!(!due(Some(now - Duration::from_secs(3600)), now));
        assert!(due(Some(now - RECHECK), now));
        assert!(due(Some(now + Duration::from_secs(60)), now));
    }
}
