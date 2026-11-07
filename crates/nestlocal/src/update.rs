//! What Home Assistant's update entity offers: this build's version, and the
//! latest release of the GitHub repository -update-repo names (owner/name).
//! Nothing is installed from here; the entity only tells of a newer release.
//!
//! The latest release is asked for when the broker connects, when Home
//! Assistant presses the entity's check button, and once a day. A day is
//! counted by the wall clock: tokio's timers stop while the unit sleeps.

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
const API_HOST: &str = "api.github.com";

/// A repository's latest release.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub tag: String,
    pub url: String,
    pub notes: String,
}

/// The release GitHub's /releases/latest answered with.
pub fn parse_release(body: &[u8]) -> Result<Release, String> {
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("release: {e}"))?;
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let tag = text("tag_name");
    if tag.is_empty() {
        return Err("release: no tag_name".into());
    }
    Ok(Release { tag, url: text("html_url"), notes: text("body") })
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

/// The entity's state: what the MQTT update entity reads from its state topic.
pub fn state(latest: Option<&Release>) -> Value {
    let mut state = serde_json::json!({ "installed_version": VERSION });
    if let Some(r) = latest {
        state["latest_version"] = offered(VERSION, &r.tag).into();
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
    tls: Arc<rustls::ClientConfig>,
    pub latest: Mutex<Option<Release>>,
    /// When the latest release was last asked for, by the wall clock.
    pub checked: Mutex<Option<SystemTime>>,
    /// Home Assistant pressed the check button.
    pub check: tokio::sync::Notify,
}

impl Updates {
    /// Mozilla's roots (webpki-roots), not the unit's own bundle: that is
    /// Nest's, from the firmware's day.
    pub fn new(repo: &str) -> Updates {
        let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let tls = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        Updates { repo: repo.into(), tls: Arc::new(tls), latest: Mutex::new(None), checked: Mutex::new(None),
                  check: tokio::sync::Notify::new() }
    }

    /// Asks GitHub for the latest release and keeps it. None: the
    /// repository has no release yet.
    pub async fn refresh(&self) -> Result<Option<Release>, String> {
        *self.checked.lock().unwrap() = Some(SystemTime::now());
        let latest = tokio::time::timeout(TIMEOUT, fetch(&self.repo, self.tls.clone())).await
            .map_err(|_| "timed out".to_string())??;
        *self.latest.lock().unwrap() = latest.clone();
        Ok(latest)
    }
}

async fn fetch(repo: &str, tls: Arc<rustls::ClientConfig>) -> Result<Option<Release>, String> {
    let tcp = tokio::net::TcpStream::connect((API_HOST, 443)).await.map_err(|e| format!("{API_HOST}: {e}"))?;
    let name = rustls::pki_types::ServerName::try_from(API_HOST).map_err(|e| e.to_string())?;
    let stream = tokio_rustls::TlsConnector::from(tls).connect(name, tcp).await.map_err(|e| format!("{API_HOST}: {e}"))?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
        .await.map_err(|e| e.to_string())?;
    tokio::spawn(conn);
    let request = hyper::Request::get(format!("/repos/{repo}/releases/latest"))
        .header("host", API_HOST)
        .header("user-agent", format!("nestlocal/{VERSION}"))
        .header("accept", "application/vnd.github+json")
        .body(http_body_util::Empty::<bytes::Bytes>::new())
        .map_err(|e| e.to_string())?;
    let response = sender.send_request(request).await.map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.into_body().collect().await.map_err(|e| e.to_string())?.to_bytes();
    match status.as_u16() {
        200 => parse_release(&body).map(Some),
        404 => Ok(None),
        code => Err(format!("{repo}: HTTP {code}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_tag_page_and_notes_of_the_latest_release() {
        let body = br#"{"tag_name":"v0.3.2","html_url":"https://github.com/o/r/releases/tag/v0.3.2","body":"Fixes","draft":false}"#;
        assert_eq!(parse_release(body).unwrap(), Release {
            tag: "v0.3.2".into(), url: "https://github.com/o/r/releases/tag/v0.3.2".into(), notes: "Fixes".into() });
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
        let s = state(None);
        assert_eq!(s["installed_version"], VERSION);
        assert!(s.get("latest_version").is_none());
        let notes = "x".repeat(300);
        let s = state(Some(&Release { tag: "v99.0.0".into(), url: "u".into(), notes }));
        assert_eq!(s["latest_version"], "v99.0.0");
        assert_eq!(s["release_url"], "u");
        assert_eq!(s["release_summary"].as_str().unwrap().len(), 255);
    }

    /// Asks GitHub itself: `cargo test -p nestlocal -- --ignored update`.
    #[tokio::test]
    #[ignore = "needs the network"]
    async fn asks_github_for_the_latest_release() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let release = Updates::new("unfr0zen/unfr0zen").refresh().await.unwrap().expect("a release");
        assert!(release.tag.starts_with('v'), "{release:?}");
        assert!(Updates::new("unfr0zen/no-such-repository-here").refresh().await.unwrap().is_none());
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
