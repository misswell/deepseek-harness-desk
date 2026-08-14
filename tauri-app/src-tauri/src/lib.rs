use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read};
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
const NODE_VERSION: &str = "24.19.0";
const DSH_VERSION: &str = "0.1.0-rc.6";
const DSH_PACKAGE: &str = "@deepseek-ai/dsh";

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
    path: Option<String>,
    installing: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    message: String,
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

    // Reuse both the runtime created by the old Swift client and the Tauri
    // app-data runtime created by the first-run installer.
    for runtime_root in runtime_roots(app) {
        let runtime_dsh = runtime_root.join("dsh");
        for name in &names {
            push_unique(&mut candidates, runtime_dsh.join(name));
        }

        if let Ok(versions) = std::fs::read_dir(&runtime_dsh) {
            for version in versions.flatten() {
                let version_dir = version.path();
                if !version_dir.is_dir() {
                    continue;
                }
                for name in &names {
                    push_unique(
                        &mut candidates,
                        version_dir.join("node_modules/.bin").join(name),
                    );
                }
            }
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
    paths.extend(managed_node_bin_directories(Some(app)));
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
    RuntimeStatus {
        available: dsh_command(app).is_some(),
        path: dsh_command(app).map(|command| command.program.to_string_lossy().into_owned()),
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

async fn download_runtime_archive(url: &str, destination: &Path) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .user_agent("DeepSeek Harness Desk")
        .build()
        .map_err(|error| format!("创建运行时下载客户端失败：{error}"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("下载 Node.js 失败：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("下载 Node.js 失败（HTTP {}）。", response.status()));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("读取 Node.js 下载内容失败：{error}"))?;
    fs::write(destination, bytes).map_err(|error| format!("保存 Node.js 安装包失败：{error}"))
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
        format!("{DSH_PACKAGE}@{DSH_VERSION}"),
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
            let _ = app.emit(
                "harness-output",
                HarnessLog {
                    stream: format!("runtime-{stream}"),
                    message: String::from_utf8_lossy(bytes).trim().to_string(),
                },
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
        None,
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
                None,
                false,
                None,
            );
            let url = format!(
                "https://nodejs.org/dist/v{NODE_VERSION}/{}",
                distribution.archive_name
            );
            download_runtime_archive(&url, &archive).await?;
            emit_runtime_progress(app, state, "正在解压 Node.js…", None, false, None);
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
                None,
                false,
                None,
            );
            install_dsh_package(app, state, &node_root, &dsh_staging)?;
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
    let quit = MenuItem::with_id(
        app,
        "quit",
        "退出 DeepSeek Harness Desk",
        true,
        None::<&str>,
    )?;
    let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;

    let mut tray = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .tooltip("DeepSeek Harness Desk")
        .icon_as_template(cfg!(target_os = "macos"))
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
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

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
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
}
