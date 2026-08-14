use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::env;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::menu::{MenuBuilder, MenuItem};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewWindow, WindowEvent};

const PORT_START: u16 = 3080;
const PORT_END: u16 = 3099;
const MAX_LOG_LINES: usize = 500;

#[derive(Clone)]
struct HarnessState {
    lifecycle: Arc<Mutex<()>>,
    child: Arc<Mutex<Option<Child>>>,
    port: Arc<AtomicU16>,
    dsh_path: Arc<Mutex<Option<PathBuf>>>,
    logs: Arc<Mutex<VecDeque<HarnessLog>>>,
    last_error: Arc<Mutex<Option<String>>>,
    last_exit_code: Arc<Mutex<Option<i32>>>,
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

fn dsh_candidates() -> Vec<PathBuf> {
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

    // Keep discovering the runtime created by the old Swift client so an
    // existing installation can be reused by the Tauri app.
    if let Some(home) = home_directory() {
        let runtime_dsh = if cfg!(target_os = "macos") {
            home.join("Library/Application Support/DeepSeek Harness Desk/runtime/dsh")
        } else if cfg!(windows) {
            env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData/Roaming"))
                .join("DeepSeek Harness Desk/runtime/dsh")
        } else {
            home.join(".local/share/DeepSeek Harness Desk/runtime/dsh")
        };

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

fn managed_node_bin_directories() -> Vec<PathBuf> {
    let Some(home) = home_directory() else {
        return Vec::new();
    };

    let runtime_node = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/DeepSeek Harness Desk/runtime/node")
    } else if cfg!(windows) {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData/Roaming"))
            .join("DeepSeek Harness Desk/runtime/node")
    } else {
        home.join(".local/share/DeepSeek Harness Desk/runtime/node")
    };

    let mut directories = Vec::new();
    if let Ok(versions) = std::fs::read_dir(runtime_node) {
        for version in versions.flatten() {
            let bin = version.path().join("bin");
            if bin.is_dir() {
                directories.push(bin);
            }
        }
    }
    directories
}

fn dsh_command() -> Option<DshCommand> {
    dsh_candidates()
        .into_iter()
        .find(|candidate| is_executable(candidate))
        .map(|program| DshCommand { program })
}

fn process_path(program: &Path) -> String {
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
    paths.extend(managed_node_bin_directories());
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

fn spawn_dsh(command: &DshCommand, port: u16) -> std::io::Result<Child> {
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
        .env("PATH", process_path(&command.program))
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

    let Some(command) = dsh_command() else {
        let message = "未找到 dsh 可执行文件。请先安装 @deepseek-ai/dsh，或设置 DSH_BIN 环境变量。";
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

    let mut child = spawn_dsh(&command, port).map_err(|error| {
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
fn runtime_status() -> RuntimeStatus {
    let path = dsh_command().map(|command| command.program.to_string_lossy().into_owned());
    RuntimeStatus {
        available: path.is_some(),
        path,
    }
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
        let focused_window = app
            .get_webview_window("main")
            .filter(|window| window.is_focused().unwrap_or(false));
        app.set_dock_visibility(visible)
            .map_err(|error| error.to_string())?;
        if let Some(window) = focused_window {
            let _ = window.set_focus();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, visible);
    }
    Ok(())
}

fn show_main_window<R: tauri::Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
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
    };

    tauri::Builder::default()
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
        let candidates = dsh_candidates();
        assert!(!candidates.is_empty());
    }
}
