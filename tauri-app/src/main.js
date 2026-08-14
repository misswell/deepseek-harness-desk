const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke;
const listen = tauri?.event?.listen;

const elements = {
  titlebarStatus: document.querySelector("#titlebar-status"),
  titlebarStatusText: document.querySelector("#titlebar-status-text"),
  sideLiveIndicator: document.querySelector("#side-live-indicator"),
  sideStatusTitle: document.querySelector("#side-status-title"),
  sideStatusDetail: document.querySelector("#side-status-detail"),
  portValue: document.querySelector("#port-value"),
  runtimeValue: document.querySelector("#runtime-value"),
  settingsRuntimePath: document.querySelector("#settings-runtime-path"),
  startButton: document.querySelector("#start-button"),
  restartButton: document.querySelector("#restart-button"),
  stopButton: document.querySelector("#stop-button"),
  emptyStartButton: document.querySelector("#empty-start-button"),
  emptyTitle: document.querySelector("#empty-title"),
  emptyMessage: document.querySelector("#empty-message"),
  emptyState: document.querySelector("#empty-state"),
  frameContainer: document.querySelector("#frame-container"),
  frameLoading: document.querySelector("#frame-loading"),
  frame: document.querySelector("#harness-frame"),
  logsButton: document.querySelector("#logs-button"),
  openLogsButton: document.querySelector("#open-logs-button"),
  logCount: document.querySelector("#log-count"),
  logsDrawer: document.querySelector("#logs-drawer"),
  logList: document.querySelector("#log-list"),
  clearLogsButton: document.querySelector("#clear-logs-button"),
  closeLogsButton: document.querySelector("#close-logs-button"),
  settingsButton: document.querySelector("#settings-button"),
  settingsDrawer: document.querySelector("#settings-drawer"),
  closeSettingsButton: document.querySelector("#close-settings-button"),
  dockIconToggle: document.querySelector("#dock-icon-toggle"),
  refreshRuntimeButton: document.querySelector("#refresh-runtime-button"),
  refreshButton: document.querySelector("#refresh-button"),
  minimizeButton: document.querySelector("#minimize-button"),
  maximizeButton: document.querySelector("#maximize-button"),
  hideButton: document.querySelector("#hide-button"),
  toast: document.querySelector("#toast"),
};

const state = {
  phase: "idle",
  status: null,
  runtime: null,
  logs: [],
  busy: false,
  frameUrl: "",
  toastTimer: null,
};

function errorMessage(error) {
  if (typeof error === "string") return error;
  if (error?.message) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return "未知错误";
  }
}

async function call(command, args) {
  if (!invoke) {
    throw new Error("当前页面不是 Tauri 应用窗口");
  }
  return invoke(command, args);
}

function setBusy(busy) {
  state.busy = busy;
  elements.startButton.disabled = busy || state.phase === "running";
  elements.emptyStartButton.disabled = busy || state.phase === "running";
  elements.restartButton.disabled = busy || !state.status?.running;
  elements.stopButton.disabled = busy || !state.status?.running;
  elements.startButton.classList.toggle("loading", busy);
  elements.startButton.querySelector(".button-icon").textContent = busy ? "◌" : "▶";
}

function setToast(message, isError = false) {
  if (!message) return;
  elements.toast.textContent = message;
  elements.toast.classList.toggle("error", isError);
  elements.toast.classList.remove("hidden");
  clearTimeout(state.toastTimer);
  state.toastTimer = setTimeout(() => elements.toast.classList.add("hidden"), 3800);
}

function statusCopy() {
  if (state.phase === "starting") {
    return { label: "启动中", title: "正在启动", detail: "正在等待本地 Harness 服务就绪…" };
  }
  if (state.phase === "running") {
    return {
      label: "运行中",
      title: "Harness 运行中",
      detail: state.status?.port ? `本地端口 ${state.status.port} · 页面已就绪` : "本地页面已就绪",
    };
  }
  if (state.phase === "error") {
    return { label: "启动失败", title: "无法启动 Harness", detail: state.status?.error || "请查看运行日志。" };
  }
  return { label: "未启动", title: "准备启动", detail: "点击启动，在本地打开 DeepSeek Harness。" };
}

function renderStatus() {
  const copy = statusCopy();
  elements.titlebarStatusText.textContent = copy.label;
  elements.titlebarStatus.classList.toggle("running", state.phase === "running");
  elements.titlebarStatus.classList.toggle("starting", state.phase === "starting");
  elements.titlebarStatus.classList.toggle("error", state.phase === "error");
  elements.sideLiveIndicator.classList.toggle("running", state.phase === "running");
  elements.sideLiveIndicator.classList.toggle("starting", state.phase === "starting");
  elements.sideLiveIndicator.classList.toggle("error", state.phase === "error");
  elements.sideStatusTitle.textContent = copy.title;
  elements.sideStatusDetail.textContent = copy.detail;
  elements.portValue.textContent = state.status?.port ? `127.0.0.1:${state.status.port}` : "—";
  elements.emptyTitle.textContent = state.phase === "error" ? "Harness 启动失败" : "Harness 尚未启动";
  elements.emptyMessage.textContent =
    state.phase === "error"
      ? state.status?.error || "请打开运行日志检查 dsh 输出。"
      : "启动后，这里会承载本机的 Harness Web UI。";

  const running = state.phase === "running";
  elements.emptyState.classList.toggle("hidden", running);
  elements.frameContainer.classList.toggle("hidden", !running);
  if (running && state.status?.url && state.frameUrl !== state.status.url) {
    state.frameUrl = state.status.url;
    elements.frameLoading.textContent = "正在加载 Harness 页面…";
    elements.frameLoading.classList.remove("hidden");
    elements.frame.src = state.status.url;
  }
  setBusy(state.busy);
}

function renderRuntime() {
  if (!state.runtime) return;
  const label = state.runtime.available ? "已找到 dsh" : "未找到 dsh";
  elements.runtimeValue.textContent = label;
  elements.runtimeValue.classList.toggle("available", state.runtime.available);
  elements.runtimeValue.classList.toggle("missing", !state.runtime.available);
  elements.settingsRuntimePath.textContent = state.runtime.path || "未找到。请安装 dsh 或设置 DSH_BIN。";
}

function renderLogs() {
  elements.logCount.textContent = String(state.logs.length);
  elements.logList.replaceChildren();
  if (!state.logs.length) {
    const empty = document.createElement("div");
    empty.className = "log-empty";
    empty.textContent = "暂无日志输出";
    elements.logList.append(empty);
    return;
  }

  const fragment = document.createDocumentFragment();
  for (const log of state.logs) {
    const row = document.createElement("div");
    row.className = "log-row";
    const stream = document.createElement("span");
    stream.className = `log-stream ${log.stream || "desk"}`;
    stream.textContent = log.stream || "desk";
    const message = document.createElement("span");
    message.className = "log-message";
    message.textContent = log.message || "";
    row.append(stream, message);
    fragment.append(row);
  }
  elements.logList.append(fragment);
  elements.logList.scrollTop = elements.logList.scrollHeight;
}

function showDrawer(drawer) {
  elements.logsDrawer.classList.toggle("hidden", drawer !== elements.logsDrawer);
  elements.settingsDrawer.classList.toggle("hidden", drawer !== elements.settingsDrawer);
}

async function refreshRuntime() {
  try {
    state.runtime = await call("runtime_status");
    renderRuntime();
  } catch (error) {
    elements.runtimeValue.textContent = "无法检查";
    elements.settingsRuntimePath.textContent = errorMessage(error);
  }
}

async function refreshStatus() {
  try {
    const status = await call("harness_status");
    state.status = status;
    if (status.running) {
      state.phase = "running";
      if (status.url && state.frameUrl !== status.url) {
        state.frameUrl = status.url;
        elements.frameLoading.classList.remove("hidden");
        elements.frame.src = status.url;
      }
    } else if (!state.busy && state.phase !== "error") {
      state.phase = "idle";
      state.frameUrl = "";
      elements.frame.removeAttribute("src");
    }
    renderStatus();
  } catch (error) {
    if (!state.busy) setToast(errorMessage(error), true);
  }
}

async function loadLogs() {
  try {
    state.logs = await call("harness_logs");
    renderLogs();
  } catch {
    state.logs = [];
    renderLogs();
  }
}

async function startHarness() {
  if (state.busy || state.status?.running) return;
  state.phase = "starting";
  state.status = { running: false };
  setBusy(true);
  renderStatus();
  try {
    state.status = await call("start_harness");
    state.phase = state.status.running ? "running" : "idle";
    setToast("Harness 已启动");
  } catch (error) {
    state.status = { running: false, error: errorMessage(error) };
    state.phase = "error";
    setToast(state.status.error, true);
  } finally {
    setBusy(false);
    await loadLogs();
    renderStatus();
  }
}

async function stopHarness() {
  if (state.busy || !state.status?.running) return;
  setBusy(true);
  try {
    state.status = await call("stop_harness");
    state.phase = "idle";
    state.frameUrl = "";
    elements.frame.removeAttribute("src");
    setToast("Harness 已停止");
  } catch (error) {
    setToast(errorMessage(error), true);
  } finally {
    setBusy(false);
    await loadLogs();
    renderStatus();
  }
}

async function restartHarness() {
  if (state.busy || !state.status?.running) return startHarness();
  setBusy(true);
  state.phase = "starting";
  renderStatus();
  try {
    state.status = await call("restart_harness");
    state.phase = state.status.running ? "running" : "idle";
    state.frameUrl = "";
    setToast("Harness 已重启");
  } catch (error) {
    state.status = { running: false, error: errorMessage(error) };
    state.phase = "error";
    setToast(state.status.error, true);
  } finally {
    setBusy(false);
    await loadLogs();
    renderStatus();
  }
}

async function toggleDockIcon() {
  const visible = elements.dockIconToggle.checked;
  localStorage.setItem("showDockIcon", String(visible));
  try {
    await call("set_dock_visibility", { visible });
    setToast(visible ? "Dock 图标已开启" : "Dock 图标已关闭，可从菜单栏图标唤醒");
  } catch (error) {
    setToast(errorMessage(error), true);
  }
}

async function windowAction(command) {
  try {
    return await call(command);
  } catch (error) {
    setToast(errorMessage(error), true);
    return null;
  }
}

function bindEvents() {
  elements.startButton.addEventListener("click", startHarness);
  elements.emptyStartButton.addEventListener("click", startHarness);
  elements.stopButton.addEventListener("click", stopHarness);
  elements.restartButton.addEventListener("click", restartHarness);
  elements.refreshButton.addEventListener("click", async () => {
    await Promise.all([refreshStatus(), refreshRuntime(), loadLogs()]);
    setToast("状态已刷新");
  });

  elements.logsButton.addEventListener("click", async () => {
    showDrawer(elements.logsDrawer);
    await loadLogs();
  });
  elements.openLogsButton.addEventListener("click", async () => {
    showDrawer(elements.logsDrawer);
    await loadLogs();
  });
  elements.closeLogsButton.addEventListener("click", () => showDrawer(null));
  elements.clearLogsButton.addEventListener("click", async () => {
    await call("clear_harness_logs");
    state.logs = [];
    renderLogs();
  });

  elements.settingsButton.addEventListener("click", () => showDrawer(elements.settingsDrawer));
  elements.closeSettingsButton.addEventListener("click", () => showDrawer(null));
  elements.dockIconToggle.addEventListener("change", toggleDockIcon);
  elements.refreshRuntimeButton.addEventListener("click", refreshRuntime);

  elements.minimizeButton.addEventListener("click", () => windowAction("window_minimize"));
  elements.maximizeButton.addEventListener("click", () => windowAction("window_toggle_maximize"));
  elements.hideButton.addEventListener("click", () => windowAction("window_hide"));
  elements.frame.addEventListener("load", () => elements.frameLoading.classList.add("hidden"));
  elements.frame.addEventListener("error", () => {
    elements.frameLoading.textContent = "Harness 页面加载失败，请查看运行日志。";
    elements.frameLoading.classList.remove("hidden");
  });

  // The native Tauri drag region handles ordinary dragging and double-click
  // maximize. This fallback covers empty header pixels in older WebViews.
  document.querySelector(".titlebar").addEventListener("mousedown", (event) => {
    if (event.button === 0 && event.target === event.currentTarget) {
      windowAction("window_start_dragging");
    }
  });
}

async function listenForOutput() {
  if (!listen) return;
  await listen("harness-output", (event) => {
    state.logs.push(event.payload);
    if (state.logs.length > 500) state.logs.shift();
    renderLogs();
  });
}

async function initialize() {
  elements.dockIconToggle.checked = localStorage.getItem("showDockIcon") !== "false";
  bindEvents();
  renderStatus();
  renderLogs();
  await listenForOutput();
  await Promise.all([refreshStatus(), refreshRuntime(), loadLogs()]);
  window.setInterval(refreshStatus, 2500);
}

window.addEventListener("DOMContentLoaded", initialize);
