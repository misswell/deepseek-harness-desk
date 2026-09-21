use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering as VersionOrdering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
#[cfg(not(target_os = "macos"))]
use tauri_plugin_notification::NotificationExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_foundation::{ns_string, NSObjectNSKeyValueCoding, NSString};
#[cfg(target_os = "macos")]
use objc2_web_kit::WKWebViewConfiguration;

use tauri::menu::{MenuBuilder, MenuItem, MenuItemKind, SubmenuBuilder};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::utils::Theme;
use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{
    AppHandle, Emitter, Manager, RunEvent, State, Url, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

const PORT_START: u16 = 3080;
const PORT_END: u16 = 3099;
const MAX_LOG_LINES: usize = 500;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const NODE_VERSION: &str = "24.19.0";
const DSH_VERSION: &str = "0.1.0-rc.6";
const DSH_PACKAGE: &str = "@deepseek-ai/dsh";
const RELEASES_URL: &str =
    "https://api.github.com/repos/misswell/deepseek-harness-desk/releases/latest";
const RELEASES_PAGE_URL: &str = "https://github.com/misswell/deepseek-harness-desk/releases";
const NPM_METADATA_URL: &str = "https://registry.npmjs.org/@deepseek-ai%2fdsh";

fn macos_proxy_url_from_scutil(output: &str, scheme: &str) -> Option<String> {
    let prefix = if scheme.eq_ignore_ascii_case("https") {
        "HTTPS"
    } else {
        "HTTP"
    };
    let value = |key: &str| {
        output.lines().find_map(|line| {
            let (candidate, value) = line.trim().split_once(" : ")?;
            (candidate == key).then(|| value.trim())
        })
    };

    if value(&format!("{prefix}Enable"))? != "1" {
        return None;
    }
    let host = value(&format!("{prefix}Proxy"))?;
    let port = value(&format!("{prefix}Port"))?.parse::<u16>().ok()?;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    Some(format!("http://{host}:{port}"))
}

#[cfg(target_os = "macos")]
fn macos_system_proxy_output() -> Option<String> {
    let output = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn outbound_http_client(
    timeout: Duration,
    user_agent: &str,
    http1_only: bool,
) -> Result<reqwest::Client, reqwest::Error> {
    let mut builder = reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(user_agent);
    if http1_only {
        builder = builder.http1_only();
    }

    #[cfg(target_os = "macos")]
    if let Some(settings) = macos_system_proxy_output() {
        if let Some(proxy_url) = macos_proxy_url_from_scutil(&settings, "http") {
            builder = builder.proxy(reqwest::Proxy::http(proxy_url)?);
        }
        if let Some(proxy_url) = macos_proxy_url_from_scutil(&settings, "https") {
            builder = builder.proxy(reqwest::Proxy::https(proxy_url)?);
        }
    }

    builder.build()
}

#[derive(Clone)]
struct HarnessState {
    lifecycle: Arc<Mutex<()>>,
    child: Arc<Mutex<Option<Child>>>,
    port: Arc<AtomicU16>,
    // The browser-facing port is a loopback proxy. It forwards to the dsh
    // port while attaching the native token-auth cookie to every request, so
    // WebKit never has to send that cookie from a third-party iframe.
    proxy_port: Arc<AtomicU16>,
    proxy_stop: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    // Launch token printed by token-authenticated dsh releases (0.1.2+). The
    // embedded WebView and the event watcher both need it to log in.
    token: Arc<Mutex<Option<String>>>,
    // "name=value" auth cookie minted by dsh after the token exchange; sent
    // with event-watcher WebSocket requests.
    auth_cookie: Arc<Mutex<Option<String>>>,
    memory_saver: Arc<AtomicBool>,
    keep_alive_after_window_destroy: Arc<AtomicBool>,
    main_window_recreating: Arc<AtomicBool>,
    dsh_path: Arc<Mutex<Option<PathBuf>>>,
    logs: Arc<Mutex<VecDeque<HarnessLog>>>,
    last_error: Arc<Mutex<Option<String>>>,
    last_exit_code: Arc<Mutex<Option<i32>>>,
    // Unix-millis timestamp of the most recent backend event frame received by
    // the task watcher; 0 when no event has been seen yet. The shell uses this
    // to decide when the Harness has been idle long enough to safely recycle
    // the WebView page and reclaim the renderer's accumulated memory.
    last_event_at: Arc<AtomicI64>,
    runtime_installing: Arc<AtomicBool>,
    runtime_message: Arc<Mutex<String>>,
    app_update_installing: Arc<AtomicBool>,
    notify_enabled: Arc<AtomicBool>,
    notify_task_completed: Arc<AtomicBool>,
    notify_interaction: Arc<AtomicBool>,
    notify_error: Arc<AtomicBool>,
    // Whether a banner may quote Harness payload text (question wording, failure
    // message). Off by default: banners are readable from the lock screen.
    notify_detail: Arc<AtomicBool>,
    pending_attention: Arc<AtomicI64>,
    language: Arc<Mutex<String>>,
}

#[derive(Serialize, Clone, Debug)]
struct NotificationPrefsView {
    enabled: bool,
    task_completed: bool,
    interaction: bool,
    error: bool,
    detail: bool,
}

#[derive(Serialize, Clone, Debug)]
struct HarnessLog {
    stream: String,
    message: String,
}

#[derive(Serialize, Clone, Debug)]
struct HarnessStatus {
    running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dsh_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_event_at: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
struct RuntimeStatus {
    available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    node_version: String,
    runtime_root: String,
    logs_directory: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    installing: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    message: String,
}

#[derive(Serialize, Clone, Debug)]
struct DshUpdateStatus {
    managed: bool,
    current_version: Option<String>,
    latest_version: Option<String>,
    available: bool,
    status: String,
    /// Update channel the check ran on: `"stable"` (npm `latest`/`next`) or
    /// `"preview"` (additionally `alpha`/`beta`).
    channel: String,
    /// True when the offered version is a preview build.
    preview: bool,
    /// Newest preview build published on npm, when it is newer than what is
    /// installed. Lets the settings page point at a beta without installing it.
    preview_version: Option<String>,
    /// True when the active managed version is a preview build.
    current_is_preview: bool,
    /// Version the launcher is pinned to, when the user chose one instead of
    /// following the newest installed build.
    pinned_version: Option<String>,
    /// Newest release candidate on npm, independent of the channel. Lets the
    /// settings page offer "回到稳定版" while running a preview build.
    stable_version: Option<String>,
}

/// One selectable version in the settings page: every version published on npm
/// plus any locally installed one, flagged so the shell can label it.
#[derive(Serialize, Clone, Debug)]
struct DshVersionOption {
    version: String,
    preview: bool,
    installed: bool,
    active: bool,
}

#[derive(Serialize, Clone, Debug)]
struct DshVersionsStatus {
    active_version: Option<String>,
    pinned_version: Option<String>,
    latest_stable: Option<String>,
    latest_preview: Option<String>,
    /// Newest first.
    versions: Vec<DshVersionOption>,
    /// False when npm was unreachable, in which case `versions` only lists the
    /// locally installed builds.
    npm_reachable: bool,
}

#[derive(Serialize, Clone, Debug)]
struct AppUpdateStatus {
    current_version: String,
    latest_version: Option<String>,
    available: bool,
    status: String,
    release_url: Option<String>,
    download_url: Option<String>,
    asset_name: Option<String>,
    notes: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
struct UpdateProgress {
    message: String,
    fraction: Option<f64>,
    done: bool,
    error: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize, Clone, Debug)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    size: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct NpmMetadata {
    #[serde(rename = "dist-tags")]
    dist_tags: NpmDistTags,
    /// Every published version, used by the settings page to offer downgrades.
    versions: Option<HashMap<String, NpmVersionEntry>>,
}

#[derive(Deserialize, Debug)]
struct NpmDistTags {
    latest: Option<String>,
    next: Option<String>,
    /// Preview builds. The harness team publishes early builds under `alpha`
    /// (and would use `beta` if it ever splits the stream); neither is ever
    /// picked unless the user opts into the preview channel.
    alpha: Option<String>,
    beta: Option<String>,
}

/// Placeholder for one `versions` entry. Only the version keys matter here, so
/// the (large) per-version manifests are parsed into nothing.
#[derive(Deserialize, Debug)]
struct NpmVersionEntry {}

#[derive(Serialize, Clone, Debug)]
struct RuntimeProgress {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fraction: Option<f64>,
    done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Clone, Debug)]
struct DshCommand {
    program: PathBuf,
}

fn is_port_available(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

fn first_available_port(start: u16, end: u16) -> Option<u16> {
    (start..=end).find(|port| is_port_available(*port))
}

fn first_available_port_excluding(start: u16, end: u16, excluded: u16) -> Option<u16> {
    (start..=end).find(|port| *port != excluded && is_port_available(*port))
}

fn version_parts(value: &str) -> (Vec<u64>, Vec<String>) {
    let normalized = value.trim().trim_start_matches(['v', 'V']);
    let mut parts = normalized.splitn(2, '-');
    let core = parts
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse::<u64>().unwrap_or(0))
        .collect::<Vec<_>>();
    let prerelease = parts
        .next()
        .unwrap_or_default()
        .split('.')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (core, prerelease)
}

fn compare_versions(candidate: &str, current: &str) -> VersionOrdering {
    let (candidate_core, candidate_pre) = version_parts(candidate);
    let (current_core, current_pre) = version_parts(current);
    let core_len = candidate_core.len().max(current_core.len());
    for index in 0..core_len {
        let candidate_value = candidate_core.get(index).copied().unwrap_or(0);
        let current_value = current_core.get(index).copied().unwrap_or(0);
        match candidate_value.cmp(&current_value) {
            VersionOrdering::Equal => {}
            ordering => return ordering,
        }
    }

    match (candidate_pre.is_empty(), current_pre.is_empty()) {
        (true, false) => return VersionOrdering::Greater,
        (false, true) => return VersionOrdering::Less,
        _ => {}
    }
    for index in 0..candidate_pre.len().max(current_pre.len()) {
        let Some(candidate_value) = candidate_pre.get(index) else {
            return VersionOrdering::Less;
        };
        let Some(current_value) = current_pre.get(index) else {
            return VersionOrdering::Greater;
        };
        match (candidate_value.parse::<u64>(), current_value.parse::<u64>()) {
            (Ok(candidate_number), Ok(current_number)) => {
                match candidate_number.cmp(&current_number) {
                    VersionOrdering::Equal => {}
                    ordering => return ordering,
                }
            }
            (Ok(_), Err(_)) => return VersionOrdering::Less,
            (Err(_), Ok(_)) => return VersionOrdering::Greater,
            (Err(_), Err(_)) => match candidate_value.cmp(current_value) {
                VersionOrdering::Equal => {}
                ordering => return ordering,
            },
        }
    }
    VersionOrdering::Equal
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    compare_versions(candidate, current) == VersionOrdering::Greater
}

/// Pick the newest version among the npm dist-tags the harness team
/// publishes. New release candidates are tagged `next` first while `latest`
/// still points at the previous one (e.g. `0.1.0-rc.8` published as `next`
/// while `latest` is still `0.1.0-rc.7`); reading only `latest` would miss a
/// brand-new rc. This returns the newer of the two, or whichever is present.
fn newest_published_version(latest: Option<&str>, next: Option<&str>) -> Option<String> {
    newest_of_published_versions(&[latest, next])
}

/// Return the newest non-empty version among the given npm dist-tag values.
fn newest_of_published_versions(candidates: &[Option<&str>]) -> Option<String> {
    let mut newest: Option<String> = None;
    for candidate in candidates.iter().flatten() {
        let candidate = candidate.trim();
        if candidate.is_empty() {
            continue;
        }
        if newest
            .as_deref()
            .is_none_or(|current| is_newer_version(candidate, current))
        {
            newest = Some(candidate.to_string());
        }
    }
    newest
}

/// Which npm dist-tags the bundled dsh updater may install.
///
/// `Stable` follows the release candidates tagged `latest`/`next`, which is
/// what the harness team ships as its normal release stream. `Preview`
/// additionally considers `alpha`/`beta`, so early builds can be checked and
/// installed on purpose instead of by accident.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DshUpdateChannel {
    Stable,
    Preview,
}

impl DshUpdateChannel {
    /// Parse the channel sent by the shell. Unknown or missing values fall back
    /// to `Stable`: a malformed request must never opt the user into previews.
    fn from_request(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some(value)
                if value.eq_ignore_ascii_case("preview")
                    || value.eq_ignore_ascii_case("beta")
                    || value.eq_ignore_ascii_case("alpha") =>
            {
                Self::Preview
            }
            _ => Self::Stable,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Preview => "preview",
        }
    }
}

/// True for managed version directories that only exist because the preview
/// channel was used (`0.1.6-alpha.2`, `0.1.6-beta.1`, …). Release candidates
/// (`0.1.5-rc.2`) belong to the stable channel. The prerelease label has to
/// match exactly so a version such as `0.1.6-alphabet.1` is not mistaken for a
/// preview build.
fn is_preview_dsh_version(version: &str) -> bool {
    let (_, prerelease) = version_parts(version);
    prerelease
        .iter()
        .any(|part| matches!(part.to_ascii_lowercase().as_str(), "alpha" | "beta"))
}

fn is_safe_package_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() < 128
        && !version.contains('/')
        && !version.contains('\\')
        && !version.contains("..")
        && !version.chars().any(char::is_whitespace)
}

fn home_directory() -> Option<PathBuf> {
    if let Some(home) = env::var_os("HOME") {
        return Some(PathBuf::from(home));
    }

    #[cfg(windows)]
    {
        if let Some(home) = env::var_os("USERPROFILE") {
            return Some(PathBuf::from(home));
        }
        if let (Some(drive), Some(path)) = (env::var_os("HOMEDRIVE"), env::var_os("HOMEPATH")) {
            return Some(PathBuf::from(drive).join(path));
        }
    }

    None
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn managed_dsh_version_paths(app: Option<&AppHandle>) -> Vec<(String, PathBuf)> {
    let mut versions = Vec::new();
    for runtime_root in runtime_roots(app) {
        let runtime_dsh = runtime_root.join("dsh");
        let Ok(entries) = fs::read_dir(runtime_dsh) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(version) = entry.file_name().to_str() {
                versions.push((version.to_string(), path));
            }
        }
    }
    versions.sort_by(|(left_version, _), (right_version, _)| {
        compare_versions(right_version, left_version)
    });
    versions
}

/// File inside `<runtime>/dsh/` that records the version the user pinned.
/// Without it the launcher always runs the newest managed version, which makes
/// a downgrade impossible once a newer build is on disk.
const DSH_PIN_FILE: &str = ".active-version";

/// Upper bound on the versions the settings page offers. The registry holds a
/// couple of dozen builds; the cap only guards against a pathological list.
const MAX_LISTED_DSH_VERSIONS: usize = 60;

/// Read a pinned version out of a pin file's contents. Only a plain version
/// string is accepted; anything else (a corrupt or handwritten file) is
/// ignored so the launcher falls back to the newest installed version.
fn pinned_version_from_contents(contents: &str) -> Option<String> {
    let version = contents.trim();
    // A real version always starts with a digit; that keeps a stray word (or a
    // dist-tag name) in the file from being treated as an installed version.
    let looks_like_version = version
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_digit());
    if looks_like_version && is_safe_package_version(version) {
        Some(version.to_string())
    } else {
        None
    }
}

/// The version pinned by the user, if any runtime root records one.
fn pinned_dsh_version(app: Option<&AppHandle>) -> Option<String> {
    for runtime_root in runtime_roots(app) {
        let Ok(contents) = fs::read_to_string(runtime_root.join("dsh").join(DSH_PIN_FILE)) else {
            continue;
        };
        if let Some(version) = pinned_version_from_contents(&contents) {
            return Some(version);
        }
    }
    None
}

/// Persist (or clear) the pinned version. `None` removes every pin file so the
/// launcher goes back to following the newest installed version.
fn write_dsh_pin(app: &AppHandle, version: Option<&str>) -> Result<(), String> {
    let mut first_error = None;
    for runtime_root in runtime_roots(Some(app)) {
        let dsh_root = runtime_root.join("dsh");
        let path = dsh_root.join(DSH_PIN_FILE);
        let result = match version {
            Some(version) => {
                if !is_safe_package_version(version) {
                    return Err("dsh 版本号无效。".to_string());
                }
                fs::create_dir_all(&dsh_root)
                    .and_then(|()| fs::write(&path, format!("{version}\n")))
            }
            None => match fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            },
        };
        if let Err(error) = result {
            first_error.get_or_insert(format!("写入内置 dsh 版本固定信息失败：{error}"));
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Pick the version the launcher must use out of the installed ones.
///
/// `installed` is newest-first and flags which entries have a usable `dsh`
/// binary. A pinned version wins as long as it is actually installed and
/// usable; otherwise the newest usable one is used, so a stale pin can never
/// leave the app without a runnable dsh.
fn select_active_dsh_version(installed: &[(String, bool)], pinned: Option<&str>) -> Option<String> {
    if let Some(pinned) = pinned {
        if installed
            .iter()
            .any(|(version, usable)| version == pinned && *usable)
        {
            return Some(pinned.to_string());
        }
    }
    installed
        .iter()
        .find(|(_, usable)| *usable)
        .map(|(version, _)| version.clone())
}

/// Version and directory of the dsh the app should run right now.
fn active_dsh_version_path(app: Option<&AppHandle>) -> Option<(String, PathBuf)> {
    let versions = managed_dsh_version_paths(app);
    let installed = versions
        .iter()
        .map(|(version, path)| (version.clone(), is_executable(&dsh_executable(path))))
        .collect::<Vec<_>>();
    let active = select_active_dsh_version(&installed, pinned_dsh_version(app).as_deref())?;
    versions.into_iter().find(|(version, _)| version == &active)
}

fn managed_dsh_version(app: Option<&AppHandle>) -> Option<String> {
    active_dsh_version_path(app).map(|(version, _)| version)
}

fn managed_node_root(app: Option<&AppHandle>) -> Option<PathBuf> {
    let mut versions = Vec::new();
    for runtime_root in runtime_roots(app) {
        let runtime_node = runtime_root.join("node");
        let Ok(entries) = fs::read_dir(runtime_node) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(version) = entry.file_name().to_str() {
                    versions.push((version.to_string(), path));
                }
            }
        }
    }
    versions.sort_by(|(left_version, _), (right_version, _)| {
        compare_versions(right_version, left_version)
    });
    versions
        .into_iter()
        .find(|(_, path)| is_executable(&node_executable(path)))
        .map(|(_, path)| path)
}

/// Quote a path for a POSIX shell single-quoted string.
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Expose the managed `dsh` CLI to the user's shell.
///
/// The dsh launcher script starts with `#!/usr/bin/env node`, so running it
/// directly from a terminal requires a system Node.js that may be missing or
/// a different version than the one the App manages. This writes a small
/// wrapper into `~/.local/bin/dsh` (which is on the default PATH for most
/// Unix shells) that invokes the bundled runtime Node against the newest
/// managed dsh package, so `dsh` works in the terminal without extra setup.
///
/// No-op when the dsh in use comes from `DSH_BIN` or the system PATH (it is
/// already reachable), or on Windows.
fn link_dsh_to_path(app: &AppHandle) {
    #[cfg(unix)]
    {
        // Only create the wrapper when the active dsh is one we manage; a
        // user-provided dsh (DSH_BIN / system PATH) is already reachable.
        let Some((_, dsh_root)) = active_dsh_version_path(Some(app)) else {
            return;
        };
        let Some(node_root) = managed_node_root(Some(app)) else {
            return;
        };
        let node = node_executable(&node_root);
        let script = dsh_executable(&dsh_root);

        let Some(home) = home_directory() else {
            return;
        };
        let bin_dir = home.join(".local/bin");
        if !bin_dir.is_dir() && fs::create_dir_all(&bin_dir).is_err() {
            return;
        }

        let wrapper = format!(
            "#!/bin/sh\nexec {} {} \"$@\"\n",
            shell_single_quote(&node.to_string_lossy()),
            shell_single_quote(&script.to_string_lossy()),
        );
        let target = bin_dir.join("dsh");
        let temp = bin_dir.join(".dsh-wrapper.tmp");
        if fs::write(&temp, wrapper).is_err() {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(0o755));
        }
        let _ = fs::rename(&temp, &target);
    }
    #[cfg(not(unix))]
    {
        let _ = app;
    }
}

fn legacy_runtime_root() -> Option<PathBuf> {
    home_directory().map(|home| {
        if cfg!(target_os = "macos") {
            home.join("Library/Application Support/DeepSeek Harness Desk/runtime")
        } else if cfg!(windows) {
            env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData/Roaming"))
                .join("DeepSeek Harness Desk/runtime")
        } else {
            home.join(".local/share/DeepSeek Harness Desk/runtime")
        }
    })
}

fn application_runtime_root(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|path| path.join("runtime"))
}

fn runtime_roots(app: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = legacy_runtime_root() {
        push_unique(&mut roots, root);
    }
    if let Some(app) = app {
        if let Some(root) = application_runtime_root(app) {
            push_unique(&mut roots, root);
        }
    }
    roots
}

fn preferred_runtime_root(app: &AppHandle) -> PathBuf {
    let roots = runtime_roots(Some(app));
    roots
        .iter()
        .find(|root| root.exists())
        .cloned()
        .or_else(|| roots.first().cloned())
        .or_else(|| home_directory().map(|home| home.join(".deepseek-harness-desk/runtime")))
        .unwrap_or_else(|| PathBuf::from("runtime"))
}

fn dsh_candidates(app: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(value) = env::var_os("DSH_BIN") {
        let explicit = PathBuf::from(value);
        if explicit.is_absolute()
            || explicit
                .parent()
                .is_some_and(|parent| !parent.as_os_str().is_empty())
        {
            push_unique(&mut candidates, explicit);
        }
    }

    #[cfg(windows)]
    let names = ["dsh", "dsh.exe", "dsh.cmd", "dsh.bat"]
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    #[cfg(not(windows))]
    let names = vec!["dsh".to_string()];

    // Reuse both the runtime created by the old Swift client and the Tauri
    // app-data runtime created by the first-run installer. The version the user
    // chose (pinned, else the newest managed one) comes first, and every other
    // installed version stays as a fallback.
    if let Some((_, version_dir)) = active_dsh_version_path(app) {
        for name in &names {
            push_unique(
                &mut candidates,
                version_dir.join("node_modules/.bin").join(name),
            );
        }
    }
    for (_, version_dir) in managed_dsh_version_paths(app) {
        for name in &names {
            push_unique(
                &mut candidates,
                version_dir.join("node_modules/.bin").join(name),
            );
        }
    }

    let mut search_directories = Vec::new();
    if let Some(path) = env::var_os("PATH") {
        search_directories.extend(env::split_paths(&path));
    }

    if let Some(home) = home_directory() {
        search_directories.extend([
            home.join(".local/bin"),
            home.join("bin"),
            home.join(".npm-global/bin"),
            home.join(".volta/bin"),
            home.join("Library/pnpm"),
        ]);

        #[cfg(windows)]
        {
            if let Some(app_data) = env::var_os("APPDATA") {
                search_directories.push(PathBuf::from(app_data).join("npm"));
            }
        }
    }

    #[cfg(unix)]
    search_directories.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ]);

    for directory in search_directories {
        for name in &names {
            push_unique(&mut candidates, directory.join(name));
        }
    }

    candidates
}

fn managed_node_bin_directories(app: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    for runtime_root in runtime_roots(app) {
        let runtime_node = runtime_root.join("node");
        if let Ok(versions) = fs::read_dir(runtime_node) {
            for version in versions.flatten() {
                let bin = if cfg!(windows) {
                    version.path()
                } else {
                    version.path().join("bin")
                };
                if bin.is_dir() {
                    directories.push(bin);
                }
            }
        }
    }
    directories
}

fn dsh_command(app: &AppHandle) -> Option<DshCommand> {
    dsh_candidates(Some(app))
        .into_iter()
        .find(|candidate| is_executable(candidate))
        .map(|program| DshCommand { program })
}

// `--no-open` was added to the web profile after the first bundled dsh
// release. Older versions never opened a browser, so omit the flag for them
// instead of making the Harness fail on an unknown option.
fn dsh_supports_no_open(version: &str) -> bool {
    compare_versions(version, "0.1.0-rc.8") != VersionOrdering::Less
}

fn dsh_supports_no_open_for_command(command: &DshCommand, app: &AppHandle) -> bool {
    managed_dsh_version_paths(Some(app))
        .into_iter()
        .find(|(_, path)| dsh_executable(path) == command.program)
        .map(|(version, _)| dsh_supports_no_open(&version))
        .unwrap_or(true)
}

fn is_managed_harness_command(
    command: &str,
    executable_path: &Path,
    port_start: u16,
    port_end: u16,
) -> bool {
    // dsh used the positional `web` profile in older releases, while newer
    // releases use the canonical `--profile web` form. Recognize both shapes
    // so orphan cleanup keeps working across an in-place dsh update.
    let executable = executable_path.to_string_lossy();
    let legacy_marker = format!("{executable} web --port ");
    let profile_marker = format!("{executable} --profile web --port ");
    let (marker, marker_start) = if let Some(start) = command.find(&profile_marker) {
        (profile_marker.as_str(), start)
    } else if let Some(start) = command.find(&legacy_marker) {
        (legacy_marker.as_str(), start)
    } else {
        return false;
    };
    if marker_start > 0
        && command[..marker_start]
            .chars()
            .last()
            .is_some_and(|character| !character.is_whitespace())
    {
        return false;
    }

    let arguments = &command[marker_start + marker.len()..];
    let Some(port_text) = arguments.split_whitespace().next() else {
        return false;
    };
    let Ok(port) = port_text.parse::<u16>() else {
        return false;
    };
    let trailing_arguments = arguments[port_text.len()..].trim();
    (port_start..=port_end).contains(&port) && matches!(trailing_arguments, "" | "--no-open")
}

fn orphaned_managed_harness_process_ids(app: &AppHandle) -> Vec<u32> {
    let executable_paths = managed_dsh_version_paths(Some(app))
        .into_iter()
        .map(|(_, path)| dsh_executable(&path))
        .collect::<Vec<_>>();
    if executable_paths.is_empty() {
        return Vec::new();
    }

    let Ok(output) = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
    else {
        return Vec::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let fields = line.trim_start();
            let pid_text = fields.split_whitespace().next()?;
            let after_pid = fields.strip_prefix(pid_text)?.trim_start();
            let parent_pid_text = after_pid.split_whitespace().next()?;
            let command = after_pid.strip_prefix(parent_pid_text)?.trim_start();
            let pid = pid_text.parse::<u32>().ok()?;
            let parent_pid = parent_pid_text.parse::<u32>().ok()?;

            // A process still owned by the current App or a manually launched
            // dsh is not an orphan. PPID 1 identifies leftovers from a
            // previous App instance after macOS has re-parented them.
            if parent_pid != 1 {
                return None;
            }
            executable_paths
                .iter()
                .any(|path| is_managed_harness_command(command, path, PORT_START, PORT_END))
                .then_some(pid)
        })
        .collect()
}

fn process_is_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn terminate_orphaned_harness_process(pid: u32) {
    let pid_text = pid.to_string();
    let process_group = format!("-{pid}");
    let _ = Command::new("/bin/kill")
        .args(["-TERM", &process_group])
        .status();
    let _ = Command::new("/bin/kill")
        .args(["-TERM", &pid_text])
        .status();

    let deadline = Instant::now() + Duration::from_secs(2);
    while process_is_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    if process_is_alive(pid) {
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &process_group])
            .status();
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &pid_text])
            .status();
    }
}

fn reap_orphaned_managed_harnesses(app: &AppHandle) -> Vec<u32> {
    let process_ids = orphaned_managed_harness_process_ids(app);
    for pid in &process_ids {
        terminate_orphaned_harness_process(*pid);
    }
    process_ids
}

fn process_path(program: &Path, app: &AppHandle) -> String {
    let mut paths = Vec::new();
    if let Some(parent) = program.parent() {
        paths.push(parent.to_path_buf());
    }
    paths.extend(managed_node_bin_directories(Some(app)));
    if let Some(path) = env::var_os("PATH") {
        paths.extend(env::split_paths(&path));
    }
    if let Some(home) = home_directory() {
        paths.extend([
            home.join(".local/bin"),
            home.join(".volta/bin"),
            home.join("Library/pnpm"),
        ]);
    }
    #[cfg(unix)]
    paths.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ]);

    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.to_string_lossy().into_owned()));
    env::join_paths(paths)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn dsh_web_arguments(port: u16, no_open_supported: bool) -> Vec<String> {
    let mut arguments = vec![
        "--profile".to_string(),
        "web".to_string(),
        "--port".to_string(),
        port.to_string(),
    ];
    if no_open_supported {
        arguments.push("--no-open".to_string());
    }
    arguments
}

fn spawn_dsh(command: &DshCommand, port: u16, app: &AppHandle) -> std::io::Result<Child> {
    let is_windows_script = cfg!(windows)
        && command
            .program
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(extension.to_ascii_lowercase().as_str(), "cmd" | "bat")
            });

    let mut process = if is_windows_script {
        let mut process = Command::new("cmd.exe");
        process.arg("/D").arg("/S").arg("/C").arg(&command.program);
        process
    } else {
        Command::new(&command.program)
    };

    // Cap the Harness backend's V8 heap so a long-lived session cannot balloon
    // to multiple GB of memory. 1 GB is ~5x the current steady-state usage.
    let node_options = env::var("NODE_OPTIONS")
        .map(|existing| format!("{existing} --max-old-space-size=1024"))
        .unwrap_or_else(|_| "--max-old-space-size=1024".to_string());

    let no_open_supported = dsh_supports_no_open_for_command(command, app);
    let arguments = dsh_web_arguments(port, no_open_supported);
    process
        .args(&arguments)
        .current_dir(home_directory().unwrap_or_else(|| PathBuf::from(".")))
        .env("PATH", process_path(&command.program, app))
        .env("NODE_OPTIONS", node_options)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Put dsh and the Node process it launches in their own group so a
        // stop/restart cannot leave an orphaned Harness server behind.
        process.process_group(0);
    }

    process.spawn()
}

fn runtime_status_snapshot(app: &AppHandle, state: &HarnessState) -> RuntimeStatus {
    let command = dsh_command(app);
    RuntimeStatus {
        available: command.is_some(),
        version: managed_dsh_version(Some(app)),
        node_version: NODE_VERSION.to_string(),
        runtime_root: preferred_runtime_root(app).to_string_lossy().into_owned(),
        logs_directory: logs_directory(app).to_string_lossy().into_owned(),
        path: command.map(|command| command.program.to_string_lossy().into_owned()),
        installing: state.runtime_installing.load(Ordering::Acquire),
        message: state
            .runtime_message
            .lock()
            .map(|message| message.clone())
            .unwrap_or_default(),
    }
}

fn emit_runtime_progress(
    app: &AppHandle,
    state: &HarnessState,
    message: impl Into<String>,
    fraction: Option<f64>,
    done: bool,
    error: Option<String>,
) {
    let message = message.into();
    if let Ok(mut current) = state.runtime_message.lock() {
        *current = message.clone();
    }
    let _ = app.emit(
        "runtime-progress",
        RuntimeProgress {
            message,
            fraction,
            done,
            error,
        },
    );
}

struct NodeDistribution {
    archive_name: String,
    extracted_directory: String,
}

fn node_distribution() -> Result<NodeDistribution, String> {
    #[cfg(target_os = "macos")]
    let platform = "darwin";
    #[cfg(target_os = "linux")]
    let platform = "linux";
    #[cfg(target_os = "windows")]
    let platform = "win";
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return Err("当前系统暂不支持自动安装 Node.js 运行时。".to_string());

    let architecture = match env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "x86",
        other => return Err(format!("当前 CPU 架构不支持自动安装 Node.js：{other}")),
    };

    let extension = if cfg!(target_os = "windows") {
        "zip"
    } else if cfg!(target_os = "linux") {
        "tar.xz"
    } else {
        "tar.gz"
    };
    let base = format!("node-v{NODE_VERSION}-{platform}-{architecture}");
    Ok(NodeDistribution {
        archive_name: format!("{base}.{extension}"),
        extracted_directory: base,
    })
}

async fn download_runtime_archive(
    app: &AppHandle,
    state: &HarnessState,
    url: &str,
    destination: &Path,
) -> Result<(), String> {
    for attempt in 0..2 {
        match download_runtime_archive_once(app, state, url, destination).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt == 0 && is_retryable_download_error(&error) => {
                let _ = fs::remove_file(destination);
                emit_runtime_progress(
                    app,
                    state,
                    "下载连接中断，正在重新尝试…",
                    Some(0.05),
                    false,
                    None,
                );
            }
            Err(error) => {
                let _ = fs::remove_file(destination);
                return Err(error);
            }
        }
    }

    let _ = fs::remove_file(destination);
    Err("下载 Node.js 失败：下载连接重试次数已用尽。".to_string())
}

async fn download_runtime_archive_once(
    app: &AppHandle,
    state: &HarnessState,
    url: &str,
    destination: &Path,
) -> Result<(), String> {
    let client = outbound_http_client(Duration::from_secs(300), "DeepSeek Harness Desk", true)
        .map_err(|error| format!("创建运行时下载客户端失败：{error}"))?;
    let response = client
        .get(url)
        .header("Accept-Encoding", "identity")
        .send()
        .await
        .map_err(|error| format!("下载 Node.js 失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("下载 Node.js 失败（HTTP {}）。", response.status()));
    }
    let total = response.content_length();
    let mut response = response;
    let mut file = fs::File::create(destination)
        .map_err(|error| format!("创建 Node.js 安装包文件失败：{error}"))?;
    let mut downloaded = 0_u64;
    let mut last_fraction = 0.05_f64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取 Node.js 下载内容失败：{error}"))?
    {
        file.write_all(&chunk)
            .map_err(|error| format!("保存 Node.js 安装包失败：{error}"))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if let Some(total) = total.filter(|total| *total > 0) {
            let fraction = 0.05 + (downloaded as f64 / total as f64).min(1.0) * 0.55;
            if fraction - last_fraction >= 0.02 || downloaded >= total {
                let percent = (fraction * 100.0).round() as u8;
                emit_runtime_progress(
                    app,
                    state,
                    format!("正在下载 Node.js… {percent}%"),
                    Some(fraction),
                    false,
                    None,
                );
                last_fraction = fraction;
            }
        }
    }
    file.flush()
        .map_err(|error| format!("保存 Node.js 安装包失败：{error}"))?;
    Ok(())
}

fn shell_literal(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn extract_node_archive(
    archive: &Path,
    extraction_directory: &Path,
    archive_name: &str,
) -> Result<(), String> {
    fs::create_dir_all(extraction_directory)
        .map_err(|error| format!("创建 Node.js 解压目录失败：{error}"))?;

    let output = if cfg!(target_os = "windows") {
        let script = format!(
            "$ErrorActionPreference='Stop'; Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
            shell_literal(archive),
            shell_literal(extraction_directory),
        );
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|error| format!("解压 Node.js 失败：{error}"))?
    } else {
        let flag = if archive_name.ends_with(".tar.xz") {
            "-xJf"
        } else {
            "-xzf"
        };
        Command::new("tar")
            .args([
                flag,
                &archive.to_string_lossy(),
                "-C",
                &extraction_directory.to_string_lossy(),
            ])
            .output()
            .map_err(|error| format!("解压 Node.js 失败：{error}"))?
    };

    if output.status.success() {
        return Ok(());
    }
    let details = String::from_utf8_lossy(&output.stderr)
        .trim()
        .chars()
        .take(1200)
        .collect::<String>();
    Err(if details.is_empty() {
        "解压 Node.js 失败。".to_string()
    } else {
        format!("解压 Node.js 失败：{details}")
    })
}

fn node_executable(node_root: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        node_root.join("node.exe")
    } else {
        node_root.join("bin/node")
    }
}

fn dsh_executable(dsh_root: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        dsh_root.join("node_modules/.bin/dsh.cmd")
    } else {
        dsh_root.join("node_modules/.bin/dsh")
    }
}

fn runtime_node_path(node_root: &Path) -> String {
    let node_bin = if cfg!(target_os = "windows") {
        node_root.to_path_buf()
    } else {
        node_root.join("bin")
    };
    let mut paths = vec![node_bin];
    if let Some(existing) = env::var_os("PATH") {
        paths.extend(env::split_paths(&existing));
    }
    env::join_paths(paths)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn windows_command_line(program: &Path, args: &[String]) -> String {
    let quote = |value: &str| format!("\"{}\"", value.replace('"', "\\\""));
    std::iter::once(quote(&program.to_string_lossy()))
        .chain(args.iter().map(|arg| quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn install_dsh_package(
    app: &AppHandle,
    state: &HarnessState,
    node_root: &Path,
    dsh_staging: &Path,
    version: &str,
) -> Result<(), String> {
    let npm = if cfg!(target_os = "windows") {
        node_root.join("npm.cmd")
    } else {
        node_root.join("bin/npm")
    };
    if !is_executable(&npm) {
        return Err("Node.js 安装完成，但没有找到 npm。".to_string());
    }

    // Keep the bundled runtime independent from the user's global npm cache.
    // A stale or root-owned ~/.npm cache can make an otherwise valid install
    // fail with EACCES/EEXIST while npm is renaming cache entries.
    let npm_cache = preferred_runtime_root(app).join("npm-cache");
    fs::create_dir_all(&npm_cache).map_err(|error| format!("创建 npm 缓存目录失败：{error}"))?;

    let args = vec![
        "install".to_string(),
        "--prefix".to_string(),
        dsh_staging.to_string_lossy().into_owned(),
        "--no-audit".to_string(),
        "--no-fund".to_string(),
        "--no-update-notifier".to_string(),
        "--no-package-lock".to_string(),
        format!("{DSH_PACKAGE}@{version}"),
    ];
    let mut command = if cfg!(target_os = "windows") {
        let mut command = Command::new("cmd.exe");
        command.args(["/D", "/S", "/C", &windows_command_line(&npm, &args)]);
        command
    } else {
        let mut command = Command::new(&npm);
        command.args(&args);
        command
    };
    let output = command
        .current_dir(dsh_staging)
        .env("PATH", runtime_node_path(node_root))
        .env("npm_config_cache", &npm_cache)
        .env("NPM_CONFIG_CACHE", &npm_cache)
        .output()
        .map_err(|error| format!("执行 npm 安装失败：{error}"))?;

    for (stream, bytes) in [("stdout", &output.stdout), ("stderr", &output.stderr)] {
        for line in String::from_utf8_lossy(bytes).lines() {
            let line = line.trim();
            if !line.is_empty() {
                emit_runtime_progress(app, state, format!("npm: {line}"), None, false, None);
            }
        }
        if !bytes.is_empty() {
            emit_log(
                app,
                &state.logs,
                &format!("runtime-{stream}"),
                String::from_utf8_lossy(bytes).trim().to_string(),
            );
        }
    }

    if output.status.success() {
        return Ok(());
    }
    let details = String::from_utf8_lossy(&output.stderr)
        .trim()
        .chars()
        .rev()
        .take(1800)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    Err(if details.is_empty() {
        format!(
            "npm 安装 DeepSeek Harness 失败（退出码 {:?}）。",
            output.status.code()
        )
    } else {
        format!("npm 安装 DeepSeek Harness 失败：{details}")
    })
}

async fn install_runtime_inner(app: &AppHandle, state: &HarnessState) -> Result<(), String> {
    if dsh_command(app).is_some() {
        return Ok(());
    }
    if state.runtime_installing.swap(true, Ordering::AcqRel) {
        return Err("运行时正在安装，请等待当前安装完成。".to_string());
    }

    let runtime_root = preferred_runtime_root(app);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let staging = runtime_root.join(format!(".staging-{}-{nonce}", std::process::id()));
    emit_runtime_progress(
        app,
        state,
        "开始安装内置 Node.js 和 DeepSeek Harness…",
        Some(0.0),
        false,
        None,
    );

    let result = async {
        let distribution = node_distribution()?;
        let archive = staging.join(&distribution.archive_name);
        let extraction = staging.join("node-extracted");
        let node_root = runtime_root.join("node").join(NODE_VERSION);
        let dsh_root = runtime_root.join("dsh").join(DSH_VERSION);

        fs::create_dir_all(&staging).map_err(|error| format!("创建运行时目录失败：{error}"))?;
        fs::create_dir_all(runtime_root.join("node"))
            .map_err(|error| format!("创建 Node.js 目录失败：{error}"))?;
        fs::create_dir_all(runtime_root.join("dsh"))
            .map_err(|error| format!("创建 Harness 目录失败：{error}"))?;

        if !is_executable(&node_executable(&node_root)) {
            emit_runtime_progress(
                app,
                state,
                format!("正在下载 Node.js {NODE_VERSION}…"),
                Some(0.05),
                false,
                None,
            );
            let url = format!(
                "https://nodejs.org/dist/v{NODE_VERSION}/{}",
                distribution.archive_name
            );
            download_runtime_archive(app, state, &url, &archive).await?;
            emit_runtime_progress(app, state, "正在解压 Node.js…", Some(0.68), false, None);
            extract_node_archive(&archive, &extraction, &distribution.archive_name)?;
            let extracted_root = extraction.join(&distribution.extracted_directory);
            if !extracted_root.is_dir() {
                return Err("Node.js 安装包内容不完整。".to_string());
            }
            if node_root.exists() {
                fs::remove_dir_all(&node_root)
                    .map_err(|error| format!("替换 Node.js 运行时失败：{error}"))?;
            }
            fs::rename(&extracted_root, &node_root)
                .map_err(|error| format!("保存 Node.js 运行时失败：{error}"))?;
        }

        if !is_executable(&node_executable(&node_root)) {
            return Err("Node.js 安装后未找到可执行文件。".to_string());
        }

        if !is_executable(&dsh_executable(&dsh_root)) {
            let dsh_staging = staging.join("dsh");
            fs::create_dir_all(&dsh_staging)
                .map_err(|error| format!("创建 Harness 安装目录失败：{error}"))?;
            emit_runtime_progress(
                app,
                state,
                format!("正在安装 DeepSeek Harness {DSH_VERSION}…"),
                Some(0.78),
                false,
                None,
            );
            install_dsh_package(app, state, &node_root, &dsh_staging, DSH_VERSION)?;
            emit_runtime_progress(
                app,
                state,
                "正在整理 Harness 运行时…",
                Some(0.93),
                false,
                None,
            );
            if !is_executable(&dsh_executable(&dsh_staging)) {
                return Err("npm 安装完成，但没有生成 dsh 命令。".to_string());
            }
            if dsh_root.exists() {
                fs::remove_dir_all(&dsh_root)
                    .map_err(|error| format!("替换 Harness 运行时失败：{error}"))?;
            }
            fs::rename(&dsh_staging, &dsh_root)
                .map_err(|error| format!("保存 Harness 运行时失败：{error}"))?;
        }

        if !is_executable(&dsh_executable(&dsh_root)) {
            return Err("DeepSeek Harness 安装后未找到 dsh 命令。".to_string());
        }
        Ok(())
    }
    .await;

    let _ = fs::remove_dir_all(&staging);
    state.runtime_installing.store(false, Ordering::Release);
    match result {
        Ok(()) => {
            link_dsh_to_path(app);
            emit_runtime_progress(app, state, "运行时安装完成。", Some(1.0), true, None);
            Ok(())
        }
        Err(error) => {
            emit_runtime_progress(
                app,
                state,
                "运行时安装失败。",
                None,
                true,
                Some(error.clone()),
            );
            Err(error)
        }
    }
}

fn emit_update_progress(
    app: &AppHandle,
    message: impl Into<String>,
    fraction: Option<f64>,
    done: bool,
    error: Option<String>,
) {
    let _ = app.emit(
        "update-progress",
        UpdateProgress {
            message: message.into(),
            fraction,
            done,
            error,
        },
    );
}

fn is_retryable_download_error(error: &str) -> bool {
    [
        "error decoding response body",
        "error reading a body",
        "unexpected end of file",
        "connection reset",
        "connection closed",
        "timed out",
        "error sending request",
    ]
    .iter()
    .any(|fragment| error.to_ascii_lowercase().contains(fragment))
}

async fn fetch_latest_release() -> Result<GithubRelease, String> {
    let user_agent = format!("DeepSeek Harness Desk/{APP_VERSION}");
    let client = outbound_http_client(Duration::from_secs(20), &user_agent, false)
        .map_err(|error| format!("创建更新检查客户端失败：{error}"))?;
    let response = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("检查 App 更新失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("GitHub 返回 HTTP {}", response.status()));
    }
    response
        .json::<GithubRelease>()
        .await
        .map_err(|error| format!("解析 App 更新信息失败：{error}"))
}

fn release_version(release: &GithubRelease) -> String {
    release.tag_name.trim_start_matches(['v', 'V']).to_string()
}

fn platform_app_asset<'a>(release: &'a GithubRelease) -> Option<&'a GithubAsset> {
    let asset = |predicate: &dyn Fn(&str) -> bool| {
        release
            .assets
            .iter()
            .find(|asset| predicate(&asset.name.to_ascii_lowercase()))
    };

    #[cfg(target_os = "macos")]
    {
        #[cfg(target_arch = "aarch64")]
        let architecture_asset = asset(&|name| name.ends_with("_aarch64.app.tar.gz"));
        #[cfg(target_arch = "x86_64")]
        let architecture_asset = asset(&|name| name.ends_with("_x64.app.tar.gz"));

        architecture_asset
            .or_else(|| asset(&|name| name.ends_with("_universal.app.tar.gz")))
            .or_else(|| asset(&|name| name.ends_with(".app.tar.gz")))
            .or_else(|| asset(&|name| name.ends_with(".dmg")))
    }
    #[cfg(target_os = "windows")]
    {
        asset(&|name| name.ends_with(".exe") && name.contains("setup"))
            .or_else(|| asset(&|name| name.ends_with(".msi")))
    }
    #[cfg(target_os = "linux")]
    {
        asset(&|name| name.ends_with(".appimage")).or_else(|| asset(&|name| name.ends_with(".deb")))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = asset;
        None
    }
}

fn app_update_status(release: &GithubRelease) -> AppUpdateStatus {
    let latest_version = release_version(release);
    let asset = platform_app_asset(release);
    if !is_newer_version(&latest_version, APP_VERSION) {
        return AppUpdateStatus {
            current_version: APP_VERSION.to_string(),
            latest_version: Some(latest_version),
            available: false,
            status: format!("已是最新版本 {APP_VERSION}"),
            release_url: Some(release.html_url.clone()),
            download_url: asset.map(|asset| asset.browser_download_url.clone()),
            asset_name: asset.map(|asset| asset.name.clone()),
            notes: release.body.clone(),
        };
    }

    let (available, status) = if asset.is_some() {
        (true, format!("发现新版本 {latest_version}"))
    } else {
        (
            false,
            format!("发现新版本 {latest_version}，但暂无当前系统安装包"),
        )
    };
    AppUpdateStatus {
        current_version: APP_VERSION.to_string(),
        latest_version: Some(latest_version),
        available,
        status,
        release_url: Some(release.html_url.clone()),
        download_url: asset.map(|asset| asset.browser_download_url.clone()),
        asset_name: asset.map(|asset| asset.name.clone()),
        notes: release.body.clone(),
    }
}

async fn download_update_asset(
    app: &AppHandle,
    asset: &GithubAsset,
    destination: &Path,
) -> Result<(), String> {
    for attempt in 0..2 {
        match download_update_asset_once(app, asset, destination).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt == 0 && is_retryable_download_error(&error) => {
                let _ = fs::remove_file(destination);
                emit_update_progress(app, "下载连接中断，正在重新尝试…", Some(0.05), false, None);
            }
            Err(error) => {
                let _ = fs::remove_file(destination);
                return Err(error);
            }
        }
    }

    let _ = fs::remove_file(destination);
    Err("下载 App 更新失败：下载连接重试次数已用尽。".to_string())
}

async fn download_update_asset_once(
    app: &AppHandle,
    asset: &GithubAsset,
    destination: &Path,
) -> Result<(), String> {
    emit_update_progress(app, "正在下载 App 更新…", Some(0.05), false, None);
    let user_agent = format!("DeepSeek Harness Desk/{APP_VERSION}");
    let client = outbound_http_client(Duration::from_secs(300), &user_agent, true)
        .map_err(|error| format!("创建 App 下载客户端失败：{error}"))?;
    let response = client
        .get(&asset.browser_download_url)
        .header("Accept", "application/octet-stream")
        .header("Accept-Encoding", "identity")
        .send()
        .await
        .map_err(|error| format!("下载 App 更新失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("下载 App 更新失败（HTTP {}）", response.status()));
    }
    let total = response.content_length().or(asset.size);
    let mut response = response;
    let mut file =
        fs::File::create(destination).map_err(|error| format!("创建 App 更新文件失败：{error}"))?;
    let mut downloaded = 0_u64;
    let mut last_fraction = 0.05_f64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("读取 App 更新失败：{error}"))?
    {
        file.write_all(&chunk)
            .map_err(|error| format!("保存 App 更新失败：{error}"))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if let Some(total) = total.filter(|total| *total > 0) {
            let fraction = 0.05 + (downloaded as f64 / total as f64).min(1.0) * 0.70;
            if fraction - last_fraction >= 0.02 || downloaded >= total {
                let percent = (fraction * 100.0).round() as u8;
                emit_update_progress(
                    app,
                    format!("正在下载 App 更新… {percent}%"),
                    Some(fraction),
                    false,
                    None,
                );
                last_fraction = fraction;
            }
        }
    }
    file.flush()
        .map_err(|error| format!("保存 App 更新失败：{error}"))?;
    emit_update_progress(app, "正在校验 App 更新…", Some(0.8), false, None);
    if let Some(expected) = asset.digest.as_deref() {
        verify_sha256(destination, expected)?;
    }
    emit_update_progress(app, "App 更新包校验通过。", Some(0.9), false, None);
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("读取更新包失败：{error}"))?;
    let digest = Sha256::digest(bytes);
    let actual = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let expected = expected
        .split_once(':')
        .map(|(_, digest)| digest)
        .unwrap_or(expected);
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err("更新包完整性校验失败，请重试。".to_string())
    }
}

fn update_root() -> PathBuf {
    env::temp_dir().join(format!(
        "DeepSeekHarnessDesk-Update-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    ))
}

fn safe_asset_filename(name: &str) -> String {
    Path::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("DeepSeekHarnessDesk-update")
        .to_string()
}

fn current_app_bundle() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .map(Path::to_path_buf)
}

fn find_app_bundle(root: &Path) -> Option<PathBuf> {
    if root.extension().is_some_and(|extension| extension == "app") {
        return Some(root.to_path_buf());
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(app) = find_app_bundle(&path) {
                return Some(app);
            }
        }
    }
    None
}

fn shell_quote(value: &Path) -> String {
    format!("'{}'", value.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn macos_app_version(app: &Path) -> Option<String> {
    let info = app.join("Contents/Info.plist");
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(info)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(target_os = "macos")]
fn launch_macos_replacement(
    app: &AppHandle,
    archive: &Path,
    expected_version: &str,
    update_root: &Path,
) -> Result<(), String> {
    let current_app = current_app_bundle().ok_or("当前 App 不是可自动替换的 macOS 应用包。")?;
    let extraction = update_root.join("extracted");
    fs::create_dir_all(&extraction).map_err(|error| format!("创建更新目录失败：{error}"))?;
    let output = Command::new("/usr/bin/tar")
        .args(["-xzf"])
        .arg(archive)
        .args(["-C"])
        .arg(&extraction)
        .output()
        .map_err(|error| format!("解压 App 更新失败：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "解压 App 更新失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let updated_app = find_app_bundle(&extraction).ok_or("更新包中没有找到 macOS App。")?;
    let updated_version = macos_app_version(&updated_app).ok_or("更新包中的 App 缺少版本信息。")?;
    if updated_version != expected_version {
        return Err("更新包版本与 Release 不一致。".to_string());
    }
    let signature = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(&updated_app)
        .output()
        .map_err(|error| format!("校验 App 签名失败：{error}"))?;
    if !signature.status.success() {
        return Err(format!(
            "更新包签名校验失败：{}",
            String::from_utf8_lossy(&signature.stderr).trim()
        ));
    }
    let script = update_root.join("replace-app.sh");
    let current_pid = std::process::id();
    let contents = format!(
        "#!/bin/sh\nset -eu\nold_pid={current_pid}\nwhile kill -0 \"$old_pid\" 2>/dev/null; do sleep 0.25; done\nrm -rf {current}\nmv {updated} {current}\nopen -n {current}\nrm -f \"$0\"\n",
        current = shell_quote(&current_app),
        updated = shell_quote(&updated_app),
    );
    fs::write(&script, contents).map_err(|error| format!("准备 App 替换程序失败：{error}"))?;
    let output = Command::new("/bin/sh")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("启动 App 替换程序失败：{error}"))?;
    let _ = output.id();
    emit_update_progress(
        app,
        "更新完成，正在重启 DeepSeek Harness Desk…",
        Some(1.0),
        true,
        None,
    );
    app.exit(0);
    Ok(())
}

async fn install_app_update_inner(
    app: &AppHandle,
    state: &HarnessState,
) -> Result<AppUpdateStatus, String> {
    if state.app_update_installing.swap(true, Ordering::AcqRel) {
        return Err("App 更新正在进行，请等待当前更新完成。".to_string());
    }
    let result: Result<AppUpdateStatus, String> = async {
        let release = fetch_latest_release().await?;
        let status = app_update_status(&release);
        if !status.available {
            return Ok(status);
        }
        let asset = platform_app_asset(&release).ok_or("最新 Release 没有当前系统的安装包。")?;
        let root = update_root();
        fs::create_dir_all(&root).map_err(|error| format!("创建更新目录失败：{error}"))?;
        let archive = root.join(safe_asset_filename(&asset.name));
        download_update_asset(app, asset, &archive).await?;
        let version = release_version(&release);

        #[cfg(target_os = "macos")]
        {
            if asset.name.to_ascii_lowercase().ends_with(".app.tar.gz") {
                launch_macos_replacement(app, &archive, &version, &root)?;
            } else {
                open_external_url(&asset.browser_download_url)?;
                emit_update_progress(app, "已打开 macOS 安装包下载。", Some(1.0), true, None);
            }
        }
        #[cfg(target_os = "windows")]
        {
            if asset.name.to_ascii_lowercase().ends_with(".msi") {
                Command::new("msiexec.exe")
                    .args(["/i"])
                    .arg(&archive)
                    .spawn()
                    .map_err(|error| format!("启动 Windows 安装程序失败：{error}"))?;
            } else {
                Command::new(&archive)
                    .spawn()
                    .map_err(|error| format!("启动 Windows 安装程序失败：{error}"))?;
            }
            emit_update_progress(
                app,
                "安装程序已启动，正在退出旧版本…",
                Some(1.0),
                true,
                None,
            );
            app.exit(0);
        }
        #[cfg(target_os = "linux")]
        {
            if asset.name.to_ascii_lowercase().ends_with(".appimage") {
                let mut permissions = fs::metadata(&archive)
                    .map_err(|error| format!("读取 AppImage 权限失败：{error}"))?
                    .permissions();
                use std::os::unix::fs::PermissionsExt;
                permissions.set_mode(permissions.mode() | 0o755);
                fs::set_permissions(&archive, permissions)
                    .map_err(|error| format!("设置 AppImage 权限失败：{error}"))?;
                Command::new(&archive)
                    .spawn()
                    .map_err(|error| format!("启动 AppImage 失败：{error}"))?;
                emit_update_progress(app, "新版本 AppImage 已启动。", Some(1.0), true, None);
            } else {
                open_external_url(&asset.browser_download_url)?;
                emit_update_progress(app, "已打开 Linux 安装包下载。", Some(1.0), true, None);
            }
        }
        Ok(status)
    }
    .await;
    state.app_update_installing.store(false, Ordering::Release);
    if let Err(error) = &result {
        emit_update_progress(app, "App 更新失败。", None, true, Some(error.clone()));
    }
    result
}

/// Fetch `@deepseek-ai/dsh` metadata from npm. Shared by the update check and
/// the version list so both see exactly the same tags and versions.
async fn fetch_npm_dsh_metadata(app: &AppHandle) -> Result<NpmMetadata, String> {
    let user_agent = format!("DeepSeek Harness Desk/{APP_VERSION}");
    let client = outbound_http_client(Duration::from_secs(20), &user_agent, false)
        .map_err(|error| format!("创建 dsh 更新检查客户端失败：{error}"))?;
    let response = client
        .get(NPM_METADATA_URL)
        .send()
        .await
        .map_err(|error| format!("检查 dsh 更新失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("npm 返回 HTTP {}", response.status()));
    }
    let _ = app;
    response
        .json::<NpmMetadata>()
        .await
        .map_err(|error| format!("解析 dsh 更新信息失败：{error}"))
}

/// The two version streams npm publishes: the release candidates behind
/// `latest`/`next`, and the early builds behind `alpha`/`beta`.
fn published_dsh_versions(metadata: &NpmMetadata) -> (Option<String>, Option<String>) {
    let tag = |value: &Option<String>| value.clone().filter(|version| !version.is_empty());
    let latest = tag(&metadata.dist_tags.latest);
    let next = tag(&metadata.dist_tags.next);
    let alpha = tag(&metadata.dist_tags.alpha);
    let beta = tag(&metadata.dist_tags.beta);
    let stable = newest_published_version(latest.as_deref(), next.as_deref());
    let preview = newest_of_published_versions(&[alpha.as_deref(), beta.as_deref()]);
    (stable, preview)
}

async fn check_dsh_update_inner(
    app: &AppHandle,
    channel: DshUpdateChannel,
) -> Result<DshUpdateStatus, String> {
    let current_version = managed_dsh_version(Some(app));
    let pinned_version = pinned_dsh_version(Some(app));
    if current_version.is_none() {
        return Ok(DshUpdateStatus {
            managed: false,
            current_version: None,
            latest_version: None,
            available: false,
            channel: channel.as_str().to_string(),
            preview: false,
            preview_version: None,
            current_is_preview: false,
            pinned_version: None,
            stable_version: None,
            status: "尚未安装内置 dsh，完成一键安装后可检查更新。".to_string(),
        });
    }
    let metadata = fetch_npm_dsh_metadata(app).await?;
    // `alpha`/`beta` carry the early builds; they only ever win when the user
    // asked for the preview channel.
    let (stable, preview) = published_dsh_versions(&metadata);
    let latest = newest_of_published_versions(&[
        stable.as_deref(),
        if channel == DshUpdateChannel::Preview {
            preview.as_deref()
        } else {
            None
        },
    ])
    .ok_or("npm 未返回 dsh 的可用版本（latest/next/alpha/beta）。")?;
    let current = current_version.unwrap_or_default();
    let available = is_newer_version(&latest, &current);
    let current_is_preview = is_preview_dsh_version(&current);
    let offered_is_preview = is_preview_dsh_version(&latest);
    let preview_version = preview.filter(|version| is_newer_version(version, &current));
    let status = if let Some(pinned) = pinned_version.as_deref() {
        if pinned == latest {
            format!("已固定到内置 dsh {pinned}")
        } else {
            format!("已固定到内置 dsh {pinned}；当前通道最新为 {latest}")
        }
    } else if available {
        if offered_is_preview {
            format!("发现内置 dsh 预览版 {latest}")
        } else {
            format!("发现内置 dsh 新版本 {latest}")
        }
    } else if current_is_preview {
        match stable.as_deref() {
            Some(stable) => format!("当前运行预览版 {current}；稳定通道最新为 {stable}"),
            None => format!("当前运行预览版 {current}"),
        }
    } else {
        format!("内置 dsh 已是最新版本 {current}")
    };
    Ok(DshUpdateStatus {
        managed: true,
        current_version: Some(current),
        latest_version: Some(latest),
        available,
        channel: channel.as_str().to_string(),
        preview: offered_is_preview,
        preview_version,
        current_is_preview,
        pinned_version,
        stable_version: stable,
        status,
    })
}

/// Install one dsh version into `<runtime>/dsh/<version>` and make it active.
///
/// `pin` is what the launcher must use once the files are in place: `Some` pins
/// the freshly installed version (used by the downgrade flow), `None` clears any
/// pin so the newest installed build wins again (used by "更新 dsh").
async fn install_dsh_version_inner(
    app: &AppHandle,
    state: &HarnessState,
    version: String,
    pin: Option<String>,
) -> Result<RuntimeStatus, String> {
    if !is_safe_package_version(&version) {
        return Err("dsh 版本号无效。".to_string());
    }
    if state.runtime_installing.swap(true, Ordering::AcqRel) {
        return Err("运行时更新正在进行，请等待当前更新完成。".to_string());
    }
    let was_running = snapshot(state).running;
    let result = async {
        let node_root =
            managed_node_root(Some(app)).ok_or("未找到内置 Node.js，请先安装运行时。")?;
        let runtime_root = preferred_runtime_root(app);
        let staging = runtime_root.join(format!(".dsh-update-{}-{}", std::process::id(), version));
        let dsh_root = runtime_root.join("dsh").join(&version);
        fs::create_dir_all(&staging).map_err(|error| format!("创建 dsh 更新目录失败：{error}"))?;
        let dsh_staging = staging.join("dsh");
        fs::create_dir_all(&dsh_staging)
            .map_err(|error| format!("创建 dsh 临时目录失败：{error}"))?;
        // 先下载并安装到临时目录，期间 Harness 保持运行、用户可继续使用。
        // 下载/安装全部完成、确认新版本可用之后，才短暂停止 Harness 来替换
        // 版本目录，替换完成后由调用方重新启动，把打断用户的时间压到最短。
        emit_runtime_progress(
            app,
            state,
            format!("正在下载并安装内置 dsh {version}…（Harness 可继续使用）"),
            Some(0.08),
            false,
            None,
        );
        install_dsh_package(app, state, &node_root, &dsh_staging, &version)?;
        emit_runtime_progress(
            app,
            state,
            "下载完成，正在校验内置 dsh…",
            Some(0.82),
            false,
            None,
        );
        if !is_executable(&dsh_executable(&dsh_staging)) {
            return Err("npm 安装完成，但没有生成 dsh 命令。".to_string());
        }
        // 下载与校验已完成，此时才停止 Harness，随即替换版本目录。
        if was_running {
            stop_harness_inner(app, state);
        }
        fs::create_dir_all(runtime_root.join("dsh"))
            .map_err(|error| format!("创建 dsh 版本目录失败：{error}"))?;
        if dsh_root.exists() {
            fs::remove_dir_all(&dsh_root).map_err(|error| format!("替换 dsh 版本失败：{error}"))?;
        }
        fs::rename(&dsh_staging, &dsh_root)
            .map_err(|error| format!("保存 dsh 更新失败：{error}"))?;
        let _ = fs::remove_dir_all(&staging);
        // 版本目录就位后才写 pin：写失败也不会留下“指向不存在版本”的固定记录。
        write_dsh_pin(app, pin.as_deref())?;
        emit_runtime_progress(
            app,
            state,
            format!("内置 dsh {version} 已就绪。"),
            Some(1.0),
            true,
            None,
        );
        Ok(())
    }
    .await;
    state.runtime_installing.store(false, Ordering::Release);
    if let Err(error) = &result {
        // 下载阶段失败时 Harness 仍在运行，start_harness_inner 检测到已运行
        // 会直接返回，不会重复启动；替换阶段失败时则在这里重新拉起 Harness。
        if was_running {
            let _ = start_harness_inner(app, state).await;
        }
        emit_runtime_progress(
            app,
            state,
            "内置 dsh 安装失败。",
            None,
            true,
            Some(error.clone()),
        );
    }
    result?;
    link_dsh_to_path(app);
    if was_running {
        start_harness_inner(app, state).await?;
    }
    Ok(runtime_status_snapshot(app, state))
}

/// Make one installed dsh version active by pinning it.
///
/// The launcher always activates the newest managed version, so a downgrade (or
/// just leaving a preview build) needs an explicit record of the version the
/// user chose. Pinning is reversible and non-destructive: the other installed
/// builds stay on disk, and "跟随最新版" clears the pin again. A version that is
/// not installed yet is fetched from npm first.
async fn activate_dsh_version_inner(
    app: &AppHandle,
    state: &HarnessState,
    version: String,
) -> Result<RuntimeStatus, String> {
    if !is_safe_package_version(&version) {
        return Err("dsh 版本号无效。".to_string());
    }
    let installed = managed_dsh_version_paths(Some(app))
        .into_iter()
        .any(|(candidate, path)| candidate == version && is_executable(&dsh_executable(&path)));
    if !installed {
        emit_runtime_progress(
            app,
            state,
            format!("内置 dsh {version} 尚未安装，正在从 npm 下载…"),
            Some(0.02),
            false,
            None,
        );
        return install_dsh_version_inner(app, state, version.clone(), Some(version)).await;
    }
    if state.runtime_installing.swap(true, Ordering::AcqRel) {
        return Err("运行时正在安装或更新，请稍后再试。".to_string());
    }
    let already_active = managed_dsh_version(Some(app)).as_deref() == Some(version.as_str());
    let was_running = snapshot(state).running;
    let result = (|| -> Result<(), String> {
        emit_runtime_progress(
            app,
            state,
            format!("正在切换到内置 dsh {version}…"),
            Some(0.3),
            false,
            None,
        );
        write_dsh_pin(app, Some(&version))?;
        if !already_active && was_running {
            stop_harness_inner(app, state);
        }
        Ok(())
    })();
    state.runtime_installing.store(false, Ordering::Release);
    if let Err(error) = result {
        emit_runtime_progress(
            app,
            state,
            "切换内置 dsh 版本失败。",
            None,
            true,
            Some(error.clone()),
        );
        return Err(error);
    }
    // Wrapper 与运行中的 Harness 都要跟着 pin 走，终端里的 dsh 才和 App 一致。
    link_dsh_to_path(app);
    if already_active {
        emit_runtime_progress(
            app,
            state,
            format!("内置 dsh 已固定到 {version}。"),
            Some(1.0),
            true,
            None,
        );
        return Ok(runtime_status_snapshot(app, state));
    }
    if was_running {
        start_harness_inner(app, state).await?;
    }
    emit_runtime_progress(
        app,
        state,
        format!("内置 dsh 已切换到 {version}。"),
        Some(1.0),
        true,
        None,
    );
    Ok(runtime_status_snapshot(app, state))
}

/// Clear the pin so the newest installed dsh version is used again.
async fn follow_latest_dsh_version_inner(
    app: &AppHandle,
    state: &HarnessState,
) -> Result<RuntimeStatus, String> {
    if state.runtime_installing.swap(true, Ordering::AcqRel) {
        return Err("运行时正在安装或更新，请稍后再试。".to_string());
    }
    let before = managed_dsh_version(Some(app));
    let was_running = snapshot(state).running;
    let result = write_dsh_pin(app, None);
    state.runtime_installing.store(false, Ordering::Release);
    if let Err(error) = result {
        emit_runtime_progress(
            app,
            state,
            "取消固定版本失败。",
            None,
            true,
            Some(error.clone()),
        );
        return Err(error);
    }
    let after = managed_dsh_version(Some(app));
    link_dsh_to_path(app);
    if before == after {
        emit_runtime_progress(
            app,
            state,
            format!(
                "内置 dsh 已改为跟随最新版（{}）。",
                after.unwrap_or_default()
            ),
            Some(1.0),
            true,
            None,
        );
        return Ok(runtime_status_snapshot(app, state));
    }
    emit_runtime_progress(
        app,
        state,
        format!(
            "正在切回最新的内置 dsh {}…",
            after.clone().unwrap_or_default()
        ),
        Some(0.4),
        false,
        None,
    );
    if was_running {
        stop_harness_inner(app, state);
        start_harness_inner(app, state).await?;
    }
    emit_runtime_progress(
        app,
        state,
        format!("内置 dsh 已跟随最新版（{}）。", after.unwrap_or_default()),
        Some(1.0),
        true,
        None,
    );
    Ok(runtime_status_snapshot(app, state))
}

/// List every selectable dsh version: what npm publishes plus what is installed.
async fn list_dsh_versions_inner(app: &AppHandle) -> Result<DshVersionsStatus, String> {
    let installed = managed_dsh_version_paths(Some(app))
        .into_iter()
        .map(|(version, path)| (version, is_executable(&dsh_executable(&path))))
        .collect::<Vec<_>>();
    let active_version = managed_dsh_version(Some(app));
    let pinned_version = pinned_dsh_version(Some(app));
    let mut ordered = Vec::new();
    let mut latest_stable = None;
    let mut latest_preview = None;
    let mut npm_reachable = false;
    if let Ok(metadata) = fetch_npm_dsh_metadata(app).await {
        npm_reachable = true;
        let (stable, preview) = published_dsh_versions(&metadata);
        latest_stable = stable;
        latest_preview = preview;
        if let Some(versions) = metadata.versions {
            ordered.extend(versions.into_keys());
            ordered.sort_by(|left, right| compare_versions(right, left));
            ordered.truncate(MAX_LISTED_DSH_VERSIONS);
        }
    }
    for (version, _) in &installed {
        if !ordered.iter().any(|candidate| candidate == version) {
            ordered.push(version.clone());
        }
    }
    ordered.sort_by(|left, right| compare_versions(right, left));
    let versions = ordered
        .into_iter()
        .map(|version| DshVersionOption {
            preview: is_preview_dsh_version(&version),
            installed: installed
                .iter()
                .any(|(candidate, usable)| candidate == &version && *usable),
            active: active_version.as_deref() == Some(version.as_str()),
            version,
        })
        .collect();
    Ok(DshVersionsStatus {
        active_version,
        pinned_version,
        latest_stable,
        latest_preview,
        versions,
        npm_reachable,
    })
}

/// Opens a web link with the system's default handler (browser or mail
/// client). Only `http(s)` and `mailto` are accepted: a page must never be able
/// to hand an arbitrary scheme to the operating system.
fn open_external_url(url: &str) -> Result<(), String> {
    if !is_openable_web_url(url) {
        return Err("只允许打开网页链接。".to_string());
    }
    launch_url(url)
}

/// True for the URL shapes the shell allows to leave the app. Deliberately a
/// prefix check on already-trimmed input so nothing like `javascript:` or a
/// relative target can slip through.
fn is_openable_web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:")
}

fn launch_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).status();
    #[cfg(target_os = "windows")]
    let result = Command::new("cmd").args(["/C", "start", "", url]).status();
    #[cfg(target_os = "linux")]
    let result = Command::new("xdg-open").arg(url).status();
    result
        .map_err(|error| format!("打开链接失败：{error}"))?
        .success()
        .then_some(())
        .ok_or_else(|| "打开链接失败。".to_string())
}

fn remember_log(logs: &Arc<Mutex<VecDeque<HarnessLog>>>, log: HarnessLog) {
    if let Ok(mut logs) = logs.lock() {
        logs.push_back(log);
        while logs.len() > MAX_LOG_LINES {
            logs.pop_front();
        }
    }
}

fn emit_log(
    app: &AppHandle,
    logs: &Arc<Mutex<VecDeque<HarnessLog>>>,
    stream: &str,
    message: impl Into<String>,
) {
    let log = HarnessLog {
        stream: stream.to_string(),
        message: message.into(),
    };
    if let Ok(directory) = app.path().app_data_dir() {
        let directory = directory.join("logs");
        if fs::create_dir_all(&directory).is_ok() {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(directory.join("harness.log"))
            {
                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or_default();
                let _ = writeln!(file, "[{timestamp}] [{}] {}", log.stream, log.message);
            }
        }
    }
    remember_log(logs, log.clone());
    let _ = app.emit("harness-output", log);
}

fn spawn_output_reader<R>(
    reader: R,
    app: AppHandle,
    logs: Arc<Mutex<VecDeque<HarnessLog>>>,
    stream: &'static str,
    token_sink: Option<Arc<Mutex<Option<String>>>>,
) where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let message = line.trim_end_matches(['\r', '\n']).to_string();
                    if !message.is_empty() {
                        if let Some(sink) = &token_sink {
                            if let Some(token) = extract_launch_token(&message) {
                                if let Ok(mut guard) = sink.lock() {
                                    *guard = Some(token);
                                }
                            }
                        }
                        emit_log(&app, &logs, stream, message);
                    }
                }
                Err(error) => {
                    emit_log(&app, &logs, stream, format!("读取进程输出失败：{error}"));
                    break;
                }
            }
        }
    });
}

/// Extract the web UI launch token from a dsh stdout line such as
/// `dsh web: http://127.0.0.1:3080/?token=abc123`. Newer dsh releases gate
/// the web UI behind token authentication, so the app must capture the token
/// to log in on behalf of the embedded WebView and the event watcher.
fn extract_launch_token(line: &str) -> Option<String> {
    const MARKER: &str = "/?token=";
    let mut search_from = 0;
    while let Some(relative) = line[search_from..].find(MARKER) {
        let token_start = search_from + relative + MARKER.len();
        let token: String = line[token_start..]
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() || *character == '-' || *character == '_'
            })
            .collect();
        if !token.is_empty() {
            return Some(token);
        }
        search_from = token_start;
        if search_from >= line.len() {
            break;
        }
    }
    None
}

fn set_last_error(state: &HarnessState, message: Option<String>) {
    if let Ok(mut error) = state.last_error.lock() {
        *error = message;
    }
}

fn set_exit_code(state: &HarnessState, code: Option<i32>) {
    if let Ok(mut exit_code) = state.last_exit_code.lock() {
        *exit_code = code;
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn touch_harness_activity(state: &HarnessState) {
    state.last_event_at.store(now_millis(), Ordering::Release);
}

fn stop_harness_proxy(state: &HarnessState) {
    if let Ok(mut stop) = state.proxy_stop.lock() {
        if let Some(stop) = stop.take() {
            stop.store(true, Ordering::Release);
        }
    }
    state.proxy_port.store(0, Ordering::Relaxed);
}

fn snapshot(state: &HarnessState) -> HarnessStatus {
    let mut running = false;
    let mut pid = None;

    if let Ok(mut guard) = state.child.lock() {
        let mut finished = false;
        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                Ok(None) => {
                    running = true;
                    pid = Some(child.id());
                }
                Ok(Some(status)) => {
                    finished = true;
                    set_exit_code(state, status.code());
                }
                Err(error) => {
                    finished = true;
                    set_last_error(state, Some(format!("读取 dsh 状态失败：{error}")));
                }
            }
        }

        if finished {
            let _ = guard.take();
            state.port.store(0, Ordering::Relaxed);
            stop_harness_proxy(state);
            if let Ok(mut path) = state.dsh_path.lock() {
                *path = None;
            }
            if let Ok(mut token) = state.token.lock() {
                *token = None;
            }
            if let Ok(mut cookie) = state.auth_cookie.lock() {
                *cookie = None;
            }
        }
    }

    let port = state.port.load(Ordering::Relaxed);
    let proxy_port = state.proxy_port.load(Ordering::Relaxed);
    // The WebView must load the browser-facing proxy rather than the dsh
    // backend directly. The proxy adds the native auth cookie to every HTTP
    // and WebSocket request, which avoids WebKit's third-party-cookie policy.
    let url = (running && proxy_port > 0).then(|| format!("http://127.0.0.1:{proxy_port}"));
    let dsh_path = if running {
        state.dsh_path.lock().ok().and_then(|path| {
            path.as_ref()
                .map(|path| path.to_string_lossy().into_owned())
        })
    } else {
        None
    };

    HarnessStatus {
        running,
        port: running.then_some(port),
        url,
        pid,
        dsh_path,
        exit_code: state.last_exit_code.lock().ok().and_then(|code| *code),
        error: state.last_error.lock().ok().and_then(|error| error.clone()),
        last_event_at: {
            let timestamp = state.last_event_at.load(Ordering::Acquire);
            (timestamp > 0).then_some(timestamp)
        },
    }
}

fn stop_harness_inner(app: &AppHandle, state: &HarnessState) {
    stop_harness_proxy(state);
    let mut stopped_pid = None;
    if let Ok(mut guard) = state.child.lock() {
        if let Some(mut child) = guard.take() {
            stopped_pid = Some(child.id());
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/PID", &child.id().to_string(), "/T", "/F"])
                    .status();
            }
            #[cfg(unix)]
            {
                let _ = Command::new("/bin/kill")
                    .args(["-TERM", &format!("-{}", child.id())])
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    state.port.store(0, Ordering::Relaxed);
    if let Ok(mut path) = state.dsh_path.lock() {
        *path = None;
    }
    if let Ok(mut token) = state.token.lock() {
        *token = None;
    }
    if let Ok(mut cookie) = state.auth_cookie.lock() {
        *cookie = None;
    }
    set_exit_code(state, None);

    if let Some(pid) = stopped_pid {
        emit_log(
            app,
            &state.logs,
            "desk",
            format!("已停止 Harness（pid {pid}）"),
        );
    }
}

async fn start_harness_inner(
    app: &AppHandle,
    state: &HarnessState,
) -> Result<HarnessStatus, String> {
    // Serialize the check-and-spawn section. Double-clicking Start must never
    // create two dsh processes before the first one becomes healthy.
    let lifecycle_guard = state
        .lifecycle
        .lock()
        .map_err(|_| "无法锁定 Harness 生命周期状态。".to_string())?;
    let current = snapshot(state);
    if current.running {
        return Ok(current);
    }

    set_last_error(state, None);
    set_exit_code(state, None);
    // Each dsh process mints a fresh launch token; a stale one from a previous
    // run must never leak into the new process's authenticated URL.
    if let Ok(mut guard) = state.token.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = state.auth_cookie.lock() {
        *guard = None;
    }

    let Some(command) = dsh_command(app) else {
        let message = "未找到 dsh 可执行文件。请先点击“安装并启动”，或设置 DSH_BIN 环境变量。";
        set_last_error(state, Some(message.to_string()));
        emit_log(app, &state.logs, "desk", message);
        return Err(message.to_string());
    };
    link_dsh_to_path(app);

    let reaped_processes = reap_orphaned_managed_harnesses(app);
    if !reaped_processes.is_empty() {
        emit_log(
            app,
            &state.logs,
            "desk",
            format!(
                "已清理上次遗留的 Harness 进程：{}",
                reaped_processes
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }

    let Some(port) = first_available_port(PORT_START, PORT_END) else {
        let message = "3080–3099 端口均不可用，请释放端口后重试。";
        set_last_error(state, Some(message.to_string()));
        emit_log(app, &state.logs, "desk", message);
        return Err(message.to_string());
    };
    let Some(proxy_port) = first_available_port_excluding(PORT_START, PORT_END, port) else {
        let message = "没有可用的 Harness 代理端口，请释放 3080–3099 端口后重试。";
        set_last_error(state, Some(message.to_string()));
        emit_log(app, &state.logs, "desk", message);
        return Err(message.to_string());
    };

    let mut child = spawn_dsh(&command, port, app).map_err(|error| {
        let message = format!("启动 dsh 失败：{error}");
        set_last_error(state, Some(message.clone()));
        message
    })?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    if let Ok(mut guard) = state.child.lock() {
        *guard = Some(child);
    } else {
        let message = "无法锁定 Harness 状态。".to_string();
        set_last_error(state, Some(message.clone()));
        return Err(message);
    }
    state.port.store(port, Ordering::Relaxed);
    if let Ok(mut path) = state.dsh_path.lock() {
        *path = Some(command.program.clone());
    }

    if let Some(stdout) = stdout {
        spawn_output_reader(
            stdout,
            app.clone(),
            state.logs.clone(),
            "stdout",
            Some(state.token.clone()),
        );
    }
    if let Some(stderr) = stderr {
        spawn_output_reader(stderr, app.clone(), state.logs.clone(), "stderr", None);
    }

    // The process is now registered. Release the guard while waiting for the
    // HTTP server; another Start call will observe the live child and return
    // its status instead of spawning a duplicate.
    drop(lifecycle_guard);

    let url = format!("http://127.0.0.1:{port}");
    emit_log(
        app,
        &state.logs,
        "desk",
        format!("正在启动 dsh（pid {pid}）：{url}"),
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(1200))
        .build()
        .map_err(|error| format!("创建健康检查客户端失败：{error}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        if !snapshot(state).running {
            let message = "dsh 启动后立即退出，请查看日志获取详细信息。".to_string();
            set_last_error(state, Some(message.clone()));
            emit_log(app, &state.logs, "desk", &message);
            return Err(message);
        }

        // The Desk is only a shell around a URL: any HTTP answer proves the
        // server is up and can be shown — 2xx, redirect and 401 alike. The
        // page's own authentication (token, cookie, whatever a new dsh ships)
        // is its business and must never gate the shell's readiness again.
        if client.get(&url).send().await.is_ok() {
            emit_log(app, &state.logs, "desk", format!("Harness 已就绪：{url}"));
            // Complete the current dsh login handshake before returning the
            // status to the shell. The browser-facing proxy must have the
            // cookie before the iframe is allowed to navigate.
            mint_harness_cookie(state.clone(), port).await;
            let cookie = state
                .auth_cookie
                .lock()
                .ok()
                .and_then(|cookie| cookie.clone());
            if cookie.is_some() {
                emit_log(
                    app,
                    &state.logs,
                    "desk",
                    "已取得 Harness 登录凭据".to_string(),
                );
            }
            if let Err(error) = start_harness_proxy(state, port, proxy_port, cookie).await {
                stop_harness_inner(app, state);
                set_last_error(state, Some(error.clone()));
                emit_log(app, &state.logs, "desk", error.clone());
                return Err(error);
            }
            return Ok(snapshot(state));
        }

        if Instant::now() >= deadline {
            let message = format!("dsh 在 30 秒内未就绪：{url}");
            stop_harness_inner(app, state);
            set_last_error(state, Some(message.clone()));
            emit_log(app, &state.logs, "desk", &message);
            return Err(message);
        }

        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

const MAX_PROXY_REQUEST_HEADER_BYTES: usize = 64 * 1024;

async fn start_harness_proxy(
    state: &HarnessState,
    backend_port: u16,
    proxy_port: u16,
    cookie: Option<String>,
) -> Result<(), String> {
    let listener = TcpListener::bind(("127.0.0.1", proxy_port))
        .await
        .map_err(|error| format!("启动 Harness 认证代理失败：{error}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    if let Ok(mut guard) = state.proxy_stop.lock() {
        *guard = Some(stop.clone());
    } else {
        return Err("无法锁定 Harness 代理状态。".to_string());
    }
    state.proxy_port.store(proxy_port, Ordering::Release);

    tauri::async_runtime::spawn(run_harness_proxy(listener, backend_port, cookie, stop));
    Ok(())
}

async fn run_harness_proxy(
    listener: TcpListener,
    backend_port: u16,
    cookie: Option<String>,
    stop: Arc<AtomicBool>,
) {
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }

        tokio::select! {
            accepted = listener.accept() => {
                let Ok((client, _)) = accepted else {
                    continue;
                };
                let cookie = cookie.clone();
                tauri::async_runtime::spawn(proxy_harness_connection(
                    client,
                    backend_port,
                    cookie,
                ));
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
}

async fn proxy_harness_connection(
    mut client: TcpStream,
    backend_port: u16,
    cookie: Option<String>,
) {
    let mut request = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 8192];
    let header_end = loop {
        let Ok(read) = client.read(&mut chunk).await else {
            return;
        };
        if read == 0 {
            return;
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        if request.len() > MAX_PROXY_REQUEST_HEADER_BYTES {
            return;
        }
    };

    let Some(rewritten) =
        rewrite_harness_proxy_request(&request[..header_end], backend_port, cookie.as_deref())
    else {
        return;
    };
    let Ok(mut backend) = TcpStream::connect(("127.0.0.1", backend_port)).await else {
        return;
    };
    if backend.write_all(&rewritten).await.is_err()
        || backend.write_all(&request[header_end..]).await.is_err()
    {
        return;
    }

    let _ = tokio::io::copy_bidirectional(&mut client, &mut backend).await;
}

fn rewrite_harness_proxy_request(
    request: &[u8],
    backend_port: u16,
    cookie: Option<&str>,
) -> Option<Vec<u8>> {
    let request = std::str::from_utf8(request).ok()?;
    let mut lines = request.split("\r\n");
    let request_line = lines.next()?.trim_end();
    if request_line.is_empty() {
        return None;
    }

    let mut headers = Vec::new();
    let mut websocket_upgrade = false;
    let mut has_origin = false;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("upgrade")
            && value
                .split(',')
                .any(|value| value.trim().eq_ignore_ascii_case("websocket"))
        {
            websocket_upgrade = true;
        }
        if name.eq_ignore_ascii_case("origin") {
            has_origin = true;
        }
        headers.push((name, value, line));
    }

    let mut rewritten = String::with_capacity(request.len() + 128);
    rewritten.push_str(request_line);
    rewritten.push_str("\r\n");
    for (name, _value, line) in headers {
        if name.eq_ignore_ascii_case("host")
            || name.eq_ignore_ascii_case("cookie")
            || name.eq_ignore_ascii_case("proxy-connection")
            || name.eq_ignore_ascii_case("origin")
            || (!websocket_upgrade && name.eq_ignore_ascii_case("connection"))
        {
            continue;
        }
        rewritten.push_str(line);
        rewritten.push_str("\r\n");
    }
    rewritten.push_str(&format!("Host: 127.0.0.1:{backend_port}\r\n"));
    if has_origin {
        rewritten.push_str(&format!("Origin: http://127.0.0.1:{backend_port}\r\n"));
    }
    if let Some(cookie) = cookie {
        rewritten.push_str("Cookie: ");
        rewritten.push_str(cookie);
        rewritten.push_str("\r\n");
    }
    if !websocket_upgrade {
        rewritten.push_str("Connection: close\r\n");
    }
    rewritten.push_str("\r\n");
    Some(rewritten.into_bytes())
}

/// Best-effort login for the browser-facing Harness proxy.
///
/// Token-authenticated dsh only mints its auth cookie when the root URL is
/// opened with the launch token, and WebKit blocks that host-only cookie when
/// the Harness is embedded as a third-party iframe. This waits for the token
/// printed on stdout and performs the exchange natively. The cookie is then
/// retained by the local proxy and attached to every forwarded request.
/// Legacy dsh versions simply time out the short token wait.
async fn mint_harness_cookie(state: HarnessState, port: u16) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(1200))
        // Keep the raw exchange response so its cookie can be captured.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => return,
    };

    // The token is printed by dsh right before the server starts, but stdout
    // arrives asynchronously. A short wait keeps legacy startup responsive.
    let token = {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let token = state.token.lock().ok().and_then(|token| token.clone());
            if token.is_some() || Instant::now() >= deadline {
                break token;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    let Some(token) = token else {
        return;
    };

    let login_url = format!("http://127.0.0.1:{port}/?token={token}");
    for _ in 0..6 {
        if let Ok(response) = client.get(&login_url).send().await {
            let cookie = response
                .headers()
                .get(reqwest::header::SET_COOKIE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(str::to_string);
            if let Some(cookie) = cookie {
                if let Ok(mut guard) = state.auth_cookie.lock() {
                    *guard = Some(cookie);
                }
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

const MAX_SEEN_INTERACTIONS: usize = 256;

/// Start the background watcher that tails the Harness's live event stream and
/// turns "needs interaction" and "task completed" into a dock badge and a
/// system notification. dsh 0.1.2+ serves a single authenticated
/// `/api/remote.mux`; older releases expose the unauthenticated
/// `events.mux` + `events.host` pair.
fn spawn_task_watcher(app: AppHandle, state: HarnessState) {
    tauri::async_runtime::spawn(async move {
        let mut retry_delay = Duration::from_millis(1500);
        // Dedup keys and per-session running state live outside the reconnect
        // loop: dsh replays still-pending interaction requests to every new
        // client, and a session observed running before a reconnect must still
        // fire "completed" when it goes idle afterwards.
        let mut ledger = AttentionLedger::default();
        loop {
            let port = state.port.load(Ordering::Relaxed);
            if port == 0 {
                // The backend is gone. Session state observed on the previous
                // process says nothing about the next one, and keeping it
                // would turn the first replay after a restart into a bogus
                // "task completed" for work the user restarted themselves.
                ledger.reset();
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
            let outcome = watch_harness_events(&app, &state, port, &mut ledger).await;
            // A dead or incompatible event stream must not turn into a tight
            // reconnect loop against the Harness backend.
            retry_delay = match outcome {
                Ok(()) => Duration::from_millis(1500),
                Err(_) => (retry_delay * 2).min(Duration::from_secs(30)),
            };
            tokio::time::sleep(retry_delay).await;
        }
    });
}

/// Connect a Harness event WebSocket, attaching the auth cookie minted by the
/// token exchange when token-authenticated dsh is in charge.
async fn connect_harness_ws(
    url: &str,
    cookie: Option<&str>,
) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::header::{HeaderValue, COOKIE};

    let mut request = url
        .into_client_request()
        .map_err(|error| error.to_string())?;
    if let Some(cookie) = cookie {
        let value = HeaderValue::from_str(cookie).map_err(|error| error.to_string())?;
        request.headers_mut().insert(COOKIE, value);
    }
    let (stream, _) = connect_async(request)
        .await
        .map_err(|error| error.to_string())?;
    Ok(stream)
}

/// Keep the Harness event stream(s) open for as long as they last. Returns
/// when the stream closes or fails; the caller reconnects after a pause.
///
/// dsh 0.1.2+ serves a single authenticated mux endpoint whose event data only
/// flows after the shell opens the forwarded-event logical stream; older
/// releases expose the unauthenticated `events.mux` + `events.host` pair,
/// which stays supported here as a fallback.
async fn watch_harness_events(
    app: &AppHandle,
    state: &HarnessState,
    port: u16,
    ledger: &mut AttentionLedger,
) -> Result<(), String> {
    let cookie = state
        .auth_cookie
        .lock()
        .ok()
        .and_then(|cookie| cookie.clone());
    let base = format!("ws://127.0.0.1:{port}");

    if let Ok(mut mux) =
        connect_harness_ws(&format!("{base}/api/remote.mux"), cookie.as_deref()).await
    {
        // The mux is a passive multiplexer: without opening the `$events`
        // logical stream it stays silent forever, which is exactly why the
        // notifications went dark on dsh 0.1.2+.
        mux.send(tokio_tungstenite::tungstenite::Message::text(
            events_stream_open_frame(),
        ))
        .await
        .map_err(|error| error.to_string())?;
        // A mux that refuses the forwarded-event stream before delivering a
        // single item is a generation this adapter does not speak; fall
        // through to the legacy endpoints instead of reconnecting forever.
        let mut stream_delivered = false;
        let mut stream_refused = false;
        loop {
            // Watching the deadline alongside the socket is what lets a
            // completion wait: the frames that decide whether it is overtaken
            // arrive on this very stream.
            let notices = tokio::select! {
                biased;
                frame = mux.next() => match frame {
                    Some(Ok(message)) => {
                        touch_harness_activity(state);
                        let Ok(text) = message.to_text() else {
                            continue;
                        };
                        match classify_mux_message(text) {
                            HarnessInbound::Notice(notice) => {
                                stream_delivered = true;
                                ledger.route(notice, Instant::now())
                            }
                            HarnessInbound::StreamFailed => {
                                stream_refused = true;
                                break;
                            }
                            HarnessInbound::Ignore => Vec::new(),
                        }
                    }
                    Some(Err(error)) => return Err(error.to_string()),
                    None => break,
                },
                _ = wait_until_deadline(ledger.deadline()) => ledger.due(Instant::now()),
            };
            for notice in notices {
                deliver_harness_notice(app, state, notice);
            }
        }
        if !stream_refused || stream_delivered {
            return Ok(());
        }
    }

    let mut mux = connect_harness_ws(&format!("{base}/api/events.mux"), cookie.as_deref()).await?;
    let mut host =
        connect_harness_ws(&format!("{base}/api/events.host"), cookie.as_deref()).await?;
    loop {
        let notices = tokio::select! {
            biased;
            frame = mux.next() => match frame {
                Some(Ok(message)) => {
                    touch_harness_activity(state);
                    match message.to_text() {
                        Ok(text) => route_legacy_mux_frame(text, ledger),
                        Err(_) => Vec::new(),
                    }
                }
                Some(Err(error)) => return Err(error.to_string()),
                None => return Ok(()),
            },
            frame = host.next() => match frame {
                Some(Ok(message)) => {
                    touch_harness_activity(state);
                    match message.to_text() {
                        Ok(text) => route_legacy_host_frame(text, ledger),
                        Err(_) => Vec::new(),
                    }
                }
                Some(Err(error)) => return Err(error.to_string()),
                None => return Ok(()),
            },
            _ = wait_until_deadline(ledger.deadline()) => ledger.due(Instant::now()),
        };
        for notice in notices {
            deliver_harness_notice(app, state, notice);
        }
    }
}

/// The stream id the shell uses for the forwarded-event logical stream.
const EVENTS_STREAM_ID: &str = "desk-events";

/// The `open` frame that subscribes to dsh 0.1.2+ forwarded events on the
/// Remote mux. The gateway validates the endpoint name and the exact
/// `{args:{}}` payload, so both are fixed by the stream protocol.
fn events_stream_open_frame() -> String {
    serde_json::json!({
        "type": "open",
        "streamId": EVENTS_STREAM_ID,
        "endpoint": "$events",
        "payload": { "args": {} }
    })
    .to_string()
}

/// The shell's semantic view of Harness activity. Every dsh generation
/// encodes the same facts with different wire shapes and event names; the
/// adapters translate into these variants so dedup, preferences, focus
/// checks, and notification text stay independent of the wire protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HarnessNotice {
    /// A question, plan review or approval needs the user before the session can proceed.
    InteractionRequested {
        key: String,
        kind: NoticeKind,
        session_id: Option<String>,
        detail: Option<String>,
    },
    /// A session switched between running and idle.
    SessionRunningChanged { session_id: String, running: bool },
    /// An agent failed outside a durable turn position.
    SessionFailed {
        session_id: String,
        detail: Option<String>,
    },
    /// A session is gone; whatever is still tracked for it belongs to nothing.
    SessionRemoved { session_id: String },
    /// A completion that waited out the debounce without being overtaken by an
    /// interaction. Only the ledger derives this one.
    TaskCompleted { session_id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeKind {
    Question,
    PlanReview,
    Approval,
}

impl HarnessNotice {
    /// The session a notice belongs to, where the wire payload says.
    fn session_id(&self) -> Option<&str> {
        match self {
            HarnessNotice::InteractionRequested { session_id, .. } => session_id.as_deref(),
            HarnessNotice::SessionRunningChanged { session_id, .. }
            | HarnessNotice::SessionFailed { session_id, .. }
            | HarnessNotice::SessionRemoved { session_id, .. }
            | HarnessNotice::TaskCompleted { session_id } => Some(session_id),
        }
    }
}

/// How long a completion waits before it fires. A question, an approval or a
/// failure that arrives right after "idle" describes the same turn of work, and
/// one turn gets one banner: `error > interaction > completed`.
const COMPLETION_DEBOUNCE: Duration = Duration::from_millis(300);

/// What the watcher remembers about one Harness generation: which interactions
/// already produced a banner, which sessions are running, and which completions
/// are still waiting out the debounce.
#[derive(Default)]
struct AttentionLedger {
    seen_interactions: VecDeque<String>,
    running_sessions: HashMap<String, bool>,
    pending_completions: HashMap<String, Instant>,
    /// Sessions whose current turn already got a banner. A turn that ends in a
    /// question, an approval or a failure still reports "idle" right afterwards,
    /// and that idle must not stack a second banner onto the same turn.
    attended_sessions: HashSet<String>,
}

impl AttentionLedger {
    /// Route one semantic notice through dedup, running-state tracking and the
    /// completion debounce, returning the notices that deserve a banner now.
    fn route(&mut self, notice: HarnessNotice, now: Instant) -> Vec<HarnessNotice> {
        match &notice {
            HarnessNotice::InteractionRequested { key, .. } => {
                if !is_new_interaction(&mut self.seen_interactions, key) {
                    return Vec::new();
                }
            }
            HarnessNotice::SessionRunningChanged {
                session_id,
                running,
            } => {
                let previous = self.running_sessions.insert(session_id.clone(), *running);
                if *running {
                    // The agent is working again, so whatever covered the
                    // previous turn says nothing about this one.
                    self.attended_sessions.remove(session_id);
                    self.pending_completions.remove(session_id);
                    return Vec::new();
                }
                // The first idle of a session is only a baseline: a page that
                // just connected reports every already-idle session, and those
                // are not tasks the user was waiting on.
                if previous == Some(true) && !self.attended_sessions.contains(session_id) {
                    self.pending_completions
                        .insert(session_id.clone(), now + COMPLETION_DEBOUNCE);
                }
                return Vec::new();
            }
            HarnessNotice::SessionRemoved { session_id } => {
                self.running_sessions.remove(session_id);
                self.pending_completions.remove(session_id);
                self.attended_sessions.remove(session_id);
                return Vec::new();
            }
            HarnessNotice::SessionFailed { .. } | HarnessNotice::TaskCompleted { .. } => {}
        }
        // Anything that fires from here covers the turn it belongs to. An
        // interaction the wire does not attribute to a session silences every
        // waiting completion: dropping one is a missed reminder, but firing
        // both is two banners for one turn, and that is the noisier mistake.
        match notice.session_id() {
            Some(session_id) => {
                self.attended_sessions.insert(session_id.to_string());
                self.pending_completions.remove(session_id);
            }
            None => self.pending_completions.clear(),
        }
        vec![notice]
    }

    /// The next deferred completion's deadline, if anything is waiting.
    fn deadline(&self) -> Option<Instant> {
        self.pending_completions.values().min().copied()
    }

    /// Completions whose wait has ended, in session order for determinism.
    fn due(&mut self, now: Instant) -> Vec<HarnessNotice> {
        let mut ready: Vec<String> = self
            .pending_completions
            .iter()
            .filter(|(_, deadline)| **deadline <= now)
            .map(|(session_id, _)| session_id.clone())
            .collect();
        ready.sort();
        ready
            .into_iter()
            .filter_map(|session_id| {
                self.pending_completions.remove(&session_id);
                (!self.attended_sessions.contains(&session_id))
                    .then_some(HarnessNotice::TaskCompleted { session_id })
            })
            .collect()
    }

    /// Forget a Harness generation's session state; the next backend says
    /// nothing about the one before it.
    fn reset(&mut self) {
        self.running_sessions.clear();
        self.pending_completions.clear();
        self.attended_sessions.clear();
    }
}

/// Wait for the next completion deadline, or park forever when none is waiting.
async fn wait_until_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => {
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
        }
        None => futures_util::future::pending::<()>().await,
    }
}

/// Deliver one routed notice: the badge and the banner the user asked for.
fn deliver_harness_notice(app: &AppHandle, state: &HarnessState, notice: HarnessNotice) {
    match notice {
        HarnessNotice::InteractionRequested { kind, detail, .. } => {
            on_needs_interaction(app, state, kind, detail.as_deref())
        }
        HarnessNotice::SessionFailed { detail, .. } => {
            on_task_failed(app, state, detail.as_deref())
        }
        HarnessNotice::TaskCompleted { session_id } => on_task_completed(app, state, &session_id),
        // Bookkeeping the ledger already consumed; nothing fires from these.
        HarnessNotice::SessionRunningChanged { .. } | HarnessNotice::SessionRemoved { .. } => {}
    }
}

/// What one raw remote.mux message means to the shell.
#[derive(Debug, PartialEq, Eq)]
enum HarnessInbound {
    Notice(HarnessNotice),
    /// The event logical stream ended or errored.
    StreamFailed,
    /// Heartbeats, stream bookkeeping, and events the shell does not use.
    /// Newer dsh releases may forward additional events; ignoring unknown
    /// names keeps the shell forward compatible.
    Ignore,
}

/// Classify one raw remote.mux WebSocket message without side effects. Each
/// mux message wraps one chunk of a logical stream: `{type:'item', streamId,
/// value}` for data, plus `end`/`error` bookkeeping.
fn classify_mux_message(text: &str) -> HarnessInbound {
    let Ok(envelope) = serde_json::from_str::<serde_json::Value>(text) else {
        return HarnessInbound::Ignore;
    };
    match envelope.get("type").and_then(|value| value.as_str()) {
        Some("item") => {
            classify_forwarded_value(envelope.get("value").unwrap_or(&serde_json::Value::Null))
        }
        // The events logical stream is the only one the shell opens; when it
        // ends or errors there is nothing left to observe on this socket.
        Some("end") | Some("error") => HarnessInbound::StreamFailed,
        // `ready` (stream provenance) and anything new are bookkeeping.
        _ => HarnessInbound::Ignore,
    }
}

/// Classify one forwarded-event stream item. The gateway delivers broadcast
/// events as `{type:'emit', event, args:[...]}` and interaction requests as
/// `{type:'waterfall', event, eventId, request}`.
fn classify_forwarded_value(value: &serde_json::Value) -> HarnessInbound {
    let Some(event) = value.get("event").and_then(|value| value.as_str()) else {
        // `ready` and `cancel` frames carry no `event` name.
        return HarnessInbound::Ignore;
    };
    match forwarded_event_kind(event) {
        Some(ForwardedEventKind::Question) => {
            let request = value.get("request").unwrap_or(&serde_json::Value::Null);
            let event_id = value.get("eventId").and_then(|value| value.as_str()).unwrap_or("");
            let detail = first_question_text(request);
            // A plan review is a question with a decision attached; it needs the
            // user the same way, but the banner should say what is waiting.
            let kind = if questions_declare_plan_review(request) {
                NoticeKind::PlanReview
            } else {
                NoticeKind::Question
            };
            HarnessInbound::Notice(HarnessNotice::InteractionRequested {
                key: interaction_key(request, event_id),
                kind,
                session_id: request_session_id(request),
                detail,
            })
        }
        Some(ForwardedEventKind::Approval) => {
            let request = value.get("request").unwrap_or(&serde_json::Value::Null);
            let event_id = value.get("eventId").and_then(|value| value.as_str()).unwrap_or("");
            HarnessInbound::Notice(HarnessNotice::InteractionRequested {
                key: interaction_key(request, event_id),
                kind: NoticeKind::Approval,
                session_id: request_session_id(request),
                detail: approval_detail(request),
            })
        }
        Some(ForwardedEventKind::SessionStatus) => {
            // `api-session/status` emits `[sessionId, running]`.
            let args = forwarded_args(value);
            match (
                first_session_id(&args),
                args.get(1).and_then(|v| v.as_bool()),
            ) {
                (Some(session_id), Some(running)) => {
                    HarnessInbound::Notice(HarnessNotice::SessionRunningChanged {
                        session_id,
                        running,
                    })
                }
                _ => HarnessInbound::Ignore,
            }
        }
        Some(ForwardedEventKind::SessionError) => {
            // `api-session/error` emits `[sessionId, message]`, the message a
            // user-safe failure chain.
            let args = forwarded_args(value);
            match first_session_id(&args) {
                Some(session_id) => HarnessInbound::Notice(HarnessNotice::SessionFailed {
                    session_id,
                    detail: args
                        .get(1)
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                        .filter(|text| !text.is_empty()),
                }),
                None => HarnessInbound::Ignore,
            }
        }
        Some(ForwardedEventKind::SessionRemoved) => {
            // `api-session/removed` emits `[sessionId]`.
            match first_session_id(&forwarded_args(value)) {
                Some(session_id) => {
                    HarnessInbound::Notice(HarnessNotice::SessionRemoved { session_id })
                }
                None => HarnessInbound::Ignore,
            }
        }
        None => HarnessInbound::Ignore,
    }
}

/// The positional arguments of a forwarded `emit` frame.
fn forwarded_args(value: &serde_json::Value) -> Vec<serde_json::Value> {
    value
        .get("args")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
}

/// The session id of a forwarded `api-session/*` frame, when it is a non-empty
/// string in the leading position the gateway documents.
fn first_session_id(args: &[serde_json::Value]) -> Option<String> {
    args.first()
        .and_then(|value| value.as_str())
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Whether any question in an interaction asks for a plan review. `intent` is
/// optional presentation metadata, so an absent one is an ordinary question.
fn questions_declare_plan_review(request: &serde_json::Value) -> bool {
    request
        .get("questions")
        .and_then(|value| value.as_array())
        .map(|items| {
            items.iter().any(|item| {
                item.get("intent")
                    .and_then(|intent| intent.get("kind"))
                    .and_then(|kind| kind.as_str())
                    == Some("plan-review")
            })
        })
        .unwrap_or(false)
}

/// The session an interaction belongs to, where the payload carries one. Older
/// generations put it on the request directly; newer ones project it through the
/// agent identity.
fn request_session_id(request: &serde_json::Value) -> Option<String> {
    [
        request.get("sessionId"),
        request.get("session_id"),
        request
            .get("agent")
            .and_then(|agent| agent.get("sessionId")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|value| value.as_str())
    .find(|text| !text.is_empty())
    .map(str::to_string)
}

/// Forwarded events the shell reacts to, matched tolerantly across the names
/// each dsh generation has used for the same fact. The current allowlist is
/// dsh 0.1.2–0.1.5's `dsh-api-remotes` forwarding table.
enum ForwardedEventKind {
    Question,
    Approval,
    SessionStatus,
    SessionError,
    SessionRemoved,
}

fn forwarded_event_kind(event: &str) -> Option<ForwardedEventKind> {
    match event {
        "user-questions/request" | "question/requested" => Some(ForwardedEventKind::Question),
        "approval/request" | "approval/requested" => Some(ForwardedEventKind::Approval),
        "api-session/status" | "host/session-status" => Some(ForwardedEventKind::SessionStatus),
        "api-session/error" => Some(ForwardedEventKind::SessionError),
        "api-session/removed" => Some(ForwardedEventKind::SessionRemoved),
        _ => None,
    }
}

fn first_question_text(request: &serde_json::Value) -> Option<String> {
    request
        .get("questions")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("question"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

fn approval_detail(request: &serde_json::Value) -> Option<String> {
    request
        .get("toolName")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

/// Route one raw legacy `events.mux` envelope (`{rpcId, payload:{type,...}}`).
fn route_legacy_mux_frame(text: &str, ledger: &mut AttentionLedger) -> Vec<HarnessNotice> {
    match classify_legacy_mux_frame(text) {
        Some(notice) => ledger.route(notice, Instant::now()),
        None => Vec::new(),
    }
}

/// Classify one raw legacy `events.mux` envelope without side effects.
fn classify_legacy_mux_frame(text: &str) -> Option<HarnessNotice> {
    let envelope = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let payload = envelope.get("payload")?;
    let event_type = payload.get("type").and_then(|value| value.as_str())?;
    let rpc_id = envelope
        .get("rpcId")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    match event_type {
        "question/requested" => Some(HarnessNotice::InteractionRequested {
            key: interaction_key(payload, rpc_id),
            kind: if questions_declare_plan_review(payload) {
                NoticeKind::PlanReview
            } else {
                NoticeKind::Question
            },
            session_id: request_session_id(payload),
            detail: first_question_text(payload),
        }),
        "approval/requested" => Some(HarnessNotice::InteractionRequested {
            key: interaction_key(payload, rpc_id),
            kind: NoticeKind::Approval,
            session_id: request_session_id(payload),
            detail: None,
        }),
        _ => None,
    }
}

/// Route one raw legacy `events.host` envelope.
fn route_legacy_host_frame(text: &str, ledger: &mut AttentionLedger) -> Vec<HarnessNotice> {
    match classify_legacy_host_frame(text) {
        Some(notice) => ledger.route(notice, Instant::now()),
        None => Vec::new(),
    }
}

/// Classify one raw legacy `events.host` envelope without side effects.
fn classify_legacy_host_frame(text: &str) -> Option<HarnessNotice> {
    let envelope = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let payload = envelope.get("payload")?;
    if payload.get("type").and_then(|value| value.as_str()) != Some("host/session-status") {
        return None;
    }
    let running = payload
        .get("running")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let session_id = payload
        .get("sessionId")
        .and_then(|value| value.as_str())?;
    if session_id.is_empty() {
        return None;
    }
    Some(HarnessNotice::SessionRunningChanged {
        session_id: session_id.to_string(),
        running,
    })
}

/// Stable dedup key for a pending interaction: approval id when present,
/// otherwise the first question id, otherwise the wire rpcId. Survives
/// reconnect replay (the harness re-emits still-pending requests).
fn interaction_key(payload: &serde_json::Value, rpc_id: &str) -> String {
    if let Some(approval_id) = payload.get("approvalId").and_then(|value| value.as_str()) {
        return format!("a:{approval_id}");
    }
    if let Some(question_id) = payload
        .get("questions")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("id"))
        .and_then(|value| value.as_str())
    {
        return format!("q:{question_id}");
    }
    format!("q:{rpc_id}")
}

fn is_new_interaction(seen: &mut VecDeque<String>, key: &str) -> bool {
    if seen.iter().any(|entry| entry == key) {
        return false;
    }
    seen.push_back(key.to_string());
    while seen.len() > MAX_SEEN_INTERACTIONS {
        seen.pop_front();
    }
    true
}

fn window_is_focused(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|window| window.is_focused().ok())
        .unwrap_or(false)
}

fn raise_attention(app: &AppHandle, state: &HarnessState, title: &str, body: &str) {
    let count = state.pending_attention.fetch_add(1, Ordering::AcqRel) + 1;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_badge_count(Some(count));
    }
    post_system_notification(app, state, title, body);
}

/// The shell's chosen UI language ("zh" or "en"), synced from the frontend.
fn current_language(state: &HarnessState) -> String {
    state
        .language
        .lock()
        .map(|language| language.clone())
        .unwrap_or_else(|_| "zh".to_string())
}

/// Whether a banner may go out at all: the master switch, this category's
/// switch, and the user not already looking at the window.
fn banners_allowed(app: &AppHandle, state: &HarnessState, category: &AtomicBool) -> bool {
    state.notify_enabled.load(Ordering::Acquire)
        && category.load(Ordering::Acquire)
        && !window_is_focused(app)
}

/// Text lifted out of the Harness payload — a question's wording, a failure's
/// message — is opt-in. A banner stays readable on the lock screen, where the
/// project name, the command and the question are nobody's business but the
/// user's.
fn notice_detail<'a>(state: &HarnessState, detail: &'a str) -> Option<&'a str> {
    state
        .notify_detail
        .load(Ordering::Acquire)
        .then_some(detail)
}

fn on_needs_interaction(
    app: &AppHandle,
    state: &HarnessState,
    kind: NoticeKind,
    detail: Option<&str>,
) {
    if !banners_allowed(app, state, &state.notify_interaction) {
        return;
    }
    let english = current_language(state) == "en";
    let (title, fallback) = match (kind, english) {
        (NoticeKind::Question, true) => ("Needs your input", "Harness is waiting for your answer"),
        (NoticeKind::Question, false) => ("需要你的输入", "Harness 正在等待你的回答"),
        (NoticeKind::PlanReview, true) => (
            "Plan needs your review",
            "Harness has a plan waiting for your approval",
        ),
        (NoticeKind::PlanReview, false) => ("计划等待你的确认", "Harness 有一份计划需要你确认"),
        (NoticeKind::Approval, true) => (
            "Needs your approval",
            "Harness needs your approval to continue",
        ),
        (NoticeKind::Approval, false) => ("需要你的批准", "Harness 需要你的批准才能继续"),
    };
    // Only a plain question gains anything from being quoted back; an approval
    // already says what it needs, and a plan review's first question is just the
    // plan heading.
    let preview = match kind {
        NoticeKind::Question => detail.and_then(|detail| notice_detail(state, detail)),
        NoticeKind::PlanReview | NoticeKind::Approval => None,
    };
    raise_attention(app, state, title, preview.unwrap_or(fallback));
}

fn on_task_failed(app: &AppHandle, state: &HarnessState, detail: Option<&str>) {
    if !banners_allowed(app, state, &state.notify_error) {
        return;
    }
    let english = current_language(state) == "en";
    let (title, fallback) = if english {
        ("Task failed", "The Harness task ended with an error")
    } else {
        ("任务执行失败", "Harness 的任务执行失败了")
    };
    let body = detail
        .and_then(|detail| notice_detail(state, detail))
        .unwrap_or(fallback);
    raise_attention(app, state, title, body);
}

fn on_task_completed(app: &AppHandle, state: &HarnessState, session_id: &str) {
    if !banners_allowed(app, state, &state.notify_task_completed) {
        return;
    }
    let english = current_language(state) == "en";
    let title = if english {
        "Task completed"
    } else {
        "任务已完成"
    };
    let body = if session_id.is_empty() {
        if english {
            "The Harness finished a task".to_string()
        } else {
            "Harness 已完成一项任务".to_string()
        }
    } else if english {
        format!("Session {session_id} finished its task")
    } else {
        format!("会话 {session_id} 已完成任务")
    };
    raise_attention(app, state, title, &body);
}

fn clear_badge<R: tauri::Runtime>(app: &AppHandle<R>, state: &HarnessState) {
    state.pending_attention.store(0, Ordering::Release);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_badge_count(None);
    }
}

/// Deliver a system notification through the first transport that works.
/// macOS talks to UNUserNotificationCenter directly because the notification
/// plugin still rides the NSUserNotification API Apple removed — on modern
/// systems everything it posts is silently dropped. The plugin stays as the
/// transport for the other desktop platforms.
fn post_system_notification(app: &AppHandle, state: &HarnessState, title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    match macos_notification::post(title, body) {
        Ok(macos_notification::PostOutcome::Sent) => return,
        Ok(macos_notification::PostOutcome::NotAuthorized) => {
            // A denied permission is a user decision, not a failure, but a
            // silent one: recording it once makes "why did nothing appear"
            // answerable from the log page.
            emit_log(
                app,
                &state.logs,
                "desk",
                "系统通知权限未开启，本次提醒只更新了角标。",
            );
            return;
        }
        Err(error) => {
            emit_log(
                app,
                &state.logs,
                "desk",
                format!("发送系统通知失败：{error}"),
            );
            return;
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = state;
        let _ = app.notification().builder().title(title).body(body).show();
    }
}

/// macOS delivery over UNUserNotificationCenter, the only notification
/// transport Apple still supports. `tauri-plugin-notification` depends on
/// `mac-notification-sys`, whose NSUserNotification API was removed by
/// macOS, so nothing it posts on modern systems ever shows up.
#[cfg(target_os = "macos")]
mod macos_notification {
    use std::panic::AssertUnwindSafe;
    use std::sync::OnceLock;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::exception::catch;
    use objc2::rc::Retained;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
        UNNotificationRequest, UNNotificationSettings, UNNotificationSound,
        UNUserNotificationCenter,
    };
    use std::ptr::NonNull;

    const STATUS_TIMEOUT: Duration = Duration::from_secs(2);
    const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
    const ADD_TIMEOUT: Duration = Duration::from_secs(5);

    /// Stable request identifier; UN replaces the previous banner with the
    /// same id instead of stacking duplicates for one pending interaction.
    const REQUEST_IDENTIFIER: &str = "com.deepseek.harnessdesk.notice";

    /// Set once this process has seen a real grant, so repeated notices never
    /// re-read the permission. Only a grant is cached: a prompt the user has
    /// not answered yet, or a denial they later lift in System Settings, must
    /// not mute every banner for the rest of the run.
    static GRANTED: OnceLock<()> = OnceLock::new();

    /// Whether the authorization dialog has been issued, so no second notice
    /// waits behind a copy of the same prompt.
    static PROMPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    /// What happened to one posted banner.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PostOutcome {
        Sent,
        /// macOS is holding the app's notifications; only the badge updates.
        NotAuthorized,
    }

    /// Why the authorization request did not end in a grant. The distinction
    /// matters when diagnosing a missing banner: a denial is a user decision,
    /// while a missing callback means the system never handled the request at
    /// all (which is what an unsigned or unregistered build looks like).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum AuthFailure {
        Denied,
        NoResponse,
        Unavailable,
    }

    /// Make sure macOS would deliver a banner now, asking once per process if
    /// it has not decided yet. Safe to call from any thread.
    pub fn ensure_authorized() -> Result<(), AuthFailure> {
        if GRANTED.get().is_some() {
            return Ok(());
        }
        // Reading the live status is what keeps this recoverable: it answers
        // immediately once the user has decided, one way or the other.
        let (determined, granted) = authorization_status();
        if granted {
            let _ = GRANTED.set(());
            return Ok(());
        }
        if determined {
            return Err(AuthFailure::Denied);
        }
        if PROMPTED.swap(true, std::sync::atomic::Ordering::AcqRel) {
            // The dialog is still up; a second request would only block here.
            return Err(AuthFailure::NoResponse);
        }
        match request_authorization() {
            true => {
                let _ = GRANTED.set(());
                Ok(())
            }
            false => Err(AUTHORIZATION_FAILURE
                .get()
                .copied()
                .unwrap_or(AuthFailure::Denied)),
        }
    }

    /// Put the dialog back on the table. The settings page's test button is how
    /// a prompt dismissed by accident gets answered, so it must not be
    /// suppressed by the one-shot guard the background notices use.
    pub fn ask_for_authorization_again() {
        PROMPTED.store(false, std::sync::atomic::Ordering::Release);
    }

    /// Records why the one-shot authorization attempt failed, for the log.
    static AUTHORIZATION_FAILURE: OnceLock<AuthFailure> = OnceLock::new();

    fn request_authorization() -> bool {
        let Some(center) = current_center() else {
            let _ = AUTHORIZATION_FAILURE.set(AuthFailure::Unavailable);
            return false;
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        // The callback arrives with the granted flag; a `false` here is either
        // an explicit denial or a request the system never answered.
        let block = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            let _ = sender.send(granted.as_bool());
        });
        center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert
                | UNAuthorizationOptions::Badge
                | UNAuthorizationOptions::Sound,
            &block,
        );
        match receiver.recv_timeout(AUTH_TIMEOUT) {
            Ok(granted) => {
                if !granted {
                    let _ = AUTHORIZATION_FAILURE.set(AuthFailure::Denied);
                }
                granted
            }
            Err(_) => {
                let _ = AUTHORIZATION_FAILURE.set(AuthFailure::NoResponse);
                false
            }
        }
    }

    /// Post one banner, reporting why nothing appeared when it could not.
    pub fn post(title: &str, body: &str) -> Result<PostOutcome, String> {
        let Some(center) = center_or_error()? else {
            return Err("当前进程没有可用的系统通知中心。".to_string());
        };
        if ensure_authorized().is_err() {
            return Ok(PostOutcome::NotAuthorized);
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(REQUEST_IDENTIFIER),
            &content,
            None,
        );
        // Wait for the completion callback: it is the only signal that the
        // request actually reached the system, and a failure there is worth
        // reporting instead of pretending the banner went out.
        let (sender, receiver) = std::sync::mpsc::channel();
        let block = RcBlock::new(move |error: *mut NSError| {
            let message = if error.is_null() {
                None
            } else {
                // SAFETY: the framework owns this NSError for the duration of
                // the callback; only its description is read out here.
                unsafe { error.as_ref() }
                    .map(|error| error.localizedDescription().to_string())
            };
            let _ = sender.send(message);
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&block));
        match receiver.recv_timeout(ADD_TIMEOUT) {
            Ok(None) => Ok(PostOutcome::Sent),
            Ok(Some(message)) => Err(message),
            Err(_) => Err("等待系统确认通知投递超时。".to_string()),
        }
    }

    /// Live authorization status for the settings page: `(determined, granted)`.
    /// Reading happens on demand so a grant added in System Settings after a
    /// denial is reflected without relaunching the app.
    pub fn authorization_status() -> (bool, bool) {
        let Some(center) = current_center() else {
            return (false, false);
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let block = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            // SAFETY: the framework hands this pointer to the block for
            // reading only, and it stays valid for the duration of the call.
            let status = unsafe { settings.as_ref().authorizationStatus() };
            let _ = sender.send(status);
        });
        center.getNotificationSettingsWithCompletionHandler(&block);
        match receiver.recv_timeout(STATUS_TIMEOUT) {
            Ok(status) => (
                status != UNAuthorizationStatus::NotDetermined,
                status == UNAuthorizationStatus::Authorized
                    || status == UNAuthorizationStatus::Provisional
                    || status == UNAuthorizationStatus::Ephemeral,
            ),
            Err(_) => (false, false),
        }
    }

    /// The shared notification center, or None where the process cannot host
    /// one. Outside a proper app bundle (e.g. `tauri dev`) the framework
    /// raises an Objective-C exception instead of returning nil, so catch it.
    fn current_center() -> Option<Retained<UNUserNotificationCenter>> {
        catch(AssertUnwindSafe(
            UNUserNotificationCenter::currentNotificationCenter,
        ))
        .ok()
    }

    /// The notification center, with the two failure modes told apart:
    /// `Ok(None)` means this process has none (unbundled dev build).
    fn center_or_error() -> Result<Option<Retained<UNUserNotificationCenter>>, String> {
        match catch(AssertUnwindSafe(
            UNUserNotificationCenter::currentNotificationCenter,
        )) {
            Ok(center) => Ok(Some(center)),
            Err(_) => Ok(None),
        }
    }
}

#[tauri::command]
fn harness_status(state: State<'_, HarnessState>) -> HarnessStatus {
    snapshot(&state)
}

#[tauri::command]
fn runtime_status(app: AppHandle, state: State<'_, HarnessState>) -> RuntimeStatus {
    runtime_status_snapshot(&app, &state)
}

#[tauri::command]
fn set_notification_prefs(
    state: State<'_, HarnessState>,
    enabled: bool,
    task_completed: bool,
    interaction: bool,
    error: bool,
    detail: bool,
) {
    state.notify_enabled.store(enabled, Ordering::Release);
    state
        .notify_task_completed
        .store(task_completed, Ordering::Release);
    state
        .notify_interaction
        .store(interaction, Ordering::Release);
    state.notify_error.store(error, Ordering::Release);
    state.notify_detail.store(detail, Ordering::Release);
}

#[tauri::command]
fn notification_prefs(state: State<'_, HarnessState>) -> NotificationPrefsView {
    NotificationPrefsView {
        enabled: state.notify_enabled.load(Ordering::Acquire),
        task_completed: state.notify_task_completed.load(Ordering::Acquire),
        interaction: state.notify_interaction.load(Ordering::Acquire),
        error: state.notify_error.load(Ordering::Acquire),
        detail: state.notify_detail.load(Ordering::Acquire),
    }
}

/// What the platform reports about the shell's notification permission.
/// `determined` stays false while there is nothing to read (dev builds
/// without an app bundle) or the system has not been asked yet.
#[derive(Serialize)]
struct NotificationPermission {
    granted: bool,
    determined: bool,
}

#[tauri::command]
fn notification_permission() -> NotificationPermission {
    #[cfg(target_os = "macos")]
    {
        let (determined, granted) = macos_notification::authorization_status();
        NotificationPermission {
            granted,
            determined,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        NotificationPermission {
            granted: true,
            determined: true,
        }
    }
}

/// Post one banner so the user can confirm the transport end to end (and see
/// the permission prompt land when macOS has not asked yet).
#[tauri::command]
fn send_test_notification(app: AppHandle, state: State<'_, HarnessState>) {
    let english = current_language(&state) == "en";
    let (title, body) = if english {
        (
            "Notifications are working",
            "This is how a task completion or a pending question will look.",
        )
    } else {
        ("通知已就绪", "任务完成或需要你回应时，就会收到这样的提醒。")
    };
    // The test button must not be gated by the per-category preferences: it
    // exists precisely to check the transport those preferences feed.
    #[cfg(target_os = "macos")]
    macos_notification::ask_for_authorization_again();
    post_system_notification(&app, &state, title, body);
}

/// Open the macOS pane where notification permission is granted. The system
/// has no per-app deep link, so the shell lands on the notifications pane
/// and the user picks the app there.
#[tauri::command]
fn open_notification_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
            .status()
            .map_err(|error| format!("无法打开系统设置：{error}"))?
            .success()
            .then_some(())
            .ok_or_else(|| "无法打开系统设置。".to_string())
    }
    #[cfg(not(target_os = "macos"))]
    Err("只有 macOS 提供系统通知设置。".to_string())
}

#[tauri::command]
fn set_language(app: AppHandle, state: State<'_, HarnessState>, language: String) {
    let language = if language == "en" { "en" } else { "zh" };
    let changed = {
        let Ok(mut current) = state.language.lock() else {
            return;
        };
        if *current == language {
            false
        } else {
            *current = language.to_string();
            true
        }
    };
    if !changed {
        return;
    }
    // Rebuild the tray menu so its labels follow the new language.
    #[cfg(feature = "tray-icon")]
    {
        let _ = app.remove_tray_by_id("main-tray");
        if let Err(error) = build_tray(&app, language) {
            emit_log(
                &app,
                &state.logs,
                "desk",
                format!("重建托盘菜单失败：{error}"),
            );
        }
    }
}

#[tauri::command]
async fn install_runtime(
    app: AppHandle,
    state: State<'_, HarnessState>,
) -> Result<RuntimeStatus, String> {
    install_runtime_inner(&app, &state).await?;
    Ok(runtime_status_snapshot(&app, &state))
}

#[tauri::command]
async fn check_app_update() -> Result<AppUpdateStatus, String> {
    let release = fetch_latest_release().await?;
    Ok(app_update_status(&release))
}

#[tauri::command]
async fn install_app_update(
    app: AppHandle,
    state: State<'_, HarnessState>,
) -> Result<AppUpdateStatus, String> {
    install_app_update_inner(&app, &state).await
}

#[tauri::command]
async fn check_dsh_update(
    app: AppHandle,
    channel: Option<String>,
) -> Result<DshUpdateStatus, String> {
    check_dsh_update_inner(&app, DshUpdateChannel::from_request(channel.as_deref())).await
}

#[tauri::command]
async fn install_dsh_update(
    app: AppHandle,
    state: State<'_, HarnessState>,
    version: String,
) -> Result<RuntimeStatus, String> {
    // "更新 dsh" always means moving forward, so any earlier pin is dropped and
    // the freshly installed version wins.
    install_dsh_version_inner(&app, &state, version, None).await
}

#[tauri::command]
async fn list_dsh_versions(app: AppHandle) -> Result<DshVersionsStatus, String> {
    list_dsh_versions_inner(&app).await
}

/// Switch to (and pin) one dsh version, installing it from npm when needed.
#[tauri::command]
async fn set_dsh_version(
    app: AppHandle,
    state: State<'_, HarnessState>,
    version: String,
) -> Result<RuntimeStatus, String> {
    activate_dsh_version_inner(&app, &state, version).await
}

/// Drop the pin and let the newest installed dsh version take over again.
#[tauri::command]
async fn follow_latest_dsh_version(
    app: AppHandle,
    state: State<'_, HarnessState>,
) -> Result<RuntimeStatus, String> {
    follow_latest_dsh_version_inner(&app, &state).await
}

#[tauri::command]
fn open_release_page(url: String) -> Result<(), String> {
    open_external_url(&url)
}

#[tauri::command]
fn open_releases_page() -> Result<(), String> {
    open_external_url(RELEASES_PAGE_URL)
}

fn open_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| format!("创建目录失败：{error}"))?;
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(path).status();
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer.exe").arg(path).status();
    #[cfg(target_os = "linux")]
    let result = Command::new("xdg-open").arg(path).status();
    result
        .map_err(|error| format!("打开目录失败：{error}"))?
        .success()
        .then_some(())
        .ok_or_else(|| "打开目录失败。".to_string())
}

fn logs_directory(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .map(|path| path.join("logs"))
        .unwrap_or_else(|_| preferred_runtime_root(app).join("../logs"))
}

#[tauri::command]
fn open_runtime_directory(app: AppHandle) -> Result<(), String> {
    open_directory(&preferred_runtime_root(&app))
}

#[tauri::command]
fn open_logs_directory(app: AppHandle) -> Result<(), String> {
    open_directory(&logs_directory(&app))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[tauri::command]
fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    let executable = env::current_exe().map_err(|error| format!("获取应用路径失败：{error}"))?;

    #[cfg(target_os = "macos")]
    {
        let home = home_directory().ok_or("无法确定用户目录。")?;
        let agents = home.join("Library/LaunchAgents");
        let plist = agents.join("com.deepseek.harnessdesk.plist");
        if enabled {
            fs::create_dir_all(&agents)
                .map_err(|error| format!("创建登录启动目录失败：{error}"))?;
            let contents = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>com.deepseek.harnessdesk</string><key>ProgramArguments</key><array><string>{}</string></array><key>RunAtLoad</key><true/></dict></plist>\n",
                xml_escape(&executable.to_string_lossy())
            );
            fs::write(&plist, contents)
                .map_err(|error| format!("写入登录启动配置失败：{error}"))?;
        } else if plist.exists() {
            fs::remove_file(&plist).map_err(|error| format!("删除登录启动配置失败：{error}"))?;
        }
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let key = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
        if enabled {
            let status = Command::new("reg.exe")
                .args([
                    "ADD",
                    key,
                    "/V",
                    "DeepSeekHarnessDesk",
                    "/T",
                    "REG_SZ",
                    "/D",
                ])
                .arg(executable)
                .arg("/F")
                .status()
                .map_err(|error| format!("设置登录启动失败：{error}"))?;
            if !status.success() {
                return Err("设置登录启动失败。".to_string());
            }
        } else {
            let status = Command::new("reg.exe")
                .args(["DELETE", key, "/V", "DeepSeekHarnessDesk", "/F"])
                .status()
                .map_err(|error| format!("关闭登录启动失败：{error}"))?;
            if !status.success() && status.code() != Some(1) {
                return Err("关闭登录启动失败。".to_string());
            }
        }
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let home = home_directory().ok_or("无法确定用户目录。")?;
        let autostart = home.join(".config/autostart");
        let desktop = autostart.join("deepseek-harness-desk.desktop");
        if enabled {
            fs::create_dir_all(&autostart)
                .map_err(|error| format!("创建登录启动目录失败：{error}"))?;
            let contents = format!(
                "[Desktop Entry]\nType=Application\nName=DeepSeek Harness Desk\nExec=\\\"{}\\\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
                executable.to_string_lossy().replace('"', "\\\"")
            );
            fs::write(&desktop, contents)
                .map_err(|error| format!("写入登录启动配置失败：{error}"))?;
        } else if desktop.exists() {
            fs::remove_file(&desktop).map_err(|error| format!("删除登录启动配置失败：{error}"))?;
        }
        return Ok(());
    }

    #[allow(unreachable_code)]
    Err("当前系统暂不支持登录时启动。".to_string())
}

#[tauri::command]
async fn start_harness(
    app: AppHandle,
    state: State<'_, HarnessState>,
) -> Result<HarnessStatus, String> {
    start_harness_inner(&app, &state).await
}

#[tauri::command]
async fn restart_harness(
    app: AppHandle,
    state: State<'_, HarnessState>,
) -> Result<HarnessStatus, String> {
    stop_harness_inner(&app, &state);
    start_harness_inner(&app, &state).await
}

#[tauri::command]
fn stop_harness(app: AppHandle, state: State<'_, HarnessState>) -> Result<HarnessStatus, String> {
    stop_harness_inner(&app, &state);
    Ok(snapshot(&state))
}

#[tauri::command]
fn harness_logs(state: State<'_, HarnessState>) -> Vec<HarnessLog> {
    state
        .logs
        .lock()
        .map(|logs| logs.iter().cloned().collect())
        .unwrap_or_default()
}

#[tauri::command]
fn clear_harness_logs(state: State<'_, HarnessState>) {
    if let Ok(mut logs) = state.logs.lock() {
        logs.clear();
    }
}

#[tauri::command]
fn window_minimize(window: WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|error| error.to_string())
}

#[tauri::command]
fn window_toggle_maximize(window: WebviewWindow) -> Result<bool, String> {
    if window.is_maximized().map_err(|error| error.to_string())? {
        window.unmaximize().map_err(|error| error.to_string())?;
    } else {
        window.maximize().map_err(|error| error.to_string())?;
    }
    window.is_maximized().map_err(|error| error.to_string())
}

#[tauri::command]
fn window_hide(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|error| error.to_string())
}

#[tauri::command]
fn set_memory_saver(state: State<'_, HarnessState>, enabled: bool) -> Result<(), String> {
    state.memory_saver.store(enabled, Ordering::Release);
    Ok(())
}

#[tauri::command]
fn window_start_dragging(window: WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command]
fn set_dock_visibility(app: AppHandle, visible: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        app.set_dock_visibility(visible)
            .map_err(|error| error.to_string())?;
        let _ = app.show();
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
        app.run_on_main_thread(activate_macos_application)
            .map_err(|error| error.to_string())?;
        let _ = app.emit("window-shown", ());
        let state = app.state::<HarnessState>();
        clear_badge(&app, state.inner());
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, visible);
    }
    Ok(())
}

/// Dock icon styles the shell can pick. `Blue` is the icon that ships inside
/// the app bundle, so it needs no custom Finder icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DockIconVariant {
    Blue,
    Black,
    Avatar,
}

impl DockIconVariant {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "blue" => Some(Self::Blue),
            "black" => Some(Self::Black),
            "avatar" => Some(Self::Avatar),
            _ => None,
        }
    }

    /// The id this style is known by in the shell and in the persisted record.
    #[cfg(target_os = "macos")]
    fn name(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Black => "black",
            Self::Avatar => "avatar",
        }
    }

    /// The bundled app icon already is this variant.
    #[cfg(target_os = "macos")]
    fn is_default(self) -> bool {
        matches!(self, Self::Blue)
    }

    #[cfg(target_os = "macos")]
    fn png_bytes(self) -> &'static [u8] {
        match self {
            Self::Blue => include_bytes!("../../../Assets/DeepSeekHarnessIcon-Prepared-1024.png"),
            Self::Black => {
                include_bytes!("../../../Assets/DeepSeekHarnessIcon-Black-Prepared-1024.png")
            }
            Self::Avatar => {
                include_bytes!("../../../Assets/DeepSeekHarnessIcon-Avatar-Prepared-1024.png")
            }
        }
    }
}

/// Outcome of a Dock icon switch. `applied` reports the running Dock tile and
/// `persisted` reports whether the icon also survives quitting the app.
#[derive(Serialize)]
struct DockIconOutcome {
    applied: bool,
    persisted: bool,
}

/// Repaints the Dock tile of the running application.
///
/// Tauri runs synchronous commands on the main thread, so this is queued with
/// `run_on_main_thread` instead of being called inline: the bundle icon write
/// below runs on the command thread and must never block waiting for the main
/// thread's event loop.
#[cfg(target_os = "macos")]
fn apply_macos_dock_icon(app: &AppHandle, variant: DockIconVariant) -> Result<(), String> {
    app.run_on_main_thread(move || {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;

        let Some(marker) = MainThreadMarker::new() else {
            return;
        };
        let Some(image) = dock_icon_image(variant) else {
            return;
        };
        let application = NSApplication::sharedApplication(marker);
        unsafe {
            application.setApplicationIconImage(Some(&image));
        }
    })
    .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn dock_icon_image(variant: DockIconVariant) -> Option<Retained<objc2_app_kit::NSImage>> {
    use objc2::AnyThread;
    use objc2_app_kit::NSImage;
    use objc2_foundation::NSData;

    let data = NSData::with_bytes(variant.png_bytes());
    NSImage::initWithData(NSImage::alloc(), &data)
}

/// Which style the app last wrote into which bundle, and from which build.
///
/// A bundle icon is not self-describing: macOS stores pixels and nothing else,
/// so a launch cannot tell the style the user picked from one left behind by an
/// older build. This record is what makes that call. It lives in the app's own
/// data directory because a bundle in `/Applications` cannot be written to by
/// the app at all — macOS there rejects even creating a single file inside the
/// package — so no note in the bundle itself can serve as the record.
#[cfg(target_os = "macos")]
#[derive(Serialize, Deserialize)]
struct DockIconRecord {
    bundle: PathBuf,
    variant: String,
    version: String,
}

#[cfg(target_os = "macos")]
fn dock_icon_record_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|directory| directory.join("dock-icon.json"))
}

#[cfg(target_os = "macos")]
fn read_dock_icon_record(path: &Path) -> Option<DockIconRecord> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Best effort: a record that never got written only costs one redundant bundle
/// rewrite on the next launch, never a wrong icon.
#[cfg(target_os = "macos")]
fn save_dock_icon_record(path: &Path, record: &DockIconRecord) {
    let Ok(bytes) = serde_json::to_vec(record) else {
        return;
    };
    if let Some(directory) = path.parent() {
        if fs::create_dir_all(directory).is_err() {
            return;
        }
    }
    let _ = fs::write(path, bytes);
}

/// Whether the app bundle currently carries a Finder custom icon, which is what
/// macOS shows for an application that is not running.
///
/// The icon pixels live in the resource fork and the file's own data stays
/// empty, so an `Icon\r` that exists is not yet an icon — only carrying icon data
/// is, and that is the difference between skipping a rewrite and needing one.
#[cfg(target_os = "macos")]
fn bundle_custom_icon_present(bundle: &Path) -> bool {
    let icon = bundle.join("Icon\r");
    match fs::metadata(icon.join("..namedfork").join("rsrc")) {
        Ok(fork) => fork.len() > 0,
        Err(_) => false,
    }
}

/// Whether the bundle already shows exactly what `variant` asks for, so the
/// rewrite — and the Dock restart that has to follow it — can be skipped.
#[cfg(target_os = "macos")]
fn dock_icon_already_applied(
    bundle: &Path,
    variant: DockIconVariant,
    record: Option<&DockIconRecord>,
) -> bool {
    let present = bundle_custom_icon_present(bundle);
    if variant.is_default() {
        return !present;
    }
    present
        && record.is_some_and(|record| {
            record.bundle == bundle
                && record.version == APP_VERSION
                && DockIconVariant::parse(&record.variant) == Some(variant)
        })
}

/// Writes (or clears) the app bundle's Finder custom icon.
///
/// macOS only reads the bundle icon while an app is *not* running, so
/// `setApplicationIconImage` alone cannot survive a quit: the Dock, Finder and
/// Launchpad would fall back to the bundled icon. The write goes through
/// `NSWorkspace`, whose icon services perform it on our behalf — that reaches a
/// bundle in `/Applications` too, where the app could not write a file itself.
/// Returns `false` only when the system refused, in which case just the running
/// Dock tile changes.
#[cfg(target_os = "macos")]
fn persist_macos_dock_icon(bundle: &Path, variant: DockIconVariant) -> bool {
    use objc2_app_kit::{NSWorkspace, NSWorkspaceIconCreationOptions};

    let workspace = NSWorkspace::sharedWorkspace();
    let path = NSString::from_str(&bundle.to_string_lossy());

    let written = if variant.is_default() {
        if !bundle_custom_icon_present(bundle) {
            return true;
        }
        workspace.setIcon_forFile_options(None, &path, NSWorkspaceIconCreationOptions(0))
    } else {
        let Some(image) = dock_icon_image(variant) else {
            return false;
        };
        workspace.setIcon_forFile_options(Some(&image), &path, NSWorkspaceIconCreationOptions(0))
    };
    if written {
        workspace.noteFileSystemChanged_(&path);
    }
    written
}

/// Restarts the Dock so it rebuilds its cached tile icons from disk.
///
/// The Dock keeps its own copy of a pinned app's icon and only re-reads the
/// bundle when the Dock process itself restarts. Finder and LaunchServices pick
/// up a rewritten bundle icon at once, but without this the pinned tile kept
/// showing the previous artwork after the app quit. Best effort: the icon on
/// disk is correct either way and simply shows up at the next Dock restart.
#[cfg(target_os = "macos")]
fn restart_macos_dock() {
    let _ = Command::new("/usr/bin/killall").arg("Dock").status();
}

#[tauri::command]
fn set_dock_icon_variant(app: AppHandle, variant: String) -> Result<DockIconOutcome, String> {
    let variant =
        DockIconVariant::parse(&variant).ok_or_else(|| "不支持的 Dock 图标样式。".to_string())?;

    #[cfg(target_os = "macos")]
    {
        let applied = dock_icon_image(variant).is_some();
        apply_macos_dock_icon(&app, variant)?;
        // The running tile is not what macOS shows once the app quits, so the
        // style also goes into the app bundle, and the Dock has to be restarted
        // for its cached tile to follow. The record keeps the boot-time re-apply
        // from restarting the Dock on every launch.
        let record_path = dock_icon_record_path(&app);
        let record = record_path.as_deref().and_then(read_dock_icon_record);
        let persisted = match current_app_bundle().as_deref() {
            None => false,
            Some(bundle) => {
                if dock_icon_already_applied(bundle, variant, record.as_ref()) {
                    true
                } else if !persist_macos_dock_icon(bundle, variant) {
                    false
                } else {
                    if let Some(path) = record_path.as_deref() {
                        save_dock_icon_record(
                            path,
                            &DockIconRecord {
                                bundle: bundle.to_path_buf(),
                                variant: variant.name().to_string(),
                                version: APP_VERSION.to_string(),
                            },
                        );
                    }
                    restart_macos_dock();
                    true
                }
            }
        };
        Ok(DockIconOutcome { applied, persisted })
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, variant);
        Ok(DockIconOutcome {
            applied: false,
            persisted: false,
        })
    }
}

#[cfg(target_os = "macos")]
fn activate_macos_application() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationOptions, NSRunningApplication};

    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let application = NSApplication::sharedApplication(marker);
    application.unhide(None);
    application.activate();
    let running = NSRunningApplication::currentApplication();
    let _ = running.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
}

#[cfg(target_os = "macos")]
fn main_window_webview_configuration() -> Retained<WKWebViewConfiguration> {
    use objc2::MainThreadOnly;
    use objc2_web_kit::{WKUserScript, WKUserScriptInjectionTime};

    let marker = objc2::MainThreadMarker::new()
        .expect("WKWebView configuration must be created on the main thread");
    let config = unsafe { WKWebViewConfiguration::new(marker) };
    let display_name = NSString::from_str("DeepSeek Harness Desk");

    // macOS labels the separate WebKit content process with the page origin by
    // default. Set its process display name so Activity Monitor attributes it
    // to the host application instead of showing tauri://localhost.
    unsafe {
        config.setValue_forKey(
            Some(display_name.as_ref()),
            ns_string!("processDisplayName"),
        );
    }

    // Paste bridge: the Harness UI runs in a cross-origin iframe, so the shell
    // frame can not reach into it. Inject the bridge into *every* frame at
    // document start; the native paste handler then only has to hand the
    // payload to the shell frame.
    let source = NSString::from_str(PASTE_BRIDGE_SCRIPT);
    let user_script = unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
            WKUserScript::alloc(marker),
            &source,
            WKUserScriptInjectionTime::AtDocumentStart,
            false,
        )
    };
    unsafe { config.userContentController().addUserScript(&user_script) };
    config
}

/// Injected into every frame of the main WebView (shell frame and the Harness
/// iframe) so the native `NSPasteboard` payload can reach the Harness
/// composer. See `tauri-app/src/paste-bridge.js`.
#[cfg(target_os = "macos")]
const PASTE_BRIDGE_SCRIPT: &str = include_str!("../../src/paste-bridge.js");

#[cfg(target_os = "macos")]
mod paste_bridge {
    use std::path::PathBuf;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use block2::RcBlock;
    use objc2_app_kit::{
        NSEvent, NSEventMask, NSEventModifierFlags, NSPasteboard, NSPasteboardItem,
        NSPasteboardTypeFileURL, NSPasteboardTypePNG, NSPasteboardTypeTIFF,
    };
    use objc2_foundation::{NSData, NSDataBase64EncodingOptions, NSString, NSURL};
    use tauri::{AppHandle, Manager, Runtime};

    /// Total pasteboard payload we are willing to move into the page. The
    /// Harness caps images far below this; this only keeps a runaway file copy
    /// from building a giant JS string.
    const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;
    /// Physical `V` key on ANSI and ISO layouts.
    const V_KEY_CODE: u16 = 9;
    /// `NSEvent.charactersIgnoringModifiers` for a printable `v`.
    const V_CHARACTER: &str = "v";
    /// Image flavors to look for, most preferred first. The Harness attachment
    /// store only accepts png/jpeg/webp/gif, so the frame bridge re-encodes any
    /// other flavor (TIFF shows up for Preview and Finder preview copies).
    const IMAGE_FLAVORS: [(&str, &str); 2] = [
        ("image/png", "pasted-image.png"),
        ("image/tiff", "pasted-image.tiff"),
    ];

    static MONITOR_INSTALLED: AtomicBool = AtomicBool::new(false);
    static PASTE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct ClipboardFile {
        name: String,
        media_type: String,
        bytes: Vec<u8>,
    }

    struct ClipboardPayload {
        files: Vec<ClipboardFile>,
        skipped: Vec<String>,
    }

    /// Installs a local key monitor that turns `Cmd+V` with a file or image
    /// pasteboard into a bridge call. Plain text pastes are returned unchanged
    /// so the regular responder chain and menu handling stay untouched.
    pub fn install<R: Runtime>(app: &AppHandle<R>) {
        if MONITOR_INSTALLED.swap(true, Ordering::SeqCst) {
            return;
        }
        let handle = app.clone();
        let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            let event_ref = unsafe { event.as_ref() };
            if !is_paste_shortcut(event_ref) {
                return event.as_ptr();
            }
            let Some(payload) = read_pasteboard() else {
                // No files or images: let WebKit paste the text itself.
                return event.as_ptr();
            };
            deliver(&handle, payload);
            // Swallow the keystroke: WebKit would otherwise paste nothing (or
            // an inline image) on top of the bridged attachment.
            std::ptr::null_mut()
        });
        let monitor = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block)
        };
        // The monitor lives for the whole application; leak the token so it is
        // never removed and the block keeps its captured app handle.
        if let Some(monitor) = monitor {
            std::mem::forget(monitor);
        } else {
            MONITOR_INSTALLED.store(false, Ordering::SeqCst);
        }
        std::mem::forget(block);
    }

    fn is_paste_shortcut(event: &NSEvent) -> bool {
        let flags = event.modifierFlags();
        if !flags.contains(NSEventModifierFlags::Command) {
            return false;
        }
        if flags.contains(NSEventModifierFlags::Option)
            || flags.contains(NSEventModifierFlags::Control)
        {
            return false;
        }
        if event.keyCode() == V_KEY_CODE {
            return true;
        }
        match event.charactersIgnoringModifiers() {
            Some(characters) => characters.to_string().eq_ignore_ascii_case(V_CHARACTER),
            None => false,
        }
    }

    fn read_pasteboard() -> Option<ClipboardPayload> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let mut files: Vec<ClipboardFile> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        let mut total: usize = 0;

        if let Some(items) = pasteboard.pasteboardItems() {
            for item in items.iter() {
                if let Some(file) = file_from_item(&item) {
                    push_file(file, &mut files, &mut skipped, &mut total);
                    continue;
                }
                if let Some(file) = image_from_item(&item) {
                    push_file(file, &mut files, &mut skipped, &mut total);
                }
            }
        }

        // Some sources put the image on the pasteboard rather than on a
        // concrete item (`screencapture -c`, browsers copying an image).
        if files.is_empty() {
            for (media_type, name) in IMAGE_FLAVORS {
                if let Some(data) = image_data(&pasteboard, media_type) {
                    push_file(
                        ClipboardFile {
                            name: name.to_string(),
                            media_type: media_type.to_string(),
                            bytes: data,
                        },
                        &mut files,
                        &mut skipped,
                        &mut total,
                    );
                    break;
                }
            }
        }

        (!files.is_empty()).then_some(ClipboardPayload { files, skipped })
    }

    fn push_file(
        file: ClipboardFile,
        files: &mut Vec<ClipboardFile>,
        skipped: &mut Vec<String>,
        total: &mut usize,
    ) {
        if *total + file.bytes.len() > MAX_TOTAL_BYTES {
            skipped.push(file.name);
            return;
        }
        *total += file.bytes.len();
        files.push(file);
    }

    fn file_from_item(item: &NSPasteboardItem) -> Option<ClipboardFile> {
        let value = unsafe { item.stringForType(NSPasteboardTypeFileURL) }?;
        let path = file_url_path(&value.to_string())?;
        let bytes = std::fs::read(&path).ok()?;
        let name = PathBuf::from(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "pasted".to_string());
        Some(ClipboardFile {
            media_type: media_type_for_path(&path),
            name,
            bytes,
        })
    }

    fn image_from_item(item: &NSPasteboardItem) -> Option<ClipboardFile> {
        for (media_type, name) in IMAGE_FLAVORS {
            let Some(data) = (unsafe {
                match media_type {
                    "image/png" => item.dataForType(NSPasteboardTypePNG),
                    _ => item.dataForType(NSPasteboardTypeTIFF),
                }
            }) else {
                continue;
            };
            return Some(ClipboardFile {
                name: name.to_string(),
                media_type: media_type.to_string(),
                bytes: data.to_vec(),
            });
        }
        None
    }

    fn image_data(pasteboard: &NSPasteboard, media_type: &str) -> Option<Vec<u8>> {
        let data = unsafe {
            match media_type {
                "image/png" => pasteboard.dataForType(NSPasteboardTypePNG),
                _ => pasteboard.dataForType(NSPasteboardTypeTIFF),
            }
        }?;
        Some(data.to_vec())
    }

    fn file_url_path(value: &str) -> Option<String> {
        let value = NSString::from_str(value);
        let url = NSURL::URLWithString(&value)?;
        if !url.isFileURL() {
            return None;
        }
        let path = url.path()?;
        Some(path.to_string())
    }

    fn media_type_for_path(path: &str) -> String {
        let extension = PathBuf::from(path)
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        match extension.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "tif" | "tiff" => "image/tiff",
            "pdf" => "application/pdf",
            "txt" | "log" | "md" => "text/plain",
            "json" => "application/json",
            "csv" => "text/csv",
            "zip" => "application/zip",
            _ => "application/octet-stream",
        }
        .to_string()
    }

    fn deliver<R: Runtime>(app: &AppHandle<R>, payload: ClipboardPayload) {
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        let id = PASTE_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let files = payload
            .files
            .into_iter()
            .map(|file| {
                serde_json::json!({
                    "name": file.name,
                    "type": file.media_type,
                    "data": base64(&file.bytes),
                })
            })
            .collect::<Vec<_>>();
        let call = serde_json::json!({
            "id": id,
            "files": files,
            "skipped": payload.skipped,
        });
        // The payload is JSON, so it is also valid JavaScript to inline.
        let script = format!("window.__dshDeskPasteFiles && window.__dshDeskPasteFiles({call});");
        let _ = window.eval(&script);
    }

    fn base64(bytes: &[u8]) -> String {
        let data = NSData::with_bytes(bytes);
        data.base64EncodedStringWithOptions(NSDataBase64EncodingOptions::empty())
            .to_string()
    }
}

#[cfg(target_os = "macos")]
fn current_appearance_theme() -> Option<Theme> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    let marker = MainThreadMarker::new()?;
    let name = NSApplication::sharedApplication(marker)
        .effectiveAppearance()
        .name()
        .to_string();
    Some(if name.contains("Dark") {
        Theme::Dark
    } else {
        Theme::Light
    })
}

/// What the shell should do with a navigation requested inside the main
/// WebView.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NavigationDisposition {
    /// Keep the navigation inside the app.
    InApp,
    /// Cancel it and hand the URL to the system's default handler.
    External,
}

/// True for hosts that can only point back at this machine. The Harness runs
/// on a loopback port that changes on every start, so the check is per host
/// rather than per exact URL. Only real loopback literals count: a name like
/// `127.0.0.1.example.com` is somebody else's website.
fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
}

/// Classifies a navigation requested inside the main WebView.
///
/// The shell only ever renders its own `tauri://` document plus the loopback
/// Harness pages, so an `http(s)` URL pointing anywhere else is a link the user
/// followed — the Harness renders every web link with `target="_blank"`. WebKit
/// cannot honour those links on its own: with no new-window handler it drops
/// the click, and left alone in the same frame it would replace the whole shell
/// with the site. Both cases belong in the user's browser instead.
///
/// Schemes the shell knows nothing about stay in WebKit's hands, so an
/// installed handler can still answer them.
fn navigation_disposition(url: &Url) -> NavigationDisposition {
    match url.scheme() {
        "http" | "https" => match url.host_str() {
            Some(host) if is_loopback_host(host) => NavigationDisposition::InApp,
            _ => NavigationDisposition::External,
        },
        "mailto" => NavigationDisposition::External,
        _ => NavigationDisposition::InApp,
    }
}

/// Hands a link to the system's default browser / mail client.
///
/// The launcher runs on its own thread: a navigation decision made by WebKit
/// must not wait for the browser to start, and a failed launch must never take
/// the click down with it.
fn open_link_externally<R: tauri::Runtime>(app: &AppHandle<R>, url: &Url) {
    let target = url.as_str().to_string();
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(error) = open_external_url(&target) {
            eprintln!("打开链接失败：{error}");
            let _ = app.emit("link-open-failed", serde_json::json!({ "url": target }));
        }
    });
}

/// Decides one navigation request from the main WebView.
fn handle_navigation<R: tauri::Runtime>(app: &AppHandle<R>, url: &Url) -> bool {
    match navigation_disposition(url) {
        NavigationDisposition::InApp => true,
        NavigationDisposition::External => {
            open_link_externally(app, url);
            false
        }
    }
}

fn create_main_window<R: tauri::Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .ok_or_else(|| tauri::Error::AssetNotFound("缺少 main 窗口配置".to_string()))?;
    let builder = WebviewWindowBuilder::from_config(app, config)?;
    let navigation_app = app.clone();
    let new_window_app = app.clone();
    let builder = builder
        // External links leave the app; everything the shell renders itself
        // (the `tauri://` document and the loopback Harness pages) keeps
        // navigating in place.
        .on_navigation(move |url| handle_navigation(&navigation_app, url))
        .on_new_window(move |url, _features| {
            // `target="_blank"` links, `window.open` and the link context menu
            // ask for a window Tauri does not provide. Without a handler WebKit
            // cancels the request and the click looks dead, so the URL goes to
            // the default browser and no second window is created.
            if navigation_disposition(&url) == NavigationDisposition::External {
                open_link_externally(&new_window_app, &url);
            }
            NewWindowResponse::Deny
        });
    #[cfg(target_os = "macos")]
    let builder = {
        // The WKWebView keeps an opaque white surface until the page's first
        // paint. Passing the color at build time is the only way wry disables
        // that white surface (drawsBackground) for the recreated window; a
        // post-build set_background_color lands after the white first frame.
        let builder = builder.with_webview_configuration(main_window_webview_configuration());
        match current_appearance_theme() {
            Some(theme) => builder.background_color(window_background_color(theme)),
            None => builder,
        }
    };
    let window = builder.build()?;
    sync_window_background(&window);
    Ok(window)
}

fn window_background_color(theme: Theme) -> Color {
    if matches!(theme, Theme::Dark) {
        Color(28, 28, 30, 255)
    } else {
        Color(245, 245, 247, 255)
    }
}

fn sync_window_background<R: tauri::Runtime>(window: &WebviewWindow<R>) {
    let Ok(theme) = window.theme() else {
        return;
    };
    let _ = window.set_background_color(Some(window_background_color(theme)));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HiddenWindowAction {
    DestroyWebview,
    Hide,
}

fn hidden_window_action(memory_saver: bool) -> HiddenWindowAction {
    if memory_saver {
        HiddenWindowAction::DestroyWebview
    } else {
        HiddenWindowAction::Hide
    }
}

fn present_main_window<R: tauri::Runtime>(app: &AppHandle<R>, window: WebviewWindow<R>) -> bool {
    if window.show().is_err() {
        return false;
    }
    let _ = window.unminimize();
    let _ = window.set_focus();
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(activate_macos_application);
    }
    // Wake an existing shell or the newly created shell so it can restore the
    // Harness web UI after the window is shown again.
    let _ = app.emit("window-shown", ());
    // The user is back — clear the attention badge.
    let state = app.state::<HarnessState>();
    clear_badge(app, state.inner());
    state
        .keep_alive_after_window_destroy
        .store(false, Ordering::Release);
    true
}

fn show_main_window<R: tauri::Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.show();
    }

    if let Some(window) = app.get_webview_window("main") {
        if present_main_window(app, window) {
            return;
        }
    }

    let state = app.state::<HarnessState>().inner().clone();
    if state.main_window_recreating.swap(true, Ordering::AcqRel) {
        return;
    }

    // A destroyed main window keeps the app alive in the tray, but that flag
    // must not also block the replacement window from being created. The
    // destroy callback has already completed by the time a tray/Dock reopen
    // event reaches this path; if creation briefly races the native teardown,
    // the retry loop below handles it.
    state
        .keep_alive_after_window_destroy
        .store(false, Ordering::Release);

    // Tauri warns that creating a WebviewWindow synchronously from a window
    // or tray callback can deadlock on Windows. Rebuild off the event handler,
    // but dispatch the actual creation back to the Tauri main thread so the
    // macOS WKWebViewConfiguration can be created on its required thread.
    let app = app.clone();
    std::thread::spawn(move || {
        for _ in 0..40 {
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            let app_for_main = app.clone();
            if app
                .run_on_main_thread(move || {
                    let window = match app_for_main.get_webview_window("main") {
                        Some(window) => Some(window),
                        None => match create_main_window(&app_for_main) {
                            Ok(window) => Some(window),
                            Err(error) => {
                                eprintln!("重建主窗口失败：{error}");
                                None
                            }
                        },
                    };
                    let shown = window
                        .map(|window| present_main_window(&app_for_main, window))
                        .unwrap_or(false);
                    let _ = sender.send(shown);
                })
                .is_err()
            {
                break;
            }
            if receiver
                .recv_timeout(Duration::from_millis(250))
                .unwrap_or(false)
            {
                state.main_window_recreating.store(false, Ordering::Release);
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        state.main_window_recreating.store(false, Ordering::Release);
        eprintln!("重建主窗口超时");
    });
}

fn setup_app_menu<R: tauri::Runtime>(app: &mut tauri::App<R>) -> tauri::Result<()> {
    let zoom_in = MenuItem::with_id(app, "zoom-in", "放大", true, Some("CmdOrCtrl+="))?;
    let zoom_out = MenuItem::with_id(app, "zoom-out", "缩小", true, Some("CmdOrCtrl+-"))?;
    let zoom_reset =
        MenuItem::with_id(app, "zoom-reset", "恢复默认大小", true, Some("CmdOrCtrl+0"))?;

    // Keep the standard macOS menu (App/File/Edit/View/Window/Help). The Edit
    // submenu is what routes Cmd+C / Cmd+V / Cmd+X keyboard shortcuts to the
    // WebView; replacing the whole menu bar with only the zoom submenu removed
    // it and broke copy/paste/cut shortcuts (right-click paste still worked
    // because it is handled by the WebView itself).
    let menu = tauri::menu::Menu::default(app.handle())?;

    // Add the zoom items to the existing "View" submenu on macOS, or append a
    // dedicated "查看" submenu on platforms that have no default View menu.
    let mut appended = false;
    for item in menu.items()? {
        if let MenuItemKind::Submenu(submenu) = item {
            if submenu.text()? == "View" {
                submenu.append_items(&[&zoom_in, &zoom_out, &zoom_reset])?;
                appended = true;
                break;
            }
        }
    }
    if !appended {
        let view = SubmenuBuilder::new(app, "查看")
            .items(&[&zoom_in, &zoom_out, &zoom_reset])
            .build()?;
        menu.append(&view)?;
    }

    app.set_menu(menu)?;
    Ok(())
}

#[cfg(feature = "tray-icon")]
fn tray_menu_label(lang: &str, id: &str) -> &'static str {
    match (lang, id) {
        ("en", "show") => "Open Window",
        ("en", "settings") => "Settings…",
        ("en", "start") => "Start Harness",
        ("en", "restart") => "Restart Harness",
        ("en", "stop") => "Stop Harness",
        ("en", "logs") => "Open Run Logs",
        ("en", "check-app") => "Check for App Updates…",
        ("en", "check-dsh") => "Check for Bundled dsh Updates…",
        ("en", "quit") => "Quit DeepSeek Harness Desk",
        (_, "show") => "打开窗口",
        (_, "settings") => "设置…",
        (_, "start") => "启动 Harness",
        (_, "restart") => "重启 Harness",
        (_, "stop") => "停止 Harness",
        (_, "logs") => "打开运行日志",
        (_, "check-app") => "检查 App 更新…",
        (_, "check-dsh") => "检查内置 dsh 更新…",
        (_, "quit") => "退出 DeepSeek Harness Desk",
        // All known ids are handled above; the fallback is unreachable.
        (_, _) => "",
    }
}

#[cfg(feature = "tray-icon")]
fn build_tray<R: tauri::Runtime>(app: &AppHandle<R>, lang: &str) -> tauri::Result<()> {
    let show = MenuItem::with_id(
        app,
        "show",
        tray_menu_label(lang, "show"),
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(
        app,
        "settings",
        tray_menu_label(lang, "settings"),
        true,
        None::<&str>,
    )?;
    let start = MenuItem::with_id(
        app,
        "start",
        tray_menu_label(lang, "start"),
        true,
        None::<&str>,
    )?;
    let restart = MenuItem::with_id(
        app,
        "restart",
        tray_menu_label(lang, "restart"),
        true,
        None::<&str>,
    )?;
    let stop = MenuItem::with_id(
        app,
        "stop",
        tray_menu_label(lang, "stop"),
        true,
        None::<&str>,
    )?;
    let logs = MenuItem::with_id(
        app,
        "logs",
        tray_menu_label(lang, "logs"),
        true,
        None::<&str>,
    )?;
    let check_app = MenuItem::with_id(
        app,
        "check-app",
        tray_menu_label(lang, "check-app"),
        true,
        None::<&str>,
    )?;
    let check_dsh = MenuItem::with_id(
        app,
        "check-dsh",
        tray_menu_label(lang, "check-dsh"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        tray_menu_label(lang, "quit"),
        true,
        None::<&str>,
    )?;
    let menu = MenuBuilder::new(app)
        .items(&[
            &show, &settings, &start, &restart, &stop, &logs, &check_app, &check_dsh, &quit,
        ])
        .build()?;

    let icon = tauri::image::Image::from_bytes(include_bytes!(
        "../../../Assets.xcassets/StatusBarIcon.imageset/statusbar_whale@2x.png"
    ))?;

    TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .tooltip("DeepSeek Harness Desk")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "settings" => {
                show_main_window(app);
                let _ = app.emit("open-settings", ());
            }
            "start" => {
                show_main_window(app);
                let _ = app.emit("start-harness", ());
            }
            "restart" => {
                show_main_window(app);
                let _ = app.emit("restart-harness", ());
            }
            "stop" => {
                show_main_window(app);
                let _ = app.emit("stop-harness", ());
            }
            "logs" => {
                show_main_window(app);
                let _ = app.emit("open-logs", ());
            }
            "check-app" => {
                show_main_window(app);
                let _ = app.emit("check-app-update", ());
            }
            "check-dsh" => {
                show_main_window(app);
                let _ = app.emit("check-dsh-update", ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg(feature = "tray-icon")]
fn setup_tray<R: tauri::Runtime>(
    app: &mut tauri::App<R>,
    state: &HarnessState,
) -> tauri::Result<()> {
    build_tray(app.handle(), &current_language(state))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = HarnessState {
        lifecycle: Arc::new(Mutex::new(())),
        child: Arc::new(Mutex::new(None)),
        port: Arc::new(AtomicU16::new(0)),
        proxy_port: Arc::new(AtomicU16::new(0)),
        proxy_stop: Arc::new(Mutex::new(None)),
        memory_saver: Arc::new(AtomicBool::new(true)),
        keep_alive_after_window_destroy: Arc::new(AtomicBool::new(false)),
        main_window_recreating: Arc::new(AtomicBool::new(false)),
        dsh_path: Arc::new(Mutex::new(None)),
        token: Arc::new(Mutex::new(None)),
        auth_cookie: Arc::new(Mutex::new(None)),
        logs: Arc::new(Mutex::new(VecDeque::new())),
        last_error: Arc::new(Mutex::new(None)),
        last_exit_code: Arc::new(Mutex::new(None)),
        last_event_at: Arc::new(AtomicI64::new(0)),
        runtime_installing: Arc::new(AtomicBool::new(false)),
        runtime_message: Arc::new(Mutex::new(String::new())),
        app_update_installing: Arc::new(AtomicBool::new(false)),
        notify_enabled: Arc::new(AtomicBool::new(true)),
        notify_task_completed: Arc::new(AtomicBool::new(true)),
        notify_interaction: Arc::new(AtomicBool::new(true)),
        notify_error: Arc::new(AtomicBool::new(true)),
        notify_detail: Arc::new(AtomicBool::new(false)),
        pending_attention: Arc::new(AtomicI64::new(0)),
        language: Arc::new(Mutex::new("zh".to_string())),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(state)
        .setup(|app| {
            setup_app_menu(app)?;
            let window = create_main_window(app.handle())?;
            sync_window_background(&window);
            #[cfg(target_os = "macos")]
            paste_bridge::install(app.handle());
            #[cfg(feature = "tray-icon")]
            {
                let state = app.state::<HarnessState>().inner().clone();
                setup_tray(app, &state)?;
            }
            spawn_task_watcher(
                app.handle().clone(),
                app.state::<HarnessState>().inner().clone(),
            );
            #[cfg(target_os = "macos")]
            {
                // Ask macOS for notification permission up front, off the main
                // thread, so the first real notice is never swallowed by an
                // unanswered authorization prompt. The outcome is logged
                // because a missing banner is otherwise indistinguishable
                // from a task that simply never needed attention.
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    let message = match macos_notification::ensure_authorized() {
                        Ok(()) => "系统通知权限已就绪。".to_string(),
                        Err(macos_notification::AuthFailure::Denied) => {
                            "系统通知权限被拒绝，提醒将只更新应用角标。".to_string()
                        }
                        Err(macos_notification::AuthFailure::NoResponse) => {
                            "系统未回应通知授权请求，提醒可能无法送达。".to_string()
                        }
                        Err(macos_notification::AuthFailure::Unavailable) => {
                            "当前没有可用的系统通知中心，提醒可能无法送达。".to_string()
                        }
                    };
                    let state = handle.state::<HarnessState>();
                    emit_log(&handle, &state.logs, "desk", message);
                });
            }
            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "zoom-in" => {
                let _ = app.emit("zoom-in", ());
            }
            "zoom-out" => {
                let _ = app.emit("zoom-out", ());
            }
            "zoom-reset" => {
                let _ = app.emit("zoom-reset", ());
            }
            _ => {}
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let state = window.app_handle().state::<HarnessState>();
                if hidden_window_action(state.memory_saver.load(Ordering::Acquire))
                    == HiddenWindowAction::DestroyWebview
                {
                    // Removing the Harness iframe does not destroy the shared
                    // WebKit WebContent process. Destroy the main WebView so
                    // its renderer can actually release its accumulated heap;
                    // the tray remains alive and recreates it on next show.
                    state
                        .keep_alive_after_window_destroy
                        .store(true, Ordering::Release);
                    if let Err(error) = window.destroy() {
                        state
                            .keep_alive_after_window_destroy
                            .store(false, Ordering::Release);
                        eprintln!("销毁主 WebView 失败：{error}");
                        let _ = window.hide();
                        let _ = window.app_handle().emit("window-hidden", ());
                    }
                } else {
                    // The app keeps running in the tray after the window
                    // closes when the memory saver is disabled.
                    let _ = window.hide();
                    let _ = window.app_handle().emit("window-hidden", ());
                }
            } else if let WindowEvent::ThemeChanged(theme) = event {
                // Route through the WebviewWindow so the color reaches both the
                // native window and the WKWebView surface (the Window-only API
                // never propagates to the webview layer).
                let color = Some(window_background_color(*theme));
                match window
                    .app_handle()
                    .get_webview_window(window.label())
                    .map(|webview_window| webview_window.set_background_color(color))
                {
                    Some(Ok(())) => {}
                    _ => {
                        let _ = window.set_background_color(color);
                    }
                }
            } else if let WindowEvent::Focused(true) = event {
                // The user is back — clear the attention badge and let the
                // shell reload the Harness page if it was unloaded on unfocus.
                let state = window.app_handle().state::<HarnessState>();
                clear_badge(window.app_handle(), state.inner());
                let _ = window.app_handle().emit("window-focused", ());
            } else if let WindowEvent::Focused(false) = event {
                // The window is visible but no longer active; the shell may
                // choose to release the Harness page after a delay.
                let _ = window.app_handle().emit("window-unfocused", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            harness_status,
            runtime_status,
            install_runtime,
            check_app_update,
            install_app_update,
            check_dsh_update,
            install_dsh_update,
            list_dsh_versions,
            set_dsh_version,
            follow_latest_dsh_version,
            open_release_page,
            open_releases_page,
            open_runtime_directory,
            open_logs_directory,
            set_launch_at_login,
            start_harness,
            restart_harness,
            stop_harness,
            harness_logs,
            clear_harness_logs,
            window_minimize,
            window_toggle_maximize,
            window_hide,
            window_start_dragging,
            set_memory_saver,
            set_dock_visibility,
            set_dock_icon_variant,
            set_notification_prefs,
            notification_prefs,
            notification_permission,
            send_test_notification,
            open_notification_settings,
            set_language,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if matches!(&event, RunEvent::Reopen { .. }) {
                show_main_window(app);
            }

            match event {
                RunEvent::ExitRequested { api, code, .. } => {
                    let state = app.state::<HarnessState>();
                    if code.is_none()
                        && state
                            .keep_alive_after_window_destroy
                            .load(Ordering::Acquire)
                    {
                        // Destroying the only window would otherwise terminate
                        // the event loop. Keep the tray resident until the user
                        // explicitly chooses Quit or uses the app's real exit
                        // action.
                        api.prevent_exit();
                    } else {
                        state
                            .keep_alive_after_window_destroy
                            .store(false, Ordering::Release);
                        stop_harness_inner(app, &state);
                    }
                }
                RunEvent::Exit => {
                    let state = app.state::<HarnessState>();
                    stop_harness_inner(app, &state);
                }
                _ => {}
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_launch_token_parses_dsh_web_line() {
        assert_eq!(
            extract_launch_token(
                "dsh web: http://127.0.0.1:3080/?token=I3p7Je_ZBcUDP-N4oyRa-0tmtOe7ePeq0mzO16PX7eE"
            ),
            Some("I3p7Je_ZBcUDP-N4oyRa-0tmtOe7ePeq0mzO16PX7eE".to_string())
        );
    }

    #[test]
    fn extract_launch_token_ignores_lines_without_token() {
        assert_eq!(extract_launch_token("dsh web: http://127.0.0.1:3080"), None);
        assert_eq!(extract_launch_token("added 521 packages in 2m"), None);
        assert_eq!(extract_launch_token(""), None);
    }

    #[test]
    fn extract_launch_token_stops_at_non_token_characters() {
        assert_eq!(
            extract_launch_token("dsh web: http://127.0.0.1:3090/?token=abcDEF-123_ tail"),
            Some("abcDEF-123_".to_string())
        );
    }

    #[test]
    fn window_background_colors_follow_system_theme() {
        assert_eq!(
            window_background_color(Theme::Light),
            Color(245, 245, 247, 255)
        );
        assert_eq!(window_background_color(Theme::Dark), Color(28, 28, 30, 255));
    }

    #[test]
    fn memory_saver_destroys_hidden_webview() {
        assert_eq!(
            hidden_window_action(true),
            HiddenWindowAction::DestroyWebview
        );
        assert_eq!(hidden_window_action(false), HiddenWindowAction::Hide);
    }

    #[test]
    fn dock_icon_variants_match_the_shell_values() {
        assert_eq!(DockIconVariant::parse("blue"), Some(DockIconVariant::Blue));
        assert_eq!(DockIconVariant::parse("black"), Some(DockIconVariant::Black));
        assert_eq!(DockIconVariant::parse("avatar"), Some(DockIconVariant::Avatar));
    }

    #[test]
    fn unknown_dock_icon_variants_are_rejected() {
        assert_eq!(DockIconVariant::parse(""), None);
        assert_eq!(DockIconVariant::parse("Blue"), None);
        assert_eq!(DockIconVariant::parse("a-vatar"), None);
    }

    /// Only the bundled icon may clear the Finder custom icon: every other
    /// style has to be written into the bundle to survive a quit.
    #[cfg(target_os = "macos")]
    #[test]
    fn only_the_blue_dock_icon_uses_the_bundled_artwork() {
        assert!(DockIconVariant::Blue.is_default());
        assert!(!DockIconVariant::Black.is_default());
        assert!(!DockIconVariant::Avatar.is_default());
    }

    /// A throwaway directory shaped like an app bundle for the Dock icon tests.
    /// Each test gets its own name because they all share one process id.
    #[cfg(target_os = "macos")]
    fn temp_dock_icon_bundle(name: &str) -> PathBuf {
        let bundle = std::env::temp_dir().join(format!(
            "dsh-dock-icon-{name}-{}/MyApp.app",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&bundle);
        fs::create_dir_all(&bundle).expect("temp bundle directory");
        bundle
    }

    /// The bundle icon is what macOS shows once the app is not running, so both
    /// writing and clearing it have to work on a real path — and the app must be
    /// able to tell the two states apart afterwards.
    #[cfg(target_os = "macos")]
    #[test]
    fn dock_icon_persistence_writes_and_clears_the_bundle_icon() {
        let bundle = temp_dock_icon_bundle("write");
        assert!(!bundle_custom_icon_present(&bundle));

        assert!(
            persist_macos_dock_icon(&bundle, DockIconVariant::Avatar),
            "writing a custom icon must succeed on a writable path"
        );
        assert!(
            bundle_custom_icon_present(&bundle),
            "the bundle must carry the custom icon"
        );

        // macOS falls back to the bundled artwork only once the custom icon is
        // removed, which is why the blue style has to clear it.
        assert!(
            persist_macos_dock_icon(&bundle, DockIconVariant::Blue),
            "clearing the custom icon must succeed"
        );
        assert!(
            !bundle_custom_icon_present(&bundle),
            "the custom icon must be gone"
        );
        assert!(
            persist_macos_dock_icon(&bundle, DockIconVariant::Blue),
            "clearing an icon that is not set must stay a no-op success"
        );

        let _ = fs::remove_dir_all(&bundle);
    }

    /// The icon pixels live in the resource fork, so an `Icon\r` that carries
    /// none is not an icon: the bundled artwork is still what the user sees, and
    /// treating the file as one would skip a rewrite that is actually needed.
    #[cfg(target_os = "macos")]
    #[test]
    fn only_icon_data_counts_as_a_custom_bundle_icon() {
        let bundle = temp_dock_icon_bundle("icon-data");
        let icon = bundle.join("Icon\r");

        fs::write(&icon, b"").expect("icon file");
        assert!(
            !bundle_custom_icon_present(&bundle),
            "an icon file without icon data must not count"
        );

        fs::write(icon.join("..namedfork").join("rsrc"), b"icon-bytes").expect("resource fork");
        assert!(
            bundle_custom_icon_present(&bundle),
            "icon data must be found in the resource fork"
        );

        let _ = fs::remove_dir_all(&bundle);
    }

    /// Skipping the rewrite is what keeps the Dock from being restarted on every
    /// launch, so it may only fire for the style this build wrote into this
    /// bundle: a hand-set icon, another style, another build or another bundle
    /// all have to trigger a rewrite.
    #[cfg(target_os = "macos")]
    #[test]
    fn dock_icon_already_applied_only_skips_a_matching_bundle() {
        let bundle = temp_dock_icon_bundle("skip");
        let blue = DockIconVariant::Blue;
        let avatar = DockIconVariant::Avatar;
        let record = |variant: &str, version: &str, bundle: &Path| DockIconRecord {
            bundle: bundle.to_path_buf(),
            variant: variant.to_string(),
            version: version.to_string(),
        };

        assert!(dock_icon_already_applied(&bundle, blue, None));
        assert!(!dock_icon_already_applied(&bundle, avatar, None));

        let icon = bundle.join("Icon\r");
        fs::write(&icon, b"").expect("icon file");
        assert!(
            dock_icon_already_applied(&bundle, blue, None),
            "an empty icon file still shows the bundled artwork"
        );
        assert!(!dock_icon_already_applied(&bundle, avatar, None));

        fs::write(icon.join("..namedfork").join("rsrc"), b"icon-bytes").expect("resource fork");
        let other_style = record("black", APP_VERSION, &bundle);
        let older_build = record("avatar", "0.0.0", &bundle);
        let other_bundle = record("avatar", APP_VERSION, &bundle.join("elsewhere"));
        let matching = record("avatar", APP_VERSION, &bundle);
        assert!(
            !dock_icon_already_applied(&bundle, blue, None),
            "a custom icon must be cleared before the bundled artwork returns"
        );
        assert!(
            !dock_icon_already_applied(&bundle, avatar, None),
            "an icon nobody recorded must be rewritten"
        );
        assert!(
            !dock_icon_already_applied(&bundle, avatar, Some(&other_style)),
            "the record must name the style in play"
        );
        assert!(
            !dock_icon_already_applied(&bundle, avatar, Some(&older_build)),
            "a rewrite is due when the artwork may have changed"
        );
        assert!(
            !dock_icon_already_applied(&bundle, avatar, Some(&other_bundle)),
            "the record must describe this very bundle"
        );
        assert!(dock_icon_already_applied(&bundle, avatar, Some(&matching)));

        let _ = fs::remove_dir_all(&bundle);
    }

    /// The record is the only thing that can live outside the bundle, so it has
    /// to survive a round trip; anything unreadable falls back to rewriting.
    #[cfg(target_os = "macos")]
    #[test]
    fn dock_icon_record_round_trips_and_tolerates_a_missing_one() {
        let bundle = temp_dock_icon_bundle("record");
        let path = bundle.join("state").join("dock-icon.json");
        assert!(read_dock_icon_record(&path).is_none());

        save_dock_icon_record(
            &path,
            &DockIconRecord {
                bundle: bundle.clone(),
                variant: DockIconVariant::Avatar.name().to_string(),
                version: APP_VERSION.to_string(),
            },
        );
        let read = read_dock_icon_record(&path).expect("the record must be readable");
        assert_eq!(read.variant, "avatar");
        assert_eq!(read.version, APP_VERSION);
        assert_eq!(read.bundle, bundle);

        fs::write(&path, b"not json").expect("state file");
        assert!(
            read_dock_icon_record(&path).is_none(),
            "a broken record must not be trusted"
        );

        let _ = fs::remove_dir_all(&bundle);
    }

    #[test]
    fn reversed_port_range_is_empty() {
        assert_eq!(first_available_port(PORT_END, PORT_START), None);
    }

    #[test]
    fn harness_proxy_rewrites_http_auth_headers() {
        let request = b"GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:3091\r\nCookie: stale=1\r\nConnection: keep-alive\r\n\r\n";
        let rewritten = String::from_utf8(
            rewrite_harness_proxy_request(request, 3080, Some("dsh-auth-l-1=secret"))
                .expect("proxy request should be rewritten"),
        )
        .expect("rewritten request should stay HTTP text");

        assert!(rewritten.starts_with("GET /api/health HTTP/1.1\r\n"));
        assert!(rewritten.contains("Host: 127.0.0.1:3080\r\n"));
        assert!(rewritten.contains("Cookie: dsh-auth-l-1=secret\r\n"));
        assert!(rewritten.contains("Connection: close\r\n"));
        assert!(!rewritten.contains("stale=1"));
    }

    #[test]
    fn harness_proxy_preserves_websocket_upgrade() {
        let request = b"GET /api/remote.mux HTTP/1.1\r\nHost: 127.0.0.1:3091\r\nOrigin: http://127.0.0.1:3091\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nCookie: stale=1\r\n\r\n";
        let rewritten = String::from_utf8(
            rewrite_harness_proxy_request(request, 3080, Some("dsh-auth-l-1=secret"))
                .expect("proxy request should be rewritten"),
        )
        .expect("rewritten request should stay HTTP text");

        assert!(rewritten.contains("Upgrade: websocket\r\n"));
        assert!(rewritten.contains("Connection: Upgrade\r\n"));
        assert!(rewritten.contains("Origin: http://127.0.0.1:3080\r\n"));
        assert!(rewritten.contains("Cookie: dsh-auth-l-1=secret\r\n"));
        assert!(!rewritten.contains("Connection: close\r\n"));
        assert!(!rewritten.contains("stale=1"));
    }

    #[test]
    fn dsh_candidates_include_path_entries() {
        let candidates = dsh_candidates(None);
        assert!(!candidates.is_empty());
    }

    #[test]
    fn harness_web_command_uses_profile_mode_without_opening_browser() {
        assert_eq!(
            dsh_web_arguments(3080, true),
            vec!["--profile", "web", "--port", "3080", "--no-open",]
        );
        assert_eq!(
            dsh_web_arguments(3080, false),
            vec!["--profile", "web", "--port", "3080"]
        );
        assert!(!dsh_supports_no_open("0.1.0-rc.7"));
        assert!(dsh_supports_no_open("0.1.0-rc.8"));
        assert!(dsh_supports_no_open("0.1.1-rc.1"));
    }

    #[test]
    fn managed_harness_command_requires_exact_path_and_port() {
        let executable = Path::new(
            "/Users/example/Library/Application Support/DeepSeek Harness Desk/runtime/dsh/0.1.0-rc.7/node_modules/.bin/dsh",
        );
        assert!(is_managed_harness_command(
            "node /Users/example/Library/Application Support/DeepSeek Harness Desk/runtime/dsh/0.1.0-rc.7/node_modules/.bin/dsh --profile web --port 3080",
            executable,
            PORT_START,
            PORT_END
        ));
        assert!(is_managed_harness_command(
            "node /Users/example/Library/Application Support/DeepSeek Harness Desk/runtime/dsh/0.1.0-rc.7/node_modules/.bin/dsh web --port 3080",
            executable,
            PORT_START,
            PORT_END
        ));
        assert!(is_managed_harness_command(
            "node /Users/example/Library/Application Support/DeepSeek Harness Desk/runtime/dsh/0.1.0-rc.7/node_modules/.bin/dsh web --port 3080 --no-open",
            executable,
            PORT_START,
            PORT_END
        ));
        assert!(is_managed_harness_command(
            "node /Users/example/Library/Application Support/DeepSeek Harness Desk/runtime/dsh/0.1.0-rc.7/node_modules/.bin/dsh --profile web --port 3080",
            executable,
            PORT_START,
            PORT_END
        ));
        assert!(is_managed_harness_command(
            "node /Users/example/Library/Application Support/DeepSeek Harness Desk/runtime/dsh/0.1.0-rc.7/node_modules/.bin/dsh --profile web --port 3080 --no-open",
            executable,
            PORT_START,
            PORT_END
        ));
        assert!(!is_managed_harness_command(
            "node /Users/example/Library/Application Support/DeepSeek Harness Desk/runtime/dsh/0.1.0-rc.7/node_modules/.bin/dsh web --port 3100",
            executable,
            PORT_START,
            PORT_END
        ));
        assert!(!is_managed_harness_command(
            "node /Users/example/Library/Application Support/DeepSeek Harness Desk/runtime/dsh/0.1.0-rc.7/node_modules/.bin/dsh web --port 3080 --verbose",
            executable,
            PORT_START,
            PORT_END
        ));
        assert!(!is_managed_harness_command(
            "node /tmp/dsh web --port 3080",
            executable,
            PORT_START,
            PORT_END
        ));
    }

    #[test]
    fn versions_compare_numeric_and_prerelease_order() {
        assert!(is_newer_version("0.2.0", "0.1.0"));
        assert!(is_newer_version("0.1.0-rc.10", "0.1.0-rc.6"));
        assert!(is_newer_version("0.1.0", "0.1.0-rc.6"));
        assert!(!is_newer_version("0.1.0-rc.6", "0.1.0"));
        assert!(!is_newer_version("v0.2.0", "0.2.0"));
    }

    #[test]
    fn newest_published_version_prefers_newer_dist_tag() {
        // rc8 is tagged `next` while `latest` still points at rc7.
        assert_eq!(
            newest_published_version(Some("0.1.0-rc.7"), Some("0.1.0-rc.8")).as_deref(),
            Some("0.1.0-rc.8")
        );
        assert_eq!(
            newest_published_version(Some("0.1.0-rc.8"), Some("0.1.0-rc.7")).as_deref(),
            Some("0.1.0-rc.8")
        );
        assert_eq!(
            newest_published_version(Some("0.1.0-rc.8"), Some("0.1.0-rc.8")).as_deref(),
            Some("0.1.0-rc.8")
        );
        assert_eq!(
            newest_published_version(Some("0.1.0-rc.8"), None).as_deref(),
            Some("0.1.0-rc.8")
        );
        assert_eq!(
            newest_published_version(None, Some("0.1.0-rc.8")).as_deref(),
            Some("0.1.0-rc.8")
        );
        assert_eq!(newest_published_version(None, None), None);
    }

    #[test]
    fn newest_of_published_versions_ignores_empty_and_missing_tags() {
        assert_eq!(
            newest_of_published_versions(&[
                Some("0.1.5-rc.2"),
                Some("  "),
                None,
                Some("0.1.6-alpha.2")
            ])
            .as_deref(),
            Some("0.1.6-alpha.2")
        );
        assert_eq!(newest_of_published_versions(&[None, Some("")]), None);
    }

    #[test]
    fn update_channel_defaults_to_stable() {
        assert_eq!(
            DshUpdateChannel::from_request(Some("preview")),
            DshUpdateChannel::Preview
        );
        assert_eq!(
            DshUpdateChannel::from_request(Some(" BETA ")),
            DshUpdateChannel::Preview
        );
        assert_eq!(
            DshUpdateChannel::from_request(Some("stable")),
            DshUpdateChannel::Stable
        );
        // A missing or unexpected value must never opt into previews.
        assert_eq!(
            DshUpdateChannel::from_request(None),
            DshUpdateChannel::Stable
        );
        assert_eq!(
            DshUpdateChannel::from_request(Some("nightly")),
            DshUpdateChannel::Stable
        );
        assert_eq!(DshUpdateChannel::Stable.as_str(), "stable");
        assert_eq!(DshUpdateChannel::Preview.as_str(), "preview");
    }

    #[test]
    fn preview_versions_are_recognized_by_prerelease_label() {
        assert!(is_preview_dsh_version("0.1.6-alpha.2"));
        assert!(is_preview_dsh_version("0.1.6-beta.1"));
        assert!(is_preview_dsh_version("v0.1.6-Alpha.1"));
        // Release candidates ship on the stable channel.
        assert!(!is_preview_dsh_version("0.1.5-rc.2"));
        assert!(!is_preview_dsh_version("0.1.5"));
        assert!(!is_preview_dsh_version("0.1.6-alphabet.1"));
    }

    #[test]
    fn package_versions_are_safe_paths() {
        assert!(is_safe_package_version("0.1.0-rc.7"));
        assert!(!is_safe_package_version("../../tmp"));
        assert!(!is_safe_package_version("0.1.0 rc.7"));
    }

    #[test]
    fn pinned_version_only_accepts_a_plain_version() {
        assert_eq!(
            pinned_version_from_contents("0.1.2-rc.1\n"),
            Some("0.1.2-rc.1".to_string())
        );
        // A corrupt or handwritten pin file must never be trusted.
        assert_eq!(pinned_version_from_contents(""), None);
        assert_eq!(pinned_version_from_contents("../0.1.2"), None);
        assert_eq!(pinned_version_from_contents(".active-version"), None);
        assert_eq!(pinned_version_from_contents("latest"), None);
    }

    #[test]
    fn active_version_prefers_the_pin_over_the_newest_build() {
        let installed = vec![
            ("0.1.6-alpha.2".to_string(), true),
            ("0.1.5-rc.2".to_string(), true),
            ("0.1.2-rc.1".to_string(), true),
        ];
        // A downgrade pins an older version and the launcher must honour it.
        assert_eq!(
            select_active_dsh_version(&installed, Some("0.1.2-rc.1")),
            Some("0.1.2-rc.1".to_string())
        );
        assert_eq!(
            select_active_dsh_version(&installed, None),
            Some("0.1.6-alpha.2".to_string())
        );
    }

    #[test]
    fn active_version_ignores_unusable_pins_and_builds() {
        let installed = vec![
            ("0.1.6-alpha.2".to_string(), false),
            ("0.1.5-rc.2".to_string(), true),
        ];
        // Pinned but not installed: fall back to the newest usable build.
        assert_eq!(
            select_active_dsh_version(&installed, Some("0.1.2-rc.1")),
            Some("0.1.5-rc.2".to_string())
        );
        // Pinned but its dsh binary is missing: same fallback.
        assert_eq!(
            select_active_dsh_version(&installed, Some("0.1.6-alpha.2")),
            Some("0.1.5-rc.2".to_string())
        );
        assert_eq!(select_active_dsh_version(&[], Some("0.1.5-rc.2")), None);
        assert_eq!(
            select_active_dsh_version(&[("0.1.5-rc.2".to_string(), false)], None),
            None
        );
    }

    /// The version picker reads every published version, so the metadata
    /// fixture keeps the (large) per-version manifests: parsing must ignore
    /// them instead of failing on the unknown fields.
    #[test]
    fn npm_metadata_yields_tags_and_every_published_version() {
        let raw = r#"{
            "name": "@deepseek-ai/dsh",
            "dist-tags": {"latest": "0.1.5-rc.2", "next": "0.1.5-rc.1", "alpha": "0.1.6-alpha.2"},
            "versions": {
                "0.1.5-rc.2": {"name": "@deepseek-ai/dsh", "version": "0.1.5-rc.2", "dist": {"tarball": "https://example.test/a.tgz"}},
                "0.1.6-alpha.2": {"name": "@deepseek-ai/dsh", "version": "0.1.6-alpha.2"}
            }
        }"#;
        let metadata: NpmMetadata = serde_json::from_str(raw).expect("npm metadata must parse");
        let (stable, preview) = published_dsh_versions(&metadata);
        assert_eq!(stable, Some("0.1.5-rc.2".to_string()));
        assert_eq!(preview, Some("0.1.6-alpha.2".to_string()));
        let mut versions = metadata
            .versions
            .expect("versions must be captured")
            .into_keys()
            .collect::<Vec<_>>();
        versions.sort_by(|left, right| compare_versions(right, left));
        assert_eq!(versions, vec!["0.1.6-alpha.2", "0.1.5-rc.2"]);
    }

    #[test]
    fn published_versions_keep_the_two_streams_apart() {
        let metadata = NpmMetadata {
            dist_tags: NpmDistTags {
                latest: Some("0.1.5-rc.2".to_string()),
                next: Some("0.1.5-rc.1".to_string()),
                alpha: Some("0.1.6-alpha.2".to_string()),
                beta: None,
            },
            versions: None,
        };
        let (stable, preview) = published_dsh_versions(&metadata);
        assert_eq!(stable, Some("0.1.5-rc.2".to_string()));
        assert_eq!(preview, Some("0.1.6-alpha.2".to_string()));
    }

    #[test]
    fn macos_system_https_proxy_matches_browser_proxy() {
        let output = r#"
<dictionary> {
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7890
  HTTPSProxy : 127.0.0.1
}
"#;

        assert_eq!(
            macos_proxy_url_from_scutil(output, "https").as_deref(),
            Some("http://127.0.0.1:7890")
        );
    }

    #[test]
    fn request_send_failures_are_retryable() {
        assert!(is_retryable_download_error(
            "error sending request for url (https://github.com/example)"
        ));
    }

    #[test]
    fn app_update_selects_the_current_macos_architecture() {
        let release = GithubRelease {
            tag_name: "v0.3.9".to_string(),
            html_url: "https://github.com/example/release".to_string(),
            body: None,
            assets: vec![
                GithubAsset {
                    name: "DeepSeek.Harness.Desk_x64.app.tar.gz".to_string(),
                    browser_download_url: "https://example/x64".to_string(),
                    digest: None,
                    size: None,
                },
                GithubAsset {
                    name: "DeepSeek.Harness.Desk_aarch64.app.tar.gz".to_string(),
                    browser_download_url: "https://example/aarch64".to_string(),
                    digest: None,
                    size: None,
                },
            ],
        };
        let selected = platform_app_asset(&release).expect("architecture asset");

        #[cfg(target_arch = "aarch64")]
        assert!(selected.name.ends_with("_aarch64.app.tar.gz"));
        #[cfg(target_arch = "x86_64")]
        assert!(selected.name.ends_with("_x64.app.tar.gz"));
    }

    #[test]
    fn interaction_keys_are_stable_and_unique() {
        let approval = serde_json::json!({ "approvalId": "approval-1" });
        assert_eq!(interaction_key(&approval, "rpc-a"), "a:approval-1");

        let question = serde_json::json!({ "questions": [{ "id": "q-1", "question": "继续？" }] });
        assert_eq!(interaction_key(&question, "rpc-a"), "q:q-1");

        // Unknown shapes fall back to the wire rpcId and stay distinct.
        let fallback = serde_json::json!({});
        assert_eq!(interaction_key(&fallback, "rpc-a"), "q:rpc-a");
        assert_ne!(
            interaction_key(&fallback, "rpc-a"),
            interaction_key(&fallback, "rpc-b")
        );
    }

    #[test]
    fn interaction_dedup_is_bounded() {
        let mut seen = VecDeque::new();
        assert!(is_new_interaction(&mut seen, "a:1"));
        assert!(!is_new_interaction(&mut seen, "a:1"));
        assert!(is_new_interaction(&mut seen, "a:2"));
        for index in 0..MAX_SEEN_INTERACTIONS {
            assert!(is_new_interaction(&mut seen, &format!("k:{index}")));
        }
        assert!(seen.len() <= MAX_SEEN_INTERACTIONS);
        // A key evicted by the bound is treated as new again.
        assert!(is_new_interaction(&mut seen, "a:1"));
    }

    fn mux_question_item(event: &str, event_id: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": {
                "type": "waterfall",
                "event": event,
                "eventId": event_id,
                "request": { "questions": [{ "id": "q-1", "question": "继续吗？" }] }
            }
        })
    }

    fn mux_approval_item(event: &str, event_id: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": {
                "type": "waterfall",
                "event": event,
                "eventId": event_id,
                "request": { "toolName": "bash", "reason": "rm -rf" }
            }
        })
    }

    #[test]
    fn mux_open_frame_matches_the_stream_protocol() {
        let frame: serde_json::Value =
            serde_json::from_str(&events_stream_open_frame()).expect("valid JSON");
        assert_eq!(frame["type"], "open");
        assert_eq!(frame["streamId"], EVENTS_STREAM_ID);
        assert_eq!(frame["endpoint"], "$events");
        assert_eq!(frame["payload"], serde_json::json!({ "args": {} }));
    }

    #[test]
    fn mux_waterfall_frames_yield_interaction_notices() {
        match classify_mux_message(
            &mux_question_item("user-questions/request", "evt-1").to_string(),
        ) {
            HarnessInbound::Notice(HarnessNotice::InteractionRequested {
                key,
                kind,
                session_id,
                detail,
            }) => {
                assert_eq!(key, "q:q-1");
                assert_eq!(kind, NoticeKind::Question);
                assert_eq!(detail.as_deref(), Some("继续吗？"));
                assert_eq!(session_id, None);
            }
            other => panic!("unexpected inbound: {other:?}"),
        }

        match classify_mux_message(&mux_approval_item("approval/request", "evt-2").to_string()) {
            HarnessInbound::Notice(HarnessNotice::InteractionRequested {
                key,
                kind,
                session_id,
                detail,
            }) => {
                assert_eq!(key, "q:evt-2");
                assert_eq!(kind, NoticeKind::Approval);
                assert_eq!(detail.as_deref(), Some("bash"));
                assert_eq!(session_id, None);
            }
            other => panic!("unexpected inbound: {other:?}"),
        }
    }

    #[test]
    fn mux_replays_dedup_through_a_stable_key() {
        // Reconnects re-deliver still-pending requests verbatim; both passes
        // must classify to the same dedup key so only one notice fires.
        let frame = mux_question_item("user-questions/request", "evt-1");
        let keys: Vec<String> = [0, 1]
            .iter()
            .map(|_| match classify_mux_message(&frame.to_string()) {
                HarnessInbound::Notice(HarnessNotice::InteractionRequested { key, .. }) => key,
                other => panic!("unexpected inbound: {other:?}"),
            })
            .collect();
        assert_eq!(keys[0], keys[1]);
    }

    #[test]
    fn mux_emit_frames_track_session_running() {
        let status = |running: bool| {
            serde_json::json!({
                "type": "item",
                "streamId": "desk-events",
                "value": {
                    "type": "emit",
                    "event": "api-session/status",
                    "args": ["sess-1", running]
                }
            })
        };
        match classify_mux_message(&status(true).to_string()) {
            HarnessInbound::Notice(HarnessNotice::SessionRunningChanged {
                session_id,
                running,
            }) => {
                assert_eq!(session_id, "sess-1");
                assert!(running);
            }
            other => panic!("unexpected inbound: {other:?}"),
        }
        match classify_mux_message(&status(false).to_string()) {
            HarnessInbound::Notice(HarnessNotice::SessionRunningChanged { running, .. }) => {
                assert!(!running);
            }
            other => panic!("unexpected inbound: {other:?}"),
        }
    }

    #[test]
    fn mux_ignores_bookkeeping_and_unknown_events() {
        let ready = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": { "type": "ready", "clientId": "c", "host": "/" }
        });
        assert_eq!(classify_mux_message(&ready.to_string()), HarnessInbound::Ignore);

        let cancel = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": { "type": "cancel", "eventId": "evt-9" }
        });
        assert_eq!(classify_mux_message(&cancel.to_string()), HarnessInbound::Ignore);

        // Future dsh releases may forward events the shell has never heard of.
        let unknown_event = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": { "type": "emit", "event": "brand-new/event", "args": [1, 2] }
        });
        assert_eq!(
            classify_mux_message(&unknown_event.to_string()),
            HarnessInbound::Ignore
        );

        let unknown_message = serde_json::json!({ "type": "something-new", "streamId": "desk-events" });
        assert_eq!(
            classify_mux_message(&unknown_message.to_string()),
            HarnessInbound::Ignore
        );

        assert_eq!(classify_mux_message("not json"), HarnessInbound::Ignore);
    }

    #[test]
    fn mux_stream_failure_frames_ask_for_reconnect() {
        assert_eq!(
            classify_mux_message(r#"{ "type": "end", "streamId": "desk-events" }"#),
            HarnessInbound::StreamFailed
        );
        assert_eq!(
            classify_mux_message(
                r#"{ "type": "error", "streamId": "desk-events", "error": { "message": "boom" } }"#
            ),
            HarnessInbound::StreamFailed
        );
    }

    #[test]
    fn forwarded_event_names_stay_compatible_across_generations() {
        assert!(matches!(
            forwarded_event_kind("user-questions/request"),
            Some(ForwardedEventKind::Question)
        ));
        assert!(matches!(
            forwarded_event_kind("question/requested"),
            Some(ForwardedEventKind::Question)
        ));
        assert!(matches!(
            forwarded_event_kind("approval/request"),
            Some(ForwardedEventKind::Approval)
        ));
        assert!(matches!(
            forwarded_event_kind("api-session/status"),
            Some(ForwardedEventKind::SessionStatus)
        ));
        assert!(matches!(
            forwarded_event_kind("host/session-status"),
            Some(ForwardedEventKind::SessionStatus)
        ));
        assert!(matches!(
            forwarded_event_kind("api-session/error"),
            Some(ForwardedEventKind::SessionError)
        ));
        assert!(matches!(
            forwarded_event_kind("api-session/removed"),
            Some(ForwardedEventKind::SessionRemoved)
        ));
        assert!(forwarded_event_kind("unrelated/event").is_none());
    }

    #[test]
    fn legacy_mux_envelopes_still_yield_interaction_notices() {
        let question = serde_json::json!({
            "rpcId": "rpc-1",
            "payload": {
                "type": "question/requested",
                "sessionId": "s1",
                "questions": [{ "id": "q-9", "question": "选择目录" }]
            }
        });
        assert_eq!(
            classify_legacy_mux_frame(&question.to_string()),
            Some(HarnessNotice::InteractionRequested {
                key: "q:q-9".to_string(),
                kind: NoticeKind::Question,
                session_id: Some("s1".to_string()),
                detail: Some("选择目录".to_string()),
            })
        );

        let approval = serde_json::json!({
            "rpcId": "rpc-2",
            "payload": {
                "type": "approval/requested",
                "sessionId": "s1",
                "approvalId": "ap-1",
                "toolName": "bash"
            }
        });
        assert_eq!(
            classify_legacy_mux_frame(&approval.to_string()),
            Some(HarnessNotice::InteractionRequested {
                key: "a:ap-1".to_string(),
                kind: NoticeKind::Approval,
                session_id: Some("s1".to_string()),
                detail: None,
            })
        );

        assert_eq!(classify_legacy_mux_frame("{}"), None);
    }

    #[test]
    fn legacy_host_envelopes_still_track_sessions() {
        let frame = serde_json::json!({
            "rpcId": "rpc-3",
            "payload": { "type": "host/session-status", "sessionId": "s1", "running": false }
        });
        assert_eq!(
            classify_legacy_host_frame(&frame.to_string()),
            Some(HarnessNotice::SessionRunningChanged {
                session_id: "s1".to_string(),
                running: false,
            })
        );
        assert_eq!(classify_legacy_host_frame("{}"), None);
    }

    #[test]
    fn empty_details_do_not_become_blank_notice_bodies() {
        let bare = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": {
                "type": "waterfall",
                "event": "user-questions/request",
                "eventId": "evt-3",
                "request": { "questions": [] }
            }
        });
        match classify_mux_message(&bare.to_string()) {
            HarnessInbound::Notice(HarnessNotice::InteractionRequested { detail, .. }) => {
                assert_eq!(detail, None);
            }
            other => panic!("unexpected inbound: {other:?}"),
        }
    }

    #[test]
    fn mux_error_frames_yield_failure_notices() {
        let frame = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": {
                "type": "emit",
                "event": "api-session/error",
                "args": ["sess-1", "Model request failed"]
            }
        });
        assert_eq!(
            classify_mux_message(&frame.to_string()),
            HarnessInbound::Notice(HarnessNotice::SessionFailed {
                session_id: "sess-1".to_string(),
                detail: Some("Model request failed".to_string()),
            })
        );

        // Without a session there is nothing to attribute the failure to.
        let anonymous = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": { "type": "emit", "event": "api-session/error", "args": [] }
        });
        assert_eq!(
            classify_mux_message(&anonymous.to_string()),
            HarnessInbound::Ignore
        );
    }

    #[test]
    fn mux_removed_frames_yield_session_cleanup_notices() {
        let frame = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": { "type": "emit", "event": "api-session/removed", "args": ["sess-9"] }
        });
        assert_eq!(
            classify_mux_message(&frame.to_string()),
            HarnessInbound::Notice(HarnessNotice::SessionRemoved {
                session_id: "sess-9".to_string(),
            })
        );
    }

    #[test]
    fn plan_review_questions_are_a_distinct_notice_kind() {
        let plan = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": {
                "type": "waterfall",
                "event": "user-questions/request",
                "eventId": "evt-p",
                "request": { "questions": [{
                    "id": "q-1",
                    "question": "按计划执行？",
                    "intent": { "kind": "plan-review", "approve": "批准" }
                }] }
            }
        });
        match classify_mux_message(&plan.to_string()) {
            HarnessInbound::Notice(HarnessNotice::InteractionRequested { kind, .. }) => {
                assert_eq!(kind, NoticeKind::PlanReview);
            }
            other => panic!("unexpected inbound: {other:?}"),
        }

        // `intent` is optional presentation metadata; without it this is a plain
        // question, and an unrelated intent must not read as a plan review.
        assert_eq!(
            question_notice_kind(&mux_question_item("user-questions/request", "evt-q")),
            NoticeKind::Question,
        );
        let unrelated_intent = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": {
                "type": "waterfall",
                "event": "user-questions/request",
                "eventId": "evt-u",
                "request": { "questions": [{
                    "id": "q-1", "question": "选哪个？", "intent": { "kind": "something-new" }
                }] }
            }
        });
        assert_eq!(
            question_notice_kind(&unrelated_intent),
            NoticeKind::Question
        );
    }

    fn question_notice_kind(frame: &serde_json::Value) -> NoticeKind {
        match classify_mux_message(&frame.to_string()) {
            HarnessInbound::Notice(HarnessNotice::InteractionRequested { kind, .. }) => kind,
            other => panic!("unexpected inbound: {other:?}"),
        }
    }

    #[test]
    fn interaction_notices_report_the_session_they_came_from() {
        let frame = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": {
                "type": "waterfall",
                "event": "approval/request",
                "eventId": "evt-s",
                "request": { "sessionId": "sess-7", "toolName": "bash" }
            }
        });
        match classify_mux_message(&frame.to_string()) {
            HarnessInbound::Notice(HarnessNotice::InteractionRequested { session_id, .. }) => {
                assert_eq!(session_id.as_deref(), Some("sess-7"));
            }
            other => panic!("unexpected inbound: {other:?}"),
        }

        // Newer generations project the identity through the agent instead.
        let projected = serde_json::json!({
            "type": "item",
            "streamId": "desk-events",
            "value": {
                "type": "waterfall",
                "event": "approval/request",
                "eventId": "evt-s2",
                "request": { "agent": { "sessionId": "sess-8" }, "toolName": "bash" }
            }
        });
        match classify_mux_message(&projected.to_string()) {
            HarnessInbound::Notice(HarnessNotice::InteractionRequested { session_id, .. }) => {
                assert_eq!(session_id.as_deref(), Some("sess-8"));
            }
            other => panic!("unexpected inbound: {other:?}"),
        }
    }

    fn status(session_id: &str, running: bool) -> HarnessNotice {
        HarnessNotice::SessionRunningChanged {
            session_id: session_id.to_string(),
            running,
        }
    }

    fn approval(session_id: Option<&str>) -> HarnessNotice {
        HarnessNotice::InteractionRequested {
            key: format!("a:{}", session_id.unwrap_or("none")),
            kind: NoticeKind::Approval,
            session_id: session_id.map(str::to_string),
            detail: Some("bash".to_string()),
        }
    }

    fn completed(session_id: &str) -> HarnessNotice {
        HarnessNotice::TaskCompleted {
            session_id: session_id.to_string(),
        }
    }

    #[test]
    fn first_idle_only_sets_a_baseline() {
        // A page that just connected reports every already-idle session; none of
        // them is a task the user was waiting on.
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        assert!(ledger.route(status("sess-1", false), now).is_empty());
        assert!(ledger.deadline().is_none());
        assert!(ledger.due(now + COMPLETION_DEBOUNCE).is_empty());
    }

    #[test]
    fn one_completion_fires_after_the_wait_and_never_again() {
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        for running in [true, true, false, false] {
            assert!(ledger.route(status("sess-1", running), now).is_empty());
        }
        assert!(
            ledger.due(now).is_empty(),
            "the completion waits for its window"
        );
        assert_eq!(ledger.deadline(), Some(now + COMPLETION_DEBOUNCE));
        assert_eq!(
            ledger.due(now + COMPLETION_DEBOUNCE),
            vec![completed("sess-1")]
        );
        assert!(ledger.due(now + COMPLETION_DEBOUNCE * 2).is_empty());
    }

    #[test]
    fn an_interaction_covers_the_completion_of_the_same_turn() {
        // The order Harness actually sends: the agent parks on the approval and
        // only then reports idle. One turn, one banner.
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        ledger.route(status("sess-1", true), now);
        assert_eq!(
            ledger.route(approval(Some("sess-1")), now),
            vec![approval(Some("sess-1"))]
        );
        assert!(ledger.route(status("sess-1", false), now).is_empty());
        assert!(ledger.due(now + COMPLETION_DEBOUNCE * 2).is_empty());
    }

    #[test]
    fn an_interaction_arriving_inside_the_window_replaces_the_completion() {
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        ledger.route(status("sess-1", true), now);
        assert!(ledger.route(status("sess-1", false), now).is_empty());
        assert_eq!(
            ledger.route(approval(Some("sess-1")), now),
            vec![approval(Some("sess-1"))]
        );
        assert!(ledger.due(now + COMPLETION_DEBOUNCE * 2).is_empty());
    }

    #[test]
    fn a_failure_covers_the_completion_of_the_same_turn() {
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        ledger.route(status("sess-1", true), now);
        let failure = HarnessNotice::SessionFailed {
            session_id: "sess-1".to_string(),
            detail: Some("boom".to_string()),
        };
        assert_eq!(ledger.route(failure.clone(), now), vec![failure]);
        assert!(ledger.route(status("sess-1", false), now).is_empty());
        assert!(ledger.due(now + COMPLETION_DEBOUNCE * 2).is_empty());
    }

    #[test]
    fn a_new_turn_clears_the_previous_one() {
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        ledger.route(status("sess-1", true), now);
        ledger.route(approval(Some("sess-1")), now);
        // The user approves, the agent resumes, and the turn that follows is
        // genuinely new work whose completion is worth reporting.
        ledger.route(status("sess-1", true), now);
        ledger.route(status("sess-1", false), now);
        assert_eq!(
            ledger.due(now + COMPLETION_DEBOUNCE),
            vec![completed("sess-1")]
        );
    }

    #[test]
    fn a_removed_session_stops_producing_notices() {
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        ledger.route(status("sess-1", true), now);
        ledger.route(status("sess-1", false), now);
        ledger.route(
            HarnessNotice::SessionRemoved {
                session_id: "sess-1".to_string(),
            },
            now,
        );
        assert!(ledger.deadline().is_none());
        assert!(ledger.due(now + COMPLETION_DEBOUNCE * 2).is_empty());
        // A replayed status for the dead session is a baseline again, not a
        // completion borrowed from the session that used to own it.
        assert!(ledger.route(status("sess-1", false), now).is_empty());
    }

    #[test]
    fn pending_completions_are_tracked_per_session() {
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        for session in ["sess-a", "sess-b"] {
            ledger.route(status(session, true), now);
            ledger.route(status(session, false), now);
        }
        // An interaction for one session must not silence the other's completion.
        ledger.route(approval(Some("sess-a")), now);
        let due = ledger.due(now + COMPLETION_DEBOUNCE);
        assert_eq!(due, vec![completed("sess-b")]);
    }

    #[test]
    fn a_replayed_interaction_banners_once() {
        // Reconnects re-deliver still-pending requests: the second pass must be
        // silent, and the turn it covers must still not produce a completion.
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        ledger.route(status("sess-1", true), now);
        assert_eq!(ledger.route(approval(Some("sess-1")), now).len(), 1);
        assert!(
            ledger.route(approval(Some("sess-1")), now).is_empty(),
            "reconnect replay"
        );
        assert!(ledger.route(status("sess-1", false), now).is_empty());
        assert!(ledger.due(now + COMPLETION_DEBOUNCE).is_empty());
    }

    #[test]
    fn reset_forgets_the_previous_backend() {
        let mut ledger = AttentionLedger::default();
        let now = Instant::now();
        ledger.route(status("sess-1", true), now);
        ledger.route(status("sess-1", false), now);
        ledger.route(approval(Some("sess-2")), now);
        ledger.reset();
        assert!(ledger.deadline().is_none());
        // Dedup keys survive on purpose: dsh replays still-pending requests to
        // the new connection, and a replay must not raise a second banner.
        assert!(ledger.route(approval(Some("sess-2")), now).is_empty());
    }

    fn disposition(url: &str) -> NavigationDisposition {
        navigation_disposition(&Url::parse(url).expect("test URL"))
    }

    #[test]
    fn shell_and_harness_pages_navigate_in_app() {
        for url in [
            // The shell document itself, plus the blank/blob/data documents the
            // frame code uses while loading or unloading the Harness page.
            "tauri://localhost/index.html",
            "about:blank",
            "blob:tauri://localhost/2f0e",
            "data:text/html,<p>hi</p>",
            // The Harness origin is a loopback port picked at every start.
            "http://127.0.0.1:3080/",
            "http://127.0.0.1:3099/?token=abc",
            "http://localhost:3080/api/remote.mux",
            "http://127.6.7.8:3080/",
            "http://[::1]:3080/",
        ] {
            assert_eq!(disposition(url), NavigationDisposition::InApp, "{url}");
        }
    }

    #[test]
    fn web_links_open_in_the_default_browser() {
        for url in [
            "https://github.com/misswell/deepseek-harness-desk/releases",
            "http://example.com/",
            "mailto:someone@example.com",
            // A host that merely looks local is still somebody else's website.
            "https://127.0.0.1.example.com/",
            "https://notlocalhost.example/",
        ] {
            assert_eq!(disposition(url), NavigationDisposition::External, "{url}");
        }
    }

    #[test]
    fn unknown_schemes_stay_with_webkit() {
        // A scheme the shell does not understand may be answerable by an
        // installed handler, so it is not treated as a web link.
        assert_eq!(
            disposition("deepseek-harness://open?file=notes.md"),
            NavigationDisposition::InApp
        );
        assert_eq!(
            disposition("vscode://file/tmp/demo.rs"),
            NavigationDisposition::InApp
        );
    }

    #[test]
    fn external_links_reject_non_web_schemes() {
        // Only the schemes a browser or mail client can answer are handed over.
        assert!(is_openable_web_url("https://example.com/release.zip"));
        assert!(is_openable_web_url("http://example.com/"));
        assert!(is_openable_web_url("mailto:someone@example.com"));
        assert!(!is_openable_web_url("file:///etc/passwd"));
        assert!(!is_openable_web_url("deepseek-harness://open"));
        assert!(!is_openable_web_url("javascript:alert(1)"));
        assert!(!is_openable_web_url("//example.com"));
    }
}
