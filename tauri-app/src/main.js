import {
  DEFAULT_ZOOM,
  ZOOM_STEP,
  clampZoom,
  zoomFromShortcut,
} from "./zoom.js";

const tauri = window.__TAURI__;
const invoke = tauri?.core?.invoke;
const listen = tauri?.event?.listen;
const currentWebview = tauri?.webviewWindow?.getCurrentWebviewWindow?.() || null;
const ZOOM_STORAGE_KEY = "uiZoom";

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
  logsPanel: document.querySelector("#logs-panel"),
  logList: document.querySelector("#log-list"),
  clearLogsButton: document.querySelector("#clear-logs-button"),
  closeLogsButton: document.querySelector("#close-logs-button"),
  settingsPanel: document.querySelector("#settings-panel"),
  closeSettingsButton: document.querySelector("#close-settings-button"),
  appUpdateBanner: document.querySelector("#app-update-banner"),
  appUpdateBannerText: document.querySelector("#app-update-banner-text"),
  appUpdateBannerInstall: document.querySelector("#app-update-banner-install"),
  appUpdateBannerIgnore: document.querySelector("#app-update-banner-ignore"),
  appUpdateBannerDismiss: document.querySelector("#app-update-banner-dismiss"),
  settingsTabs: [...document.querySelectorAll("[data-settings-tab]")],
  settingsPages: [...document.querySelectorAll("[data-settings-page]")],
  launchAtLoginToggle: document.querySelector("#launch-at-login-toggle"),
  restoreLastWindowToggle: document.querySelector("#restore-last-window-toggle"),
  dockIconToggle: document.querySelector("#dock-icon-toggle"),
  appCurrentVersion: document.querySelector("#app-current-version"),
  appLatestVersion: document.querySelector("#app-latest-version"),
  appUpdateStatus: document.querySelector("#app-update-status"),
  checkAppUpdateButton: document.querySelector("#check-app-update-button"),
  installAppUpdateButton: document.querySelector("#install-app-update-button"),
  openReleasesPageButton: document.querySelector("#open-releases-page-button"),
  openReleasePageButton: document.querySelector("#open-release-page-button"),
  autoCheckAppToggle: document.querySelector("#auto-check-app-toggle"),
  autoCheckInterval: document.querySelector("#auto-check-interval"),
  dshCurrentVersion: document.querySelector("#dsh-current-version"),
  dshLatestVersion: document.querySelector("#dsh-latest-version"),
  dshUpdateStatus: document.querySelector("#dsh-update-status"),
  checkDshUpdateButton: document.querySelector("#check-dsh-update-button"),
  installDshUpdateButton: document.querySelector("#install-dsh-update-button"),
  autoCheckDshToggle: document.querySelector("#auto-check-dsh-toggle"),
  autoInstallDshToggle: document.querySelector("#auto-install-dsh-toggle"),
  updateProgressCard: document.querySelector("#update-progress-card"),
  updateProgressTitle: document.querySelector("#update-progress-title"),
  updateProgressMessage: document.querySelector("#update-progress-message"),
  updateProgressPercent: document.querySelector("#update-progress-percent"),
  updateProgressBar: document.querySelector("#update-progress-bar"),
  settingsRuntimePath: document.querySelector("#settings-runtime-path"),
  runtimeInstallStatus: document.querySelector("#runtime-install-status"),
  runtimeInstallButton: document.querySelector("#runtime-install-button"),
  refreshRuntimeButton: document.querySelector("#refresh-runtime-button"),
  settingsHarnessStatus: document.querySelector("#settings-harness-status"),
  settingsHarnessVersion: document.querySelector("#settings-harness-version"),
  settingsHarnessPid: document.querySelector("#settings-harness-pid"),
  settingsHarnessPort: document.querySelector("#settings-harness-port"),
  settingsHarnessPath: document.querySelector("#settings-harness-path"),
  settingsStartHarnessButton: document.querySelector("#settings-start-harness-button"),
  settingsRestartHarnessButton: document.querySelector("#settings-restart-harness-button"),
  settingsStopHarnessButton: document.querySelector("#settings-stop-harness-button"),
  settingsOpenLogsButton: document.querySelector("#settings-open-logs-button"),
  settingsRefreshHarnessButton: document.querySelector("#settings-refresh-harness-button"),
  settingsLogDirectory: document.querySelector("#settings-log-directory"),
  openRuntimeDirectoryButton: document.querySelector("#open-runtime-directory-button"),
  openLogsDirectoryButton: document.querySelector("#open-logs-directory-button"),
  aboutVersion: document.querySelector("#about-version"),
  memorySaverToggle: document.querySelector("#memory-saver-toggle"),
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
  updateProgressTimer: null,
  updateKind: null,
  appUpdate: null,
  dshUpdate: null,
  settingsTab: localStorage.getItem("settingsTab") || "general",
  automaticUpdateTimer: null,
  updateChecking: false,
  busyOperation: null,
  zoom: DEFAULT_ZOOM,
  memorySaver: localStorage.getItem("memorySaver") !== "false",
  statusInterval: null,
  frameUnloaded: false,
  windowHidden: false,
};

const UPDATE_INTERVALS = {
  hourly: 60 * 60 * 1000,
  daily: 24 * 60 * 60 * 1000,
  weekly: 7 * 24 * 60 * 60 * 1000,
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

function storedZoom() {
  const value = Number.parseFloat(localStorage.getItem(ZOOM_STORAGE_KEY));
  return Number.isFinite(value) ? clampZoom(value) : DEFAULT_ZOOM;
}

async function applyZoom(zoom, { persist = true, announce = false } = {}) {
  const nextZoom = clampZoom(zoom);
  state.zoom = nextZoom;
  if (persist) localStorage.setItem(ZOOM_STORAGE_KEY, String(nextZoom));

  let appliedByWebview = false;
  if (currentWebview?.setZoom) {
    try {
      await currentWebview.setZoom(nextZoom);
      appliedByWebview = true;
    } catch (error) {
      console.warn("无法设置原生 WebView 缩放，改用页面缩放", error);
    }
  }
  if (!appliedByWebview) {
    document.documentElement.style.zoom = String(nextZoom);
  }

  if (announce) setToast(`界面缩放：${Math.round(nextZoom * 100)}%`);
  return nextZoom;
}

function handleZoomShortcut(event) {
  const nextZoom = zoomFromShortcut(event, state.zoom);
  if (nextZoom === null) return;

  event.preventDefault();
  event.stopPropagation();
  void applyZoom(nextZoom, { announce: true }).catch((error) => {
    setToast(errorMessage(error), true);
  });
}

function bindFrameZoomShortcuts() {
  try {
    const frameWindow = elements.frame.contentWindow;
    frameWindow?.addEventListener("keydown", handleZoomShortcut, true);
  } catch {
    // Cross-origin Harness frames do not expose their event target; the
    // native menu accelerators still handle shortcuts while the frame is focused.
  }
}

function showUpdatesPanel() {
  showPanel(elements.settingsPanel);
  state.settingsTab = "updates";
  renderSettingsTab();
}

function renderUpdateProgress({ title, message, fraction, done = false, error = null }) {
  if (state.updateProgressTimer) {
    clearTimeout(state.updateProgressTimer);
    state.updateProgressTimer = null;
  }

  const current = Number(elements.updateProgressBar.value) || 0;
  const value = typeof fraction === "number"
    ? Math.min(1, Math.max(0, fraction))
    : current;
  const percent = Math.round(value * 100);
  elements.updateProgressCard.classList.remove("hidden");
  elements.updateProgressTitle.textContent = error ? "更新失败" : title || "正在更新…";
  elements.updateProgressMessage.textContent = error || message || "准备中…";
  elements.updateProgressPercent.textContent = error ? "失败" : `${percent}%`;
  elements.updateProgressBar.value = value;
  elements.updateProgressBar.setAttribute("aria-valuenow", String(percent));

  if (done && !error) {
    state.updateProgressTimer = window.setTimeout(() => {
      elements.updateProgressCard.classList.add("hidden");
      state.updateProgressTimer = null;
    }, 2400);
  }
}

function setBusy(busy) {
  state.busy = busy;
  if (!busy) state.busyOperation = null;
  const running = state.status?.running === true;
  const busyLabel = state.busyOperation === "dsh-update"
    ? "更新中…"
    : state.busyOperation === "runtime-install"
      ? "安装中…"
      : state.busyOperation === "stop"
        ? "停止中…"
        : "启动中…";
  elements.startButton.disabled = busy || running;
  elements.retryButton.disabled = busy || running;
  elements.runtimeInstallButton.disabled = busy || state.runtime?.installing === true;
  elements.settingsStartHarnessButton.disabled = busy || running;
  elements.settingsRestartHarnessButton.disabled = busy || !running;
  elements.settingsStopHarnessButton.disabled = busy || !running;
  elements.checkAppUpdateButton.disabled = busy;
  elements.checkDshUpdateButton.disabled = busy || state.runtime?.available !== true;
  elements.installDshUpdateButton.disabled = busy;
  elements.startButton.textContent = busy ? busyLabel : running ? "已启动" : "启动 Harness";
  elements.retryButton.textContent = busy ? busyLabel : "重新启动";
}

function phaseCopy() {
  if (state.busyOperation === "dsh-update") {
    return {
      label: "更新中",
      title: "正在更新内置 dsh…",
      message: "更新期间会暂时停止 Harness，完成后自动恢复。",
    };
  }
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
    runtime.runtime_root || runtime.path || runtime.message || "首次启动会自动安装 Node.js 和 dsh。";
  elements.settingsLogDirectory.textContent = runtime.logs_directory || "—";
  elements.runtimeInstallStatus.textContent =
    runtime.message || (runtime.available ? "运行时已就绪。" : "点击安装，应用会自动准备运行环境。");
  elements.runtimeInstallButton.classList.toggle("hidden", runtime.available && !runtime.installing);
  elements.runtimeInstallButton.textContent = runtime.installing ? "安装中…" : "安装运行时";
  elements.runtimeInstallStatus.classList.toggle("active", runtime.installing);
  elements.dshCurrentVersion.textContent = runtime.version || (runtime.available ? "系统 PATH" : "未安装");
  elements.settingsHarnessVersion.textContent = runtime.version || "—";
  setBusy(state.busy);
}

function renderSettingsTab() {
  const tab = state.settingsTab;
  for (const button of elements.settingsTabs) {
    const active = button.dataset.settingsTab === tab;
    button.classList.toggle("active", active);
    button.setAttribute("aria-selected", String(active));
  }
  for (const page of elements.settingsPages) {
    page.classList.toggle("hidden", page.dataset.settingsPage !== tab);
  }
  localStorage.setItem("settingsTab", tab);
}

function renderAppUpdate() {
  const update = state.appUpdate;
  if (!update) {
    elements.appCurrentVersion.textContent = "检查中…";
    elements.appLatestVersion.textContent = "未检查";
    elements.appUpdateStatus.textContent = "未检查";
    elements.aboutVersion.textContent = "版本检查中…";
    return;
  }
  elements.appCurrentVersion.textContent = update.current_version || "—";
  elements.appLatestVersion.textContent = update.latest_version || "未检查";
  elements.appUpdateStatus.textContent = update.status || "未检查";
  elements.aboutVersion.textContent = `版本 ${update.current_version || "—"}`;
  const available = update.available === true;
  const ignored = localStorage.getItem("ignoredAppUpdateVersion") === update.latest_version;
  elements.installAppUpdateButton.classList.toggle("hidden", !available);
  elements.openReleasePageButton.classList.toggle("hidden", !update.release_url);
  elements.appUpdateBanner.classList.toggle("hidden", !available || ignored);
  if (available && !ignored) {
    elements.appUpdateBannerText.textContent = `发现新版本 ${update.latest_version}`;
  }
}

function renderDshUpdate() {
  const update = state.dshUpdate;
  if (!update) return;
  elements.dshCurrentVersion.textContent = update.current_version || (state.runtime?.version || "未安装");
  elements.dshLatestVersion.textContent = update.latest_version || "未检查";
  elements.dshUpdateStatus.textContent = update.status || "未检查";
  elements.installDshUpdateButton.classList.toggle("hidden", update.available !== true);
  elements.installDshUpdateButton.disabled = update.available !== true || state.busy;
}

function renderStatus() {
  const copy = phaseCopy();
  const running = state.phase === "running" && state.status?.running === true;
  const failed = state.phase === "error";
  const updatingDsh = state.busyOperation === "dsh-update";

  elements.startupView.classList.toggle("hidden", running || failed);
  elements.errorView.classList.toggle("hidden", !failed);
  elements.startupSpinner.classList.toggle("hidden", state.phase !== "starting" && !updatingDsh);
  elements.startupTitle.textContent = copy.title;
  if (updatingDsh) {
    elements.startupMessage.textContent = copy.message;
  } else if (state.phase !== "starting" || !state.runtime?.installing) {
    elements.startupMessage.textContent = copy.message;
  }
  elements.harnessStatus.textContent = updatingDsh
    ? "更新中…"
    : running
      ? "运行中"
      : state.phase === "error"
        ? "启动失败"
        : state.phase === "starting"
          ? "启动中…"
          : "已停止";
  elements.harnessStatus.classList.toggle("available", running);
  elements.harnessStatus.classList.toggle("missing", failed);
  elements.portStatus.textContent = state.status?.port ? String(state.status.port) : "—";
  elements.settingsHarnessStatus.textContent = updatingDsh
    ? "更新中…"
    : running
      ? "运行中"
      : failed
        ? "启动失败"
        : state.phase === "starting"
          ? "启动中…"
          : "已停止";
  elements.settingsHarnessPid.textContent = state.status?.pid ? String(state.status.pid) : "—";
  elements.settingsHarnessPort.textContent = state.status?.port ? String(state.status.port) : "—";
  elements.settingsHarnessPath.textContent = state.status?.dsh_path || "—";
  elements.errorMessage.textContent = state.status?.error || "发生未知错误，请查看运行日志。";
  elements.frameContainer.classList.toggle("hidden", !running);

  if (running && state.status?.url && state.frameUrl !== state.status.url) {
    if (state.windowHidden) {
      // Keep the heavy Harness UI unloaded while the window is hidden so the
      // WebView can release its rendering memory; restore on the next show.
      state.frameUnloaded = true;
      return;
    }
    state.frameUrl = state.status.url;
    elements.frameLoading.textContent = "正在加载 Harness 页面…";
    elements.frameLoading.classList.remove("hidden");
    elements.frame.src = state.status.url;
  }
  renderAppUpdate();
  renderDshUpdate();
  setBusy(state.busy);
}

function renderLogs() {
  if (state.windowHidden) return;
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
  elements.logsPanel.classList.toggle("hidden", panel !== elements.logsPanel);
  elements.settingsPanel.classList.toggle("hidden", panel !== elements.settingsPanel);
  if (panel === elements.settingsPanel) renderSettingsTab();
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

async function checkAppUpdate(interactive = true) {
  if (state.updateChecking) return state.appUpdate;
  state.updateChecking = true;
  elements.checkAppUpdateButton.disabled = true;
  elements.appUpdateStatus.textContent = "正在检查更新…";
  try {
    state.appUpdate = await call("check_app_update");
    renderAppUpdate();
    if (state.appUpdate.available) {
      if (interactive) {
        showUpdatesPanel();
        setToast(`发现 App 新版本 ${state.appUpdate.latest_version}`);
      }
    } else if (interactive) {
      setToast(state.appUpdate.status || "已是最新版本");
    }
    return state.appUpdate;
  } catch (error) {
    elements.appUpdateStatus.textContent = errorMessage(error);
    if (interactive) setToast(errorMessage(error), true);
    return null;
  } finally {
    state.updateChecking = false;
    setBusy(state.busy);
  }
}

async function installAppUpdate() {
  if (state.busy || state.updateChecking) return;
  showUpdatesPanel();
  state.updateKind = "app";
  renderUpdateProgress({
    title: "正在更新 App…",
    message: "准备下载更新包…",
    fraction: 0.01,
  });
  state.updateChecking = true;
  elements.installAppUpdateButton.disabled = true;
  elements.appUpdateBannerInstall.disabled = true;
  try {
    state.appUpdate = await call("install_app_update");
    renderAppUpdate();
    if (state.appUpdate?.status) setToast(state.appUpdate.status);
  } catch (error) {
    const message = errorMessage(error);
    renderUpdateProgress({
      title: "App 更新失败",
      message,
      error: message,
      done: true,
    });
    setToast(message, true);
  } finally {
    state.updateChecking = false;
    elements.appUpdateBannerInstall.disabled = false;
    setBusy(state.busy);
  }
}

async function checkDshUpdate(automaticallyInstall = false, interactive = true) {
  if (state.updateChecking) return state.dshUpdate;
  state.updateChecking = true;
  elements.checkDshUpdateButton.disabled = true;
  elements.dshUpdateStatus.textContent = "正在检查更新…";
  try {
    state.dshUpdate = await call("check_dsh_update");
    renderDshUpdate();
    if (state.dshUpdate.available && automaticallyInstall && elements.autoInstallDshToggle.checked) {
      await installDshUpdate(true);
    } else if (interactive) {
      showUpdatesPanel();
      setToast(state.dshUpdate.status || "内置 dsh 已是最新版本");
    }
    return state.dshUpdate;
  } catch (error) {
    elements.dshUpdateStatus.textContent = errorMessage(error);
    if (interactive) setToast(errorMessage(error), true);
    return null;
  } finally {
    state.updateChecking = false;
    setBusy(state.busy);
  }
}

async function installDshUpdate(automatically = false) {
  const version = state.dshUpdate?.latest_version;
  if (state.busy || !version) return;
  if (!automatically) showUpdatesPanel();
  state.updateKind = "dsh";
  state.busyOperation = "dsh-update";
  renderUpdateProgress({
    title: "正在更新内置 dsh…",
    message: `准备安装 ${version}…`,
    fraction: 0.02,
  });
  state.busy = true;
  renderStatus();
  try {
    state.runtime = await call("install_dsh_update", { version });
    state.dshUpdate = {
      ...(state.dshUpdate || {}),
      current_version: state.runtime.version,
      available: false,
      status: `内置 dsh 更新完成：${state.runtime.version || version}`,
    };
    setToast(automatically ? `内置 dsh 已自动更新到 ${version}` : `内置 dsh 已更新到 ${version}`);
  } catch (error) {
    const message = errorMessage(error);
    renderUpdateProgress({
      title: "内置 dsh 更新失败",
      message,
      error: message,
      done: true,
    });
    setToast(message, true);
  } finally {
    state.busy = false;
    state.busyOperation = null;
    await refreshStatus();
    await loadLogs();
    renderRuntime();
    renderDshUpdate();
    renderStatus();
  }
}

function scheduleAutomaticUpdateChecks() {
  if (state.automaticUpdateTimer) {
    clearTimeout(state.automaticUpdateTimer);
    clearInterval(state.automaticUpdateTimer);
    state.automaticUpdateTimer = null;
  }
  const appEnabled = elements.autoCheckAppToggle.checked;
  const dshEnabled = elements.autoCheckDshToggle.checked;
  if (!appEnabled && !dshEnabled) return;
  const interval = UPDATE_INTERVALS[elements.autoCheckInterval.value] || UPDATE_INTERVALS.hourly;
  state.automaticUpdateTimer = window.setTimeout(async () => {
    if (appEnabled) await checkAppUpdate(false);
    if (dshEnabled) await checkDshUpdate(true, false);
    state.automaticUpdateTimer = window.setInterval(async () => {
      if (elements.autoCheckAppToggle.checked) await checkAppUpdate(false);
      if (elements.autoCheckDshToggle.checked) await checkDshUpdate(true, false);
    }, interval);
  }, 3500);
}

function unloadHarnessFrame() {
  if (!state.status?.running || state.frameUnloaded) return;
  state.frameUnloaded = true;
  state.frameUrl = "";
  elements.frame.removeAttribute("src");
  elements.frameLoading.classList.add("hidden");
}

function restoreHarnessFrame() {
  if (!state.frameUnloaded) return;
  state.frameUnloaded = false;
  void refreshStatus();
}

function pauseStatusPolling() {
  if (state.statusInterval) {
    clearInterval(state.statusInterval);
    state.statusInterval = null;
  }
}

function resumeStatusPolling() {
  if (state.statusInterval) return;
  state.statusInterval = window.setInterval(refreshStatus, 2500);
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
  state.busyOperation = "start";
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
  state.busyOperation = "start";
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
  state.busyOperation = "stop";
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
  state.busyOperation = "runtime-install";
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

async function toggleLaunchAtLogin() {
  const enabled = elements.launchAtLoginToggle.checked;
  localStorage.setItem("launchAtLogin", String(enabled));
  try {
    await call("set_launch_at_login", { enabled });
    setToast(enabled ? "已开启登录时启动" : "已关闭登录时启动");
  } catch (error) {
    elements.launchAtLoginToggle.checked = !enabled;
    setToast(errorMessage(error), true);
  }
}

function toggleRestoreLastWindow() {
  localStorage.setItem("restoreLastWindow", String(elements.restoreLastWindowToggle.checked));
}

async function openRuntimeDirectory() {
  try {
    await call("open_runtime_directory");
  } catch (error) {
    setToast(errorMessage(error), true);
  }
}

async function openLogsDirectory() {
  try {
    await call("open_logs_directory");
  } catch (error) {
    setToast(errorMessage(error), true);
  }
}

function selectSettingsTab(tab) {
  state.settingsTab = tab;
  renderSettingsTab();
}

function openLogs() {
  showPanel(elements.logsPanel);
  loadLogs();
}

function bindEvents() {
  window.addEventListener("keydown", handleZoomShortcut, true);
  elements.startButton.addEventListener("click", startHarness);
  elements.retryButton.addEventListener("click", startHarness);
  elements.startupLogsButton.addEventListener("click", openLogs);
  elements.errorLogsButton.addEventListener("click", openLogs);
  elements.clearLogsButton.addEventListener("click", async () => {
    await call("clear_harness_logs");
    state.logs = [];
    renderLogs();
  });
  elements.closeLogsButton.addEventListener("click", closePanels);
  elements.closeSettingsButton.addEventListener("click", closePanels);
  for (const tab of elements.settingsTabs) {
    tab.addEventListener("click", () => selectSettingsTab(tab.dataset.settingsTab));
  }
  elements.launchAtLoginToggle.addEventListener("change", toggleLaunchAtLogin);
  elements.restoreLastWindowToggle.addEventListener("change", toggleRestoreLastWindow);
  elements.dockIconToggle.addEventListener("change", toggleDockIcon);
  elements.checkAppUpdateButton.addEventListener("click", () => checkAppUpdate(true));
  elements.installAppUpdateButton.addEventListener("click", () => installAppUpdate());
  elements.appUpdateBannerInstall.addEventListener("click", () => installAppUpdate());
  elements.appUpdateBannerIgnore.addEventListener("click", () => {
    const version = state.appUpdate?.latest_version;
    if (version) localStorage.setItem("ignoredAppUpdateVersion", version);
    elements.appUpdateBanner.classList.add("hidden");
  });
  elements.appUpdateBannerDismiss.addEventListener("click", () => {
    elements.appUpdateBanner.classList.add("hidden");
  });
  elements.openReleasePageButton.addEventListener("click", async () => {
    if (!state.appUpdate?.release_url) return;
    await call("open_release_page", { url: state.appUpdate.release_url }).catch((error) => setToast(errorMessage(error), true));
  });
  elements.openReleasesPageButton.addEventListener("click", async () => {
    await call("open_releases_page").catch((error) => setToast(errorMessage(error), true));
  });
  elements.checkDshUpdateButton.addEventListener("click", () => checkDshUpdate(false, true));
  elements.installDshUpdateButton.addEventListener("click", () => installDshUpdate(false));
  elements.autoCheckAppToggle.addEventListener("change", () => {
    localStorage.setItem("autoCheckForUpdates", String(elements.autoCheckAppToggle.checked));
    scheduleAutomaticUpdateChecks();
  });
  elements.autoCheckDshToggle.addEventListener("change", () => {
    localStorage.setItem("autoCheckHarnessUpdates", String(elements.autoCheckDshToggle.checked));
    scheduleAutomaticUpdateChecks();
  });
  elements.autoInstallDshToggle.addEventListener("change", () => {
    localStorage.setItem("autoInstallHarnessUpdates", String(elements.autoInstallDshToggle.checked));
  });
  elements.autoCheckInterval.addEventListener("change", () => {
    localStorage.setItem("autoCheckInterval", elements.autoCheckInterval.value);
    scheduleAutomaticUpdateChecks();
  });
  elements.memorySaverToggle.addEventListener("change", () => {
    const enabled = elements.memorySaverToggle.checked;
    state.memorySaver = enabled;
    localStorage.setItem("memorySaver", String(enabled));
  });
  elements.runtimeInstallButton.addEventListener("click", installRuntime);
  elements.refreshRuntimeButton.addEventListener("click", refreshRuntime);
  elements.openRuntimeDirectoryButton.addEventListener("click", openRuntimeDirectory);
  elements.openLogsDirectoryButton.addEventListener("click", openLogsDirectory);
  elements.settingsStartHarnessButton.addEventListener("click", startHarness);
  elements.settingsRestartHarnessButton.addEventListener("click", restartHarness);
  elements.settingsStopHarnessButton.addEventListener("click", stopHarness);
  elements.settingsOpenLogsButton.addEventListener("click", openLogs);
  elements.settingsRefreshHarnessButton.addEventListener("click", refreshStatus);
  elements.frame.addEventListener("load", () => {
    elements.frameLoading.classList.add("hidden");
    bindFrameZoomShortcuts();
  });
  elements.frame.addEventListener("error", () => {
    elements.frameLoading.textContent = "Harness 页面加载失败，请查看运行日志。";
    elements.frameLoading.classList.remove("hidden");
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
    if (progress.message && state.busy) {
      elements.dshUpdateStatus.textContent = progress.message;
    }
    if (state.updateKind === "dsh") {
      renderUpdateProgress({
        title: progress.error ? "内置 dsh 更新失败" : "正在更新内置 dsh…",
        message: progress.message,
        fraction: progress.fraction,
        done: progress.done === true,
        error: progress.error,
      });
      if (progress.error) setToast(progress.error, true);
    }
    renderRuntime();
  });
  await listen("open-settings", () => showPanel(elements.settingsPanel));
  await listen("zoom-in", () => applyZoom(state.zoom + ZOOM_STEP, { announce: true }));
  await listen("zoom-out", () => applyZoom(state.zoom - ZOOM_STEP, { announce: true }));
  await listen("zoom-reset", () => applyZoom(DEFAULT_ZOOM, { announce: true }));
  await listen("check-app-update", () => checkAppUpdate(true));
  await listen("check-dsh-update", () => checkDshUpdate(false, true));
  await listen("open-logs", openLogs);
  await listen("start-harness", startHarness);
  await listen("restart-harness", restartHarness);
  await listen("stop-harness", stopHarness);
  await listen("refresh-status", async () => Promise.all([refreshStatus(), refreshRuntime()]));
  await listen("update-progress", (event) => {
    const progress = event.payload || {};
    renderUpdateProgress({
      title: "正在更新 App…",
      message: progress.message,
      fraction: progress.fraction,
      done: progress.done === true,
      error: progress.error,
    });
    if (progress.error) {
      setToast(progress.error, true);
    } else if (progress.done && progress.message) {
      setToast(progress.message);
    }
    if (progress.message && state.settingsTab === "updates") {
      elements.appUpdateStatus.textContent = progress.message;
    }
  });
  await listen("window-hidden", () => {
    state.windowHidden = true;
    pauseStatusPolling();
    if (state.memorySaver) unloadHarnessFrame();
  });
  await listen("window-shown", () => {
    state.windowHidden = false;
    resumeStatusPolling();
    restoreHarnessFrame();
    renderLogs();
  });
}

async function initialize() {
  state.zoom = storedZoom();
  await applyZoom(state.zoom, { persist: false });
  elements.launchAtLoginToggle.checked = localStorage.getItem("launchAtLogin") === "true";
  elements.restoreLastWindowToggle.checked = localStorage.getItem("restoreLastWindow") !== "false";
  elements.dockIconToggle.checked = localStorage.getItem("showDockIcon") !== "false";
  elements.autoCheckAppToggle.checked = localStorage.getItem("autoCheckForUpdates") !== "false";
  elements.autoCheckDshToggle.checked = localStorage.getItem("autoCheckHarnessUpdates") !== "false";
  elements.autoInstallDshToggle.checked = localStorage.getItem("autoInstallHarnessUpdates") !== "false";
  elements.autoCheckInterval.value = localStorage.getItem("autoCheckInterval") || "hourly";
  elements.memorySaverToggle.checked = state.memorySaver;
  bindEvents();
  renderSettingsTab();
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
  await call("set_launch_at_login", { enabled: elements.launchAtLoginToggle.checked }).catch(() => {});
  if (elements.autoCheckAppToggle.checked) await checkAppUpdate(false);
  if (elements.autoCheckDshToggle.checked) await checkDshUpdate(false, false);
  scheduleAutomaticUpdateChecks();
  resumeStatusPolling();
}

window.addEventListener("DOMContentLoaded", initialize);
