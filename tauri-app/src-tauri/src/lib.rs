use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cmp::Ordering as VersionOrdering;
use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tauri::menu::{MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewWindow, WindowEvent};

const PORT_START: u16 = 3080;
const PORT_END: u16 = 3099;
const MAX_LOG_LINES: usize = 500;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const NODE_VERSION: &str = "24.19.0";
const DSH_VERSION: &str = "0.1.0-rc.6";
const DSH_PACKAGE: &str = "@deepseek-ai/dsh";
const RELEASES_URL: &str =
    "https://api.github.com/repos/misswell/deepseek-harness-desk/releases/latest";
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
    dsh_path: Arc<Mutex<Option<PathBuf>>>,
    logs: Arc<Mutex<VecDeque<HarnessLog>>>,
    last_error: Arc<Mutex<Option<String>>>,
    last_exit_code: Arc<Mutex<Option<i32>>>,
    runtime_installing: Arc<AtomicBool>,
    runtime_message: Arc<Mutex<String>>,
    app_update_installing: Arc<AtomicBool>,
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
}

#[derive(Deserialize, Debug)]
struct NpmDistTags {
    latest: Option<String>,
}

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

fn managed_dsh_version(app: Option<&AppHandle>) -> Option<String> {
    let executable = dsh_executable;
    managed_dsh_version_paths(app)
        .into_iter()
        .find(|(_, path)| is_executable(&executable(path)))
        .map(|(version, _)| version)
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
    // app-data runtime created by the first-run installer. Always prefer the
    // newest managed package so an installed dsh update becomes active.
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

    process
        .args(["web", "--port", &port.to_string()])
        .current_dir(home_directory().unwrap_or_else(|| PathBuf::from(".")))
        .env("PATH", process_path(&command.program, app))
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
        asset(&|name| name.ends_with(".app.tar.gz"))
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

async fn check_dsh_update_inner(app: &AppHandle) -> Result<DshUpdateStatus, String> {
    let current_version = managed_dsh_version(Some(app));
    if current_version.is_none() {
        return Ok(DshUpdateStatus {
            managed: false,
            current_version: None,
            latest_version: None,
            available: false,
            status: "尚未安装内置 dsh，完成一键安装后可检查更新。".to_string(),
        });
    }
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
    let metadata = response
        .json::<NpmMetadata>()
        .await
        .map_err(|error| format!("解析 dsh 更新信息失败：{error}"))?;
    let latest = metadata
        .dist_tags
        .latest
        .filter(|version| !version.is_empty())
        .ok_or("npm 未返回 dsh 的 latest 版本。")?;
    let current = current_version.unwrap_or_default();
    let available = is_newer_version(&latest, &current);
    Ok(DshUpdateStatus {
        managed: true,
        current_version: Some(current.clone()),
        latest_version: Some(latest.clone()),
        available,
        status: if available {
            format!("发现内置 dsh 新版本 {latest}")
        } else {
            format!("内置 dsh 已是最新版本 {current}")
        },
    })
}

async fn install_dsh_update_inner(
    app: &AppHandle,
    state: &HarnessState,
    version: String,
) -> Result<RuntimeStatus, String> {
    if !is_safe_package_version(&version) {
        return Err("dsh 版本号无效。".to_string());
    }
    if state.runtime_installing.swap(true, Ordering::AcqRel) {
        return Err("运行时更新正在进行，请等待当前更新完成。".to_string());
    }
    let was_running = snapshot(state).running;
    if was_running {
        stop_harness_inner(app, state);
    }
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
        emit_runtime_progress(
            app,
            state,
            format!("正在安装内置 dsh {version}…"),
            Some(0.08),
            false,
            None,
        );
        install_dsh_package(app, state, &node_root, &dsh_staging, &version)?;
        emit_runtime_progress(app, state, "正在校验内置 dsh…", Some(0.82), false, None);
        if !is_executable(&dsh_executable(&dsh_staging)) {
            return Err("npm 安装完成，但没有生成 dsh 命令。".to_string());
        }
        fs::create_dir_all(runtime_root.join("dsh"))
            .map_err(|error| format!("创建 dsh 版本目录失败：{error}"))?;
        if dsh_root.exists() {
            fs::remove_dir_all(&dsh_root).map_err(|error| format!("替换 dsh 版本失败：{error}"))?;
        }
        fs::rename(&dsh_staging, &dsh_root)
            .map_err(|error| format!("保存 dsh 更新失败：{error}"))?;
        let _ = fs::remove_dir_all(&staging);
        emit_runtime_progress(app, state, "内置 dsh 更新完成。", Some(1.0), true, None);
        Ok(())
    }
    .await;
    state.runtime_installing.store(false, Ordering::Release);
    if let Err(error) = &result {
        if was_running {
            let _ = start_harness_inner(app, state).await;
        }
        emit_runtime_progress(
            app,
            state,
            "内置 dsh 更新失败。",
            None,
            true,
            Some(error.clone()),
        );
    }
    result?;
    if was_running {
        start_harness_inner(app, state).await?;
    }
    Ok(runtime_status_snapshot(app, state))
}

fn open_external_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("只允许打开 HTTPS 更新地址。".to_string());
    }
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
            if let Ok(mut path) = state.dsh_path.lock() {
                *path = None;
            }
        }
    }

    let port = state.port.load(Ordering::Relaxed);
    let url = running.then(|| format!("http://127.0.0.1:{port}"));
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
    }
}

fn stop_harness_inner(app: &AppHandle, state: &HarnessState) {
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

    let Some(command) = dsh_command(app) else {
        let message = "未找到 dsh 可执行文件。请先点击“安装并启动”，或设置 DSH_BIN 环境变量。";
        set_last_error(state, Some(message.to_string()));
        emit_log(app, &state.logs, "desk", message);
        return Err(message.to_string());
    };

    let Some(port) = first_available_port(PORT_START, PORT_END) else {
        let message = "3080–3099 端口均不可用，请释放端口后重试。";
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
        spawn_output_reader(stdout, app.clone(), state.logs.clone(), "stdout");
    }
    if let Some(stderr) = stderr {
        spawn_output_reader(stderr, app.clone(), state.logs.clone(), "stderr");
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

        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                emit_log(app, &state.logs, "desk", format!("Harness 已就绪：{url}"));
                return Ok(snapshot(state));
            }
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

#[tauri::command]
fn harness_status(state: State<'_, HarnessState>) -> HarnessStatus {
    snapshot(&state)
}

#[tauri::command]
fn runtime_status(app: AppHandle, state: State<'_, HarnessState>) -> RuntimeStatus {
    runtime_status_snapshot(&app, &state)
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
async fn check_dsh_update(app: AppHandle) -> Result<DshUpdateStatus, String> {
    check_dsh_update_inner(&app).await
}

#[tauri::command]
async fn install_dsh_update(
    app: AppHandle,
    state: State<'_, HarnessState>,
    version: String,
) -> Result<RuntimeStatus, String> {
    install_dsh_update_inner(&app, &state, version).await
}

#[tauri::command]
fn open_release_page(url: String) -> Result<(), String> {
    open_external_url(&url)
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
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, visible);
    }
    Ok(())
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

fn show_main_window<R: tauri::Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.show();
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(activate_macos_application);
    }
}

#[cfg(feature = "tray-icon")]
fn setup_tray<R: tauri::Runtime>(app: &mut tauri::App<R>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开窗口", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let start = MenuItem::with_id(app, "start", "启动 Harness", true, None::<&str>)?;
    let restart = MenuItem::with_id(app, "restart", "重启 Harness", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "停止 Harness", true, None::<&str>)?;
    let logs = MenuItem::with_id(app, "logs", "打开运行日志", true, None::<&str>)?;
    let check_app = MenuItem::with_id(app, "check-app", "检查 App 更新…", true, None::<&str>)?;
    let check_dsh = MenuItem::with_id(app, "check-dsh", "检查内置 dsh 更新…", true, None::<&str>)?;
    let quit = MenuItem::with_id(
        app,
        "quit",
        "退出 DeepSeek Harness Desk",
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

    let tray = TrayIconBuilder::with_id("main-tray")
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
        });

    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = HarnessState {
        lifecycle: Arc::new(Mutex::new(())),
        child: Arc::new(Mutex::new(None)),
        port: Arc::new(AtomicU16::new(0)),
        dsh_path: Arc::new(Mutex::new(None)),
        logs: Arc::new(Mutex::new(VecDeque::new())),
        last_error: Arc::new(Mutex::new(None)),
        last_exit_code: Arc::new(Mutex::new(None)),
        runtime_installing: Arc::new(AtomicBool::new(false)),
        runtime_message: Arc::new(Mutex::new(String::new())),
        app_update_installing: Arc::new(AtomicBool::new(false)),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(|app| {
            #[cfg(feature = "tray-icon")]
            setup_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
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
            open_release_page,
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
            set_dock_visibility,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = event {
                show_main_window(app);
            }

            if let RunEvent::ExitRequested { .. } = event {
                let state = app.state::<HarnessState>();
                stop_harness_inner(app, &state);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reversed_port_range_is_empty() {
        assert_eq!(first_available_port(PORT_END, PORT_START), None);
    }

    #[test]
    fn dsh_candidates_include_path_entries() {
        let candidates = dsh_candidates(None);
        assert!(!candidates.is_empty());
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
    fn package_versions_are_safe_paths() {
        assert!(is_safe_package_version("0.1.0-rc.7"));
        assert!(!is_safe_package_version("../../tmp"));
        assert!(!is_safe_package_version("0.1.0 rc.7"));
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
}
