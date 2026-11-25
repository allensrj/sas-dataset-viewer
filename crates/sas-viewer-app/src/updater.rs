use serde::Deserialize;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = "https://github.com/allensrj/sas-dataset-viewer";
pub const RELEASES_URL: &str = "https://github.com/allensrj/sas-dataset-viewer/releases";
const LATEST_API_URL: &str =
    "https://api.github.com/repos/allensrj/sas-dataset-viewer/releases/latest";
const LATEST_PAGE_URL: &str = "https://github.com/allensrj/sas-dataset-viewer/releases/latest";

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub html_url: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone)]
pub enum UpdateCheckState {
    Idle,
    Checking,
    UpToDate {
        latest_tag: String,
    },
    UpdateAvailable(ReleaseInfo),
    Error(String),
}

pub struct UpdateChecker {
    pub state: UpdateCheckState,
    rx: Option<Receiver<Result<ReleaseInfo, String>>>,
    last_check_was_manual: bool,
    pub banner_dismissed_tag: Option<String>,
}

impl Default for UpdateChecker {
    fn default() -> Self {
        Self {
            state: UpdateCheckState::Idle,
            rx: None,
            last_check_was_manual: false,
            banner_dismissed_tag: None,
        }
    }
}

impl UpdateChecker {
    pub fn is_checking(&self) -> bool {
        matches!(self.state, UpdateCheckState::Checking)
    }

    pub fn available_update(&self) -> Option<&ReleaseInfo> {
        match &self.state {
            UpdateCheckState::UpdateAvailable(info) => Some(info),
            _ => None,
        }
    }

    pub fn should_show_toolbar_badge(&self) -> Option<&ReleaseInfo> {
        let info = self.available_update()?;
        if self.banner_dismissed_tag.as_deref() == Some(info.tag_name.as_str()) {
            None
        } else {
            Some(info)
        }
    }

    pub fn start_check(&mut self, manual: bool, repaint_ctx: Option<egui::Context>) {
        if self.is_checking() {
            return;
        }
        self.state = UpdateCheckState::Checking;
        self.last_check_was_manual = manual;

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

        std::thread::spawn(move || {
            let res = fetch_latest_release();
            let _ = tx.send(res);
            if let Some(ctx) = repaint_ctx {
                ctx.request_repaint();
            }
        });
    }

    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(info)) => {
                self.rx = None;
                if is_newer_version(APP_VERSION, &info.version) {
                    self.state = UpdateCheckState::UpdateAvailable(info);
                } else {
                    self.state = UpdateCheckState::UpToDate {
                        latest_tag: info.tag_name,
                    };
                }
                true
            }
            Ok(Err(err)) => {
                self.rx = None;
                if self.last_check_was_manual {
                    self.state = UpdateCheckState::Error(err);
                } else {
                    // Silent failure on automatic background startup check (e.g. offline/intranet AWS)
                    self.state = UpdateCheckState::Idle;
                }
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                if self.last_check_was_manual {
                    self.state = UpdateCheckState::Error("检查更新线程意外中止".to_string());
                } else {
                    self.state = UpdateCheckState::Idle;
                }
                true
            }
        }
    }
}

#[derive(Deserialize)]
struct GitHubLatestRelease {
    tag_name: String,
    html_url: Option<String>,
    published_at: Option<String>,
}

fn build_http_agent(max_redirects: u32) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::NativeTls)
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();

    let cfg = ureq::Agent::config_builder()
        .tls_config(tls)
        .timeout_global(Some(Duration::from_secs(8)))
        .http_status_as_error(false)
        .max_redirects(max_redirects)
        .user_agent(format!("sas-dataset-viewer/{APP_VERSION}"))
        .build();

    ureq::Agent::new_with_config(cfg)
}

/// Fetch latest release from GitHub Releases API, automatically falling back to the rate-limit-free
/// `/releases/latest` HTTP redirect if the unauthenticated API quota is exhausted (403/429).
pub fn fetch_latest_release() -> Result<ReleaseInfo, String> {
    let api_err = match fetch_via_github_api() {
        Ok(info) => return Ok(info),
        Err(e) => e,
    };

    match fetch_via_redirect_fallback() {
        Ok(info) => Ok(info),
        Err(_) => Err(api_err),
    }
}

fn fetch_via_github_api() -> Result<ReleaseInfo, String> {
    let agent = build_http_agent(5);
    let mut resp = agent
        .get(LATEST_API_URL)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("无法连接 GitHub（请检查网络或代理）：{e}"))?;

    let status = resp.status().as_u16();
    if status != 200 {
        return Err(format!("GitHub API 返回状态码 {status}"));
    }

    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("读取更新信息失败：{e}"))?;

    let parsed: GitHubLatestRelease =
        serde_json::from_str(&body).map_err(|e| format!("解析发布信息失败：{e}"))?;

    let tag = parsed.tag_name.trim().to_string();
    let version = normalize_version(&tag).to_string();
    if version.is_empty() {
        return Err("未获取到有效版本号".to_string());
    }
    let html_url = parsed
        .html_url
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| format!("{RELEASES_URL}/tag/{tag}"));
    let published_at = parsed
        .published_at
        .map(|s| s.split('T').next().unwrap_or(&s).to_string());

    Ok(ReleaseInfo {
        tag_name: tag,
        version,
        html_url,
        published_at,
    })
}

fn fetch_via_redirect_fallback() -> Result<ReleaseInfo, String> {
    let agent = build_http_agent(0);
    let resp = agent
        .get(LATEST_PAGE_URL)
        .call()
        .map_err(|e| format!("连接 GitHub Releases 失败：{e}"))?;

    let status = resp.status().as_u16();
    if !(300..400).contains(&status) {
        return Err(format!("Releases 页面未返回重定向（状态码 {status}）"));
    }
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "Releases 响应缺少重定向地址".to_string())?;

    let tag = extract_tag_from_release_url(location)
        .ok_or_else(|| format!("无法从地址解析版本号：{location}"))?;
    let version = normalize_version(&tag).to_string();
    let html_url = if location.starts_with("http://") || location.starts_with("https://") {
        location.to_string()
    } else {
        format!("{RELEASES_URL}/tag/{tag}")
    };

    Ok(ReleaseInfo {
        tag_name: tag,
        version,
        html_url,
        published_at: None,
    })
}

pub fn extract_tag_from_release_url(url: &str) -> Option<String> {
    let marker = "/releases/tag/";
    let idx = url.find(marker)?;
    let tail = &url[idx + marker.len()..];
    let tag = tail
        .split(['?', '#', '/'])
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some(tag.to_string())
}

pub fn normalize_version(raw: &str) -> &str {
    raw.trim()
        .strip_prefix('v')
        .or_else(|| raw.trim().strip_prefix('V'))
        .unwrap_or_else(|| raw.trim())
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedVersion {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

fn parse_version(raw: &str) -> Option<ParsedVersion> {
    let s = normalize_version(raw);
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) => (c, Some(p.split('+').next().unwrap_or(p).to_string())),
        None => (s.split('+').next().unwrap_or(s), None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().and_then(|p| p.parse::<u64>().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| p.parse::<u64>().ok()).unwrap_or(0);
    Some(ParsedVersion {
        major,
        minor,
        patch,
        pre,
    })
}

/// Returns `true` strictly when `latest` represents a newer semantic version than `current`.
pub fn is_newer_version(current: &str, latest: &str) -> bool {
    let (Some(cur), Some(lat)) = (parse_version(current), parse_version(latest)) else {
        return false;
    };
    let cur_tuple = (cur.major, cur.minor, cur.patch);
    let lat_tuple = (lat.major, lat.minor, lat.patch);
    if lat_tuple != cur_tuple {
        return lat_tuple > cur_tuple;
    }
    // Same major.minor.patch: stable release > prerelease
    match (cur.pre, lat.pre) {
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (Some(c), Some(l)) => l > c,
        (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_handles_prefixes_and_semver() {
        assert!(is_newer_version("0.3.2", "v0.3.3"));
        assert!(is_newer_version("0.3.2", "V0.3.10"));
        assert!(is_newer_version("0.3.2", "0.4.0"));
        assert!(is_newer_version("0.3.2", "v1.0.0"));
        assert!(is_newer_version("0.3.2-beta.1", "v0.3.2"));

        assert!(!is_newer_version("0.3.2", "v0.3.2"));
        assert!(!is_newer_version("0.3.2", "v0.3.1"));
        assert!(!is_newer_version("0.3.2", "0.2.9"));
        assert!(!is_newer_version("0.3.2", "v0.3.2-rc.1"));
        assert!(!is_newer_version("0.3.2", "not-a-version"));
    }

    #[test]
    fn extract_tag_from_redirect_location() {
        assert_eq!(
            extract_tag_from_release_url(
                "https://github.com/allensrj/sas-dataset-viewer/releases/tag/v0.3.2"
            ),
            Some("v0.3.2".to_string())
        );
        assert_eq!(
            extract_tag_from_release_url(
                "https://github.com/allensrj/sas-dataset-viewer/releases/tag/V0.4.0?foo=1"
            ),
            Some("V0.4.0".to_string())
        );
        assert_eq!(
            extract_tag_from_release_url(
                "https://github.com/allensrj/sas-dataset-viewer/releases"
            ),
            None
        );
    }

    #[test]
    #[ignore]
    fn live_github_release_check() {
        let api = fetch_via_github_api().expect("GitHub API latest release check should succeed");
        println!("API release: {:?}", api);
        assert!(!api.tag_name.is_empty());
        let redir = fetch_via_redirect_fallback()
            .expect("GitHub redirect fallback latest release check should succeed");
        println!("Redirect release: {:?}", redir);
        assert_eq!(api.tag_name, redir.tag_name);
    }
}
