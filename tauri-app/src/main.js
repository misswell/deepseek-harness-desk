const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke;
const listen = tauri?.event?.listen;

const elements = {
  frameContainer: document.querySelector("#frame-container"),
  frameLoading: document.querySelector("#frame-loading"),
  frame: document.querySelector("#harness-frame"),
  startupView: document.querySelector("#startup-view"),
  startupSpinner: document.querySelector("#startup-spinner"),
  startupTitle: document.querySelector("#startup-title"),
  startupMessage: document.querySelector("#startup-message"),
  runtimeStatus: document.querySelector("#runtime-status"),
  harnessStatus: document.querySelector("#harness-status"),
  portStatus: document.querySelector("#port-status"),
  startButton: document.querySelector("#start-button"),
  startupLogsButton: document.querySelector("#startup-logs-button"),
  errorView: document.querySelector("#error-view"),
  errorMessage: document.querySelector("#error-message"),
  retryButton: document.querySelector("#retry-button"),
  errorLogsButton: document.querySelector("#error-logs-button"),
  deskStatus: document.querySelector("#desk-status"),
  deskStatusText: document.querySelector("#desk-status-text"),
  menuButton: document.querySelector("#menu-button"),
  deskMenu: document.querySelector("#desk-menu"),
  restartButton: document.querySelector("#restart-button"),
  stopButton: document.querySelector("#stop-button"),
  logsButton: document.querySelector("#logs-button"),
  settingsButton: document.querySelector("#settings-button"),
  refreshButton: document.querySelector("#refresh-button"),
  logsPanel: document.querySelector("#logs-panel"),
  logList: document.querySelector("#log-list"),
  clearLogsButton: document.querySelector("#clear-logs-button"),
  closeLogsButton: document.querySelector("#close-logs-button"),
  settingsPanel: document.querySelector("#settings-panel"),
  closeSettingsButton: document.querySelector("#close-settings-button"),
  dockIconToggle: document.querySelector("#dock-icon-toggle"),
  settingsRuntimePath: document.querySelector("#settings-runtime-path"),
  runtimeInstallStatus: document.querySelector("#runtime-install-status"),
  runtimeInstallButton: document.querySelector("#runtime-install-button"),
  refreshRuntimeButton: document.querySelector("#refresh-runtime-button"),
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
  if (!invoke) throw new Error("当前页面不是 Tauri 应用窗口");
  return invoke(command, args);
}

function setToast(message, isError = false) {
  if (!message) return;
  elements.toast.textContent = message;
  elements.toast.classList.toggle("error", isError);
  elements.toast.classList.remove("hidden");
  clearTimeout(state.toastTimer);
  state.toastTimer = setTimeout(() => elements.toast.classList.add("hidden"), 3800);
}

function setBusy(busy) {
  state.busy = busy;
  const running = state.status?.running === true;
  elements.startButton.disabled = busy || running;
  elements.retryButton.disabled = busy || running;
  elements.restartButton.disabled = busy || !running;
  elements.stopButton.disabled = busy || !running;
  elements.runtimeInstallButton.disabled = busy || state.runtime?.installing === true;
  elements.startButton.textContent = busy ? "启动中…" : running ? "已启动" : "启动 Harness";
  elements.retryButton.textContent = busy ? "启动中…" : "重新启动";
}

function phaseCopy() {
  if (state.phase === "starting") {
    return {
      label: "启动中",
      title: "正在启动 DeepSeek Harness…",
      message: "正在准备本机运行环境，请稍候。",
    };
  }
  if (state.phase === "running") {
    return { label: "运行中", title: "Harness 已启动", message: "" };
  }
  if (state.phase === "error") {
    return { label: "启动失败", title: "DeepSeek Harness 无法启动", message: "" };
  }
  return { label: "已停止", title: "DeepSeek Harness 已停止", message: "点击启动按钮重新打开 Harness。" };
}

function renderRuntime() {
  const runtime = state.runtime;
  if (!runtime) return;

  const label = runtime.installing
    ? "安装中…"
    : runtime.available
      ? "内置 Node + dsh"
      : "待安装";
  elements.runtimeStatus.textContent = label;
  elements.runtimeStatus.classList.toggle("available", runtime.available && !runtime.installing);
  elements.runtimeStatus.classList.toggle("missing", !runtime.available && !runtime.installing);
  elements.settingsRuntimePath.textContent =
    runtime.path || runtime.message || "首次启动会自动安装 Node.js 和 dsh。";
  elements.runtimeInstallStatus.textContent =
    runtime.message || (runtime.available ? "运行时已就绪。" : "点击安装，应用会自动准备运行环境。");
  elements.runtimeInstallButton.classList.toggle("hidden", runtime.available && !runtime.installing);
  elements.runtimeInstallButton.textContent = runtime.installing ? "安装中…" : "安装运行时";
  elements.runtimeInstallStatus.classList.toggle("active", runtime.installing);
  setBusy(state.busy);
}

function renderStatus() {
  const copy = phaseCopy();
  const running = state.phase === "running" && state.status?.running === true;
  const failed = state.phase === "error";

  elements.deskStatusText.textContent = copy.label;
  elements.deskStatus.classList.toggle("running", running);
  elements.deskStatus.classList.toggle("starting", state.phase === "starting");
  elements.deskStatus.classList.toggle("error", failed);
  elements.startupView.classList.toggle("hidden", running || failed);
  elements.errorView.classList.toggle("hidden", !failed);
  elements.startupSpinner.classList.toggle("hidden", state.phase !== "starting");
  elements.startupTitle.textContent = copy.title;
  if (state.phase !== "starting" || !state.runtime?.installing) {
    elements.startupMessage.textContent = copy.message;
  }
  elements.harnessStatus.textContent = running ? "运行中" : state.phase === "error" ? "启动失败" : state.phase === "starting" ? "启动中…" : "已停止";
  elements.harnessStatus.classList.toggle("available", running);
  elements.harnessStatus.classList.toggle("missing", failed);
  elements.portStatus.textContent = state.status?.port ? String(state.status.port) : "—";
  elements.errorMessage.textContent = state.status?.error || "发生未知错误，请查看运行日志。";
  elements.frameContainer.classList.toggle("hidden", !running);

  if (running && state.status?.url && state.frameUrl !== state.status.url) {
    state.frameUrl = state.status.url;
    elements.frameLoading.textContent = "正在加载 Harness 页面…";
    elements.frameLoading.classList.remove("hidden");
    elements.frame.src = state.status.url;
  }
  setBusy(state.busy);
}

function renderLogs() {
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

function closePanels() {
  elements.logsPanel.classList.add("hidden");
  elements.settingsPanel.classList.add("hidden");
}

function showPanel(panel) {
  elements.deskMenu.classList.add("hidden");
  elements.logsPanel.classList.toggle("hidden", panel !== elements.logsPanel);
  elements.settingsPanel.classList.toggle("hidden", panel !== elements.settingsPanel);
}

function toggleMenu() {
  elements.deskMenu.classList.toggle("hidden");
}

async function refreshRuntime() {
  try {
    state.runtime = await call("runtime_status");
    renderRuntime();
  } catch (error) {
    elements.runtimeStatus.textContent = "无法检查";
    elements.settingsRuntimePath.textContent = errorMessage(error);
  }
}

async function refreshStatus() {
  try {
    const status = await call("harness_status");
    state.status = status;
    if (status.running) {
      state.phase = "running";
    } else if (!state.busy && state.phase === "running") {
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
  } catch {
    state.logs = [];
  }
  renderLogs();
}

async function startHarness() {
  if (state.busy || state.status?.running) return;
  state.phase = "starting";
  state.status = { running: false };
  state.busy = true;
  renderStatus();
  try {
    if (!state.runtime?.available) {
      state.runtime = await call("install_runtime");
      renderRuntime();
    }
    state.status = await call("start_harness");
    if (!state.status.running) throw new Error(state.status.error || "Harness 未能启动。");
    state.phase = "running";
    setToast("Harness 已启动");
  } catch (error) {
    state.status = { running: false, error: errorMessage(error) };
    state.phase = "error";
    setToast(state.status.error, true);
  } finally {
    state.busy = false;
    await loadLogs();
    renderRuntime();
    renderStatus();
  }
}

async function restartHarness() {
  if (state.busy) return;
  state.phase = "starting";
  state.busy = true;
  renderStatus();
  try {
    state.status = await call("restart_harness");
    if (!state.status.running) throw new Error(state.status.error || "Harness 未能启动。");
    state.phase = "running";
    setToast("Harness 已重启");
  } catch (error) {
    state.status = { running: false, error: errorMessage(error) };
    state.phase = "error";
    setToast(state.status.error, true);
  } finally {
    state.busy = false;
    await loadLogs();
    renderStatus();
  }
}

async function stopHarness() {
  if (state.busy || !state.status?.running) return;
  state.busy = true;
  try {
    state.status = await call("stop_harness");
    state.phase = "idle";
    state.frameUrl = "";
    elements.frame.removeAttribute("src");
    setToast("Harness 已停止");
  } catch (error) {
    setToast(errorMessage(error), true);
  } finally {
    state.busy = false;
    await loadLogs();
    renderStatus();
  }
}

async function installRuntime() {
  if (state.busy) return;
  state.busy = true;
  if (state.runtime) state.runtime.installing = true;
  renderRuntime();
  try {
    state.runtime = await call("install_runtime");
    setToast("运行时安装完成");
  } catch (error) {
    if (state.runtime) {
      state.runtime.installing = false;
      state.runtime.message = errorMessage(error);
    }
    setToast(errorMessage(error), true);
  } finally {
    state.busy = false;
    await refreshRuntime();
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

function openLogs() {
  showPanel(elements.logsPanel);
  loadLogs();
}

function bindEvents() {
  elements.startButton.addEventListener("click", startHarness);
  elements.retryButton.addEventListener("click", startHarness);
  elements.startupLogsButton.addEventListener("click", openLogs);
  elements.errorLogsButton.addEventListener("click", openLogs);
  elements.menuButton.addEventListener("click", toggleMenu);
  elements.restartButton.addEventListener("click", restartHarness);
  elements.stopButton.addEventListener("click", stopHarness);
  elements.logsButton.addEventListener("click", openLogs);
  elements.settingsButton.addEventListener("click", () => showPanel(elements.settingsPanel));
  elements.refreshButton.addEventListener("click", async () => {
    elements.deskMenu.classList.add("hidden");
    await Promise.all([refreshStatus(), refreshRuntime(), loadLogs()]);
    setToast("状态已刷新");
  });
  elements.clearLogsButton.addEventListener("click", async () => {
    await call("clear_harness_logs");
    state.logs = [];
    renderLogs();
  });
  elements.closeLogsButton.addEventListener("click", closePanels);
  elements.closeSettingsButton.addEventListener("click", closePanels);
  elements.dockIconToggle.addEventListener("change", toggleDockIcon);
  elements.runtimeInstallButton.addEventListener("click", installRuntime);
  elements.refreshRuntimeButton.addEventListener("click", refreshRuntime);
  elements.frame.addEventListener("load", () => elements.frameLoading.classList.add("hidden"));
  elements.frame.addEventListener("error", () => {
    elements.frameLoading.textContent = "Harness 页面加载失败，请查看运行日志。";
    elements.frameLoading.classList.remove("hidden");
  });

  document.addEventListener("click", (event) => {
    if (!elements.deskMenu.contains(event.target) && event.target !== elements.menuButton) {
      elements.deskMenu.classList.add("hidden");
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
  await listen("runtime-progress", (event) => {
    const progress = event.payload || {};
    if (state.runtime) {
      state.runtime.installing = !progress.done;
      state.runtime.message = progress.message || "";
    }
    if (state.phase === "starting" && progress.message) {
      elements.startupMessage.textContent = progress.message;
    }
    renderRuntime();
  });
}

async function initialize() {
  elements.dockIconToggle.checked = localStorage.getItem("showDockIcon") !== "false";
  bindEvents();
  renderStatus();
  renderLogs();
  await listenForOutput();
  await Promise.all([refreshStatus(), refreshRuntime(), loadLogs()]);

  if (state.status?.running) {
    state.phase = "running";
    renderStatus();
  } else {
    await startHarness();
  }

  if (!elements.dockIconToggle.checked) {
    await call("set_dock_visibility", { visible: false }).catch((error) => setToast(errorMessage(error), true));
  }
  window.setInterval(refreshStatus, 2500);
}

window.addEventListener("DOMContentLoaded", initialize);
