/**
 * Minimal internationalization for the DeepSeek Harness Desk shell.
 *
 * Supported languages: Chinese (zh) and English (en). The preference is
 * stored as `uiLang`: "auto" (follow the system/browser language), "zh", or
 * "en". All user-facing shell strings live in the `messages` dictionary so
 * adding a language is a matter of adding one entry.
 */

export const LANG_PREF_STORAGE_KEY = "uiLang";

export const LANGUAGES = [
  { code: "auto", zh: "跟随系统", en: "System" },
  { code: "zh", zh: "中文", en: "Chinese" },
  { code: "en", zh: "English", en: "English" },
];

const messages = {
  zh: {
    // Startup / frame
    "startup.title.starting": "正在启动 DeepSeek Harness…",
    "startup.title.running": "Harness 已启动",
    "startup.title.failed": "DeepSeek Harness 无法启动",
    "startup.title.stopped": "DeepSeek Harness 已停止",
    "startup.message.preparing": "正在准备本机运行环境，请稍候。",
    "startup.message.stopped": "点击启动按钮重新打开 Harness。",
    "startup.error": "发生未知错误。",
    "startup.runtime": "Runtime",
    "startup.harness": "Harness",
    "startup.port": "Port",
    "frame.loading": "正在加载 Harness 页面…",
    "frame.failed": "Harness 页面加载失败，请查看运行日志。",

    // Common status / buttons
    "common.checking": "检查中…",
    "common.notChecked": "未检查",
    "common.notInstalled": "未安装",
    "common.sysPath": "系统 PATH",
    "common.unknown": "未知错误",
    "common.starting": "启动中…",
    "common.updating": "更新中…",
    "common.installing": "安装中…",
    "common.stopping": "停止中…",
    "common.running": "运行中",
    "common.stopped": "已停止",
    "common.failed": "启动失败",
    "common.restart": "重新启动",
    "common.viewLogs": "查看日志",
    "common.start": "启动 Harness",
    "common.started": "已启动",

    // Update banner
    "update.banner": "发现新版本",
    "update.install": "更新",
    "update.ignoreTitle": "忽略此版本",
    "update.dismissTitle": "关闭提醒",

    // Logs panel
    "logs.title": "运行日志",
    "logs.clear": "清空",
    "logs.closeTitle": "关闭日志",
    "logs.empty": "暂无日志输出",

    // Settings
    "settings.title": "设置",
    "settings.category": "设置分类",
    "settings.closeTitle": "关闭设置",
    "tabs.general": "通用",
    "tabs.updates": "更新",
    "tabs.harness": "Harness",
    "tabs.advanced": "高级",
    "tabs.about": "关于",

    // General
    "general.section.startup": "启动与窗口",
    "general.launchAtLogin": "登录时启动",
    "general.launchAtLoginDesc": "登录 macOS、Windows 或 Linux 桌面后自动启动应用。",
    "general.restoreWindow": "恢复上次窗口",
    "general.restoreWindowDesc": "应用启动或从菜单栏唤醒时恢复主窗口内容。",
    "general.dockIcon": "在 Dock 中显示图标",
    "general.dockIconDesc": "关闭后应用仍在运行，可从菜单栏图标唤醒窗口。",
    "general.note": "窗口关闭时应用会继续运行，托盘/菜单栏图标仍可打开主窗口。",
    "general.language": "界面语言",
    "general.languageDesc": "选择界面显示语言。",
    "lang.auto": "跟随系统",
    "lang.zh": "中文",
    "lang.en": "English",

    // Updates
    "updates.section.app": "App 壳子",
    "updates.currentVersion": "当前版本",
    "updates.latestVersion": "最新版本",
    "updates.status": "状态",
    "updates.checkApp": "检查 App 更新…",
    "updates.installApp": "下载并安装",
    "updates.openDownloads": "打开下载页",
    "updates.viewRelease": "查看 Release",
    "updates.autoCheckApp": "自动检查 App 更新",
    "updates.autoCheckAppDesc": "后台静默检查，发现新版本后在主界面提示。",
    "updates.autoCheckInterval": "自动检查频率",
    "updates.interval.hourly": "每小时",
    "updates.interval.daily": "每天",
    "updates.interval.weekly": "每周",
    "updates.note": "自动更新不可用或安装失败时，可点击“打开下载页”前往 GitHub Releases 手动下载安装包。",
    "updates.section.dsh": "内置 DeepSeek Harness / dsh",
    "updates.checkDsh": "检查内置 dsh 更新…",
    "updates.updateDsh": "更新 dsh",
    "updates.autoCheckDsh": "自动检查内置 dsh 更新",
    "updates.autoCheckDshDesc": "启动后和后台定期检查 npm 最新版本。",
    "updates.autoInstallDsh": "自动安装内置 dsh 更新",
    "updates.autoInstallDshDesc": "发现新版后自动安装，并重启 Harness。",
    "updates.note2": "更新过程会保留当前版本；安装完成后自动切换到新版本。系统 PATH 中手动安装的 dsh 不会被修改。",

    // Harness page
    "harness.section.current": "当前 Harness",
    "harness.status": "状态",
    "harness.version": "版本",
    "harness.port": "端口",
    "harness.start": "启动 Harness",
    "harness.restart": "重启 Harness",
    "harness.stop": "停止 Harness",
    "harness.openLogs": "打开日志",
    "harness.executable": "运行程序",
    "harness.refresh": "刷新状态",

    // Advanced
    "advanced.section.runtime": "运行环境",
    "advanced.runtimeDir": "Runtime 目录",
    "advanced.logsDir": "日志目录",
    "advanced.installRuntime": "安装运行时",
    "advanced.recheck": "重新检查",
    "advanced.openDir": "打开目录",
    "advanced.openLogsDir": "打开日志目录",
    "advanced.runtimeNote": "首次启动会自动准备隔离的 Node.js 和 DeepSeek Harness，不依赖系统 Node.js 版本。",
    "advanced.section.memory": "内存与性能",
    "advanced.memorySaver": "窗口隐藏时释放 Harness 页面内存",
    "advanced.memorySaverDesc": "关闭窗口到菜单栏/托盘时卸载 Harness 页面以释放内存；重新打开窗口时页面会重新加载，需要短暂等待。",
    "advanced.memorySaverNote": "释放的是渲染页面（WebView）占用的内存，Harness 后台服务会保持运行，重新打开窗口即可继续使用。",
    "advanced.section.notify": "通知提醒",
    "advanced.notifyEnabled": "启用任务通知",
    "advanced.notifyEnabledDesc": "任务完成或需要交互时，在应用图标上显示角标并发送系统通知。",
    "advanced.notifyTask": "任务完成时提醒",
    "advanced.notifyTaskDesc": "Harness 完成一项任务后通知你。",
    "advanced.notifyInteraction": "需要交互时提醒",
    "advanced.notifyInteractionDesc": "Harness 向你提问或请求批准时通知你。",
    "advanced.notifyNote": "仅当窗口未处于聚焦状态时才会提醒，避免打扰正在使用的你；回到窗口后角标会自动清除。",

    // About
    "about.tagline": "原生跨平台 DeepSeek Harness 桌面客户端",
    "about.disclaimer": "DeepSeek Harness 由 DeepSeek AI 开发。DeepSeek Harness Desk 是独立的第三方项目，不隶属于 DeepSeek AI，也未获得其认可或赞助。",
    "about.version": "版本检查中…",

    // Runtime / harness status (JS)
    "runtime.builtin": "内置 Node + dsh",
    "runtime.installPending": "待安装",
    "runtime.pathDefault": "首次启动会自动安装 Node.js 和 dsh。",
    "runtime.ready": "运行时已就绪。",
    "runtime.installHint": "点击安装，应用会自动准备运行环境。",
    "runtime.installButton": "安装运行时",
    "runtime.installButtonBusy": "安装中…",
    "runtime.unavailable": "无法检查",

    // Update flow (JS)
    "update.checking": "正在检查更新…",
    "update.latest": "已是最新版本",
    "update.newVersion": "发现新版本",
    "update.app.title": "正在更新 App…",
    "update.app.preparing": "准备下载更新包…",
    "update.app.failed": "App 更新失败",
    "update.app.doneToast": "App 更新完成",
    "update.dsh.title": "正在更新内置 dsh…",
    "update.dsh.preparing": "准备安装 {version}…",
    "update.dsh.failed": "内置 dsh 更新失败",
    "update.dsh.done": "内置 dsh 更新完成：{version}",
    "update.dsh.toast": "内置 dsh 已更新到 {version}",
    "update.dsh.autoToast": "内置 dsh 已自动更新到 {version}",
    "update.progress.updating": "正在更新…",
    "update.progress.preparing": "准备中…",
    "update.progress.failed": "失败",
    "update.progress.failedTitle": "更新失败",
    "update.progress.label": "更新进度",
    "update.checkingApp": "检查中…",
    "update.bannerWith": "发现新版本 {version}",
    "startup.failedToStart": "Harness 未能启动。",
    "phase.dshUpdate.message": "更新期间会暂时停止 Harness，完成后自动恢复。",
    "about.versionLabel": "版本 {version}",

    // Toasts / misc (JS)
    "toast.zoom": "界面缩放：{percent}",
    "toast.harnessStarted": "Harness 已启动",
    "toast.harnessRestarted": "Harness 已重启",
    "toast.harnessStopped": "Harness 已停止",
    "toast.runtimeInstalled": "运行时安装完成",
    "toast.dockOn": "Dock 图标已开启",
    "toast.dockOff": "Dock 图标已关闭，可从菜单栏图标唤醒",
    "toast.launchOn": "已开启登录时启动",
    "toast.launchOff": "已关闭登录时启动",
    "toast.appNewVersion": "发现 App 新版本 {version}",
    "toast.appUpToDate": "已是最新版本",
    "error.notTauri": "当前页面不是 Tauri 应用窗口",
    "error.unexpected": "发生未知错误，请查看运行日志。",
  },
  en: {
    "startup.title.starting": "Starting DeepSeek Harness…",
    "startup.title.running": "Harness started",
    "startup.title.failed": "DeepSeek Harness failed to start",
    "startup.title.stopped": "DeepSeek Harness stopped",
    "startup.message.preparing": "Preparing your local environment, please wait.",
    "startup.message.stopped": "Click the start button to reopen Harness.",
    "startup.error": "An unknown error occurred.",
    "startup.runtime": "Runtime",
    "startup.harness": "Harness",
    "startup.port": "Port",
    "frame.loading": "Loading Harness page…",
    "frame.failed": "Failed to load the Harness page. Check the logs.",

    "common.checking": "Checking…",
    "common.notChecked": "Not checked",
    "common.notInstalled": "Not installed",
    "common.sysPath": "System PATH",
    "common.unknown": "Unknown error",
    "common.starting": "Starting…",
    "common.updating": "Updating…",
    "common.installing": "Installing…",
    "common.stopping": "Stopping…",
    "common.running": "Running",
    "common.stopped": "Stopped",
    "common.failed": "Failed to start",
    "common.restart": "Restart",
    "common.viewLogs": "View logs",
    "common.start": "Start Harness",
    "common.started": "Started",

    "update.banner": "New version available",
    "update.install": "Update",
    "update.ignoreTitle": "Ignore this version",
    "update.dismissTitle": "Dismiss",

    "logs.title": "Run logs",
    "logs.clear": "Clear",
    "logs.closeTitle": "Close logs",
    "logs.empty": "No log output yet",

    "settings.title": "Settings",
    "settings.category": "Settings categories",
    "settings.closeTitle": "Close settings",
    "tabs.general": "General",
    "tabs.updates": "Updates",
    "tabs.harness": "Harness",
    "tabs.advanced": "Advanced",
    "tabs.about": "About",

    "general.section.startup": "Startup & Window",
    "general.launchAtLogin": "Launch at login",
    "general.launchAtLoginDesc": "Start the app automatically after logging into macOS, Windows, or Linux.",
    "general.restoreWindow": "Restore last window",
    "general.restoreWindowDesc": "Restore the main window on launch or when waking from the menu bar.",
    "general.dockIcon": "Show Dock icon",
    "general.dockIconDesc": "When off, the app keeps running and can be woken from the menu bar icon.",
    "general.note": "Closing the window keeps the app running; use the tray/menu bar icon to reopen the main window.",
    "general.language": "Language",
    "general.languageDesc": "Choose the interface language.",
    "lang.auto": "System",
    "lang.zh": "Chinese",
    "lang.en": "English",

    "updates.section.app": "App shell",
    "updates.currentVersion": "Current version",
    "updates.latestVersion": "Latest version",
    "updates.status": "Status",
    "updates.checkApp": "Check for App updates…",
    "updates.installApp": "Download & install",
    "updates.openDownloads": "Open downloads",
    "updates.viewRelease": "View release",
    "updates.autoCheckApp": "Auto-check for App updates",
    "updates.autoCheckAppDesc": "Check silently in the background and notify you in the UI when a new version is found.",
    "updates.autoCheckInterval": "Check frequency",
    "updates.interval.hourly": "Hourly",
    "updates.interval.daily": "Daily",
    "updates.interval.weekly": "Weekly",
    "updates.note": "If auto-update is unavailable or fails, use \u201cOpen downloads\u201d to grab the installer from GitHub Releases.",
    "updates.section.dsh": "Bundled DeepSeek Harness / dsh",
    "updates.checkDsh": "Check for bundled dsh updates…",
    "updates.updateDsh": "Update dsh",
    "updates.autoCheckDsh": "Auto-check bundled dsh updates",
    "updates.autoCheckDshDesc": "Checks npm for the latest version at startup and in the background.",
    "updates.autoInstallDsh": "Auto-install bundled dsh updates",
    "updates.autoInstallDshDesc": "Automatically installs new versions and restarts Harness.",
    "updates.note2": "Updates keep the current version; the new one becomes active after install. A dsh installed manually on the system PATH is never modified.",

    "harness.section.current": "Current Harness",
    "harness.status": "Status",
    "harness.version": "Version",
    "harness.port": "Port",
    "harness.start": "Start Harness",
    "harness.restart": "Restart Harness",
    "harness.stop": "Stop Harness",
    "harness.openLogs": "Open logs",
    "harness.executable": "Executable",
    "harness.refresh": "Refresh status",

    "advanced.section.runtime": "Runtime environment",
    "advanced.runtimeDir": "Runtime directory",
    "advanced.logsDir": "Logs directory",
    "advanced.installRuntime": "Install runtime",
    "advanced.recheck": "Re-check",
    "advanced.openDir": "Open directory",
    "advanced.openLogsDir": "Open logs directory",
    "advanced.runtimeNote": "On first launch the app prepares an isolated Node.js and DeepSeek Harness, independent of the system Node.js.",
    "advanced.section.memory": "Memory & performance",
    "advanced.memorySaver": "Release Harness page memory when hidden",
    "advanced.memorySaverDesc": "Unload the Harness page when the window is hidden to free memory; the page reloads when the window is reopened.",
    "advanced.memorySaverNote": "This frees the rendering page (WebView) memory; the Harness backend keeps running, and reopening the window resumes it.",
    "advanced.section.notify": "Notifications",
    "advanced.notifyEnabled": "Enable task notifications",
    "advanced.notifyEnabledDesc": "Show a badge on the app icon and send a system notification when a task completes or needs input.",
    "advanced.notifyTask": "Notify on task completion",
    "advanced.notifyTaskDesc": "Notify you when the Harness finishes a task.",
    "advanced.notifyInteraction": "Notify when interaction is needed",
    "advanced.notifyInteractionDesc": "Notify you when the Harness asks a question or requests approval.",
    "advanced.notifyNote": "Notifications only fire while the window is unfocused so they never interrupt your work; the badge clears when you return.",

    "about.tagline": "Native cross-platform DeepSeek Harness desktop client",
    "about.disclaimer": "DeepSeek Harness is developed by DeepSeek AI. DeepSeek Harness Desk is an independent third-party project, not affiliated with or endorsed by DeepSeek AI.",
    "about.version": "Checking version…",

    "runtime.builtin": "Bundled Node + dsh",
    "runtime.installPending": "Not installed",
    "runtime.pathDefault": "Node.js and dsh are installed automatically on first launch.",
    "runtime.ready": "Runtime is ready.",
    "runtime.installHint": "Click install and the app prepares the runtime automatically.",
    "runtime.installButton": "Install runtime",
    "runtime.installButtonBusy": "Installing…",
    "runtime.unavailable": "Unavailable",

    "update.checking": "Checking for updates…",
    "update.latest": "Up to date",
    "update.newVersion": "New version available",
    "update.app.title": "Updating App…",
    "update.app.preparing": "Preparing to download the update…",
    "update.app.failed": "App update failed",
    "update.app.doneToast": "App update finished",
    "update.dsh.title": "Updating bundled dsh…",
    "update.dsh.preparing": "Preparing to install {version}…",
    "update.dsh.failed": "Bundled dsh update failed",
    "update.dsh.done": "Bundled dsh updated to {version}",
    "update.dsh.toast": "Bundled dsh updated to {version}",
    "update.dsh.autoToast": "Bundled dsh auto-updated to {version}",
    "update.progress.updating": "Updating…",
    "update.progress.preparing": "Preparing…",
    "update.progress.failed": "Failed",
    "update.progress.failedTitle": "Update failed",
    "update.progress.label": "Update progress",
    "update.checkingApp": "Checking…",
    "update.bannerWith": "New version {version} available",
    "startup.failedToStart": "Harness failed to start.",
    "phase.dshUpdate.message": "Harness is paused during the update and resumes automatically when done.",
    "about.versionLabel": "Version {version}",

    "toast.zoom": "Zoom: {percent}",
    "toast.harnessStarted": "Harness started",
    "toast.harnessRestarted": "Harness restarted",
    "toast.harnessStopped": "Harness stopped",
    "toast.runtimeInstalled": "Runtime installed",
    "toast.dockOn": "Dock icon enabled",
    "toast.dockOff": "Dock icon disabled; wake from the menu bar icon",
    "toast.launchOn": "Launch at login enabled",
    "toast.launchOff": "Launch at login disabled",
    "toast.appNewVersion": "New App version {version} available",
    "toast.appUpToDate": "Up to date",
    "error.notTauri": "This page is not a Tauri app window",
    "error.unexpected": "An unknown error occurred; check the logs.",
  },
};

/** Resolve a language preference ("auto" | "zh" | "en") to an actual code. */
export function resolveLanguage(pref) {
  if (pref === "zh" || pref === "en") return pref;
  const lang = (navigator.language || "zh-CN").toLowerCase();
  return lang.startsWith("zh") ? "zh" : "en";
}

/** Read the persisted preference, defaulting to "auto". */
export function storedLanguagePreference() {
  const value = localStorage.getItem(LANG_PREF_STORAGE_KEY);
  return value === "zh" || value === "en" || value === "auto" ? value : "auto";
}

/**
 * Translate a key using the resolved language. `vars` are interpolated as
 * `{name}` placeholders. Falls back to the key itself so a missing entry is
 * visible during development instead of silently blank.
 */
export function translate(key, lang, vars) {
  const table = messages[lang] || messages.zh;
  let value = table[key];
  if (value === undefined) return key;
  if (vars) {
    for (const [name, replacement] of Object.entries(vars)) {
      value = value.replaceAll(`{${name}}`, String(replacement));
    }
  }
  return value;
}

/** Apply static translations to `[data-i18n]` / `[data-i18n-title]` nodes. */
export function applyStaticTranslations(root, lang) {
  const t = (key) => translate(key, lang);
  for (const element of root.querySelectorAll("[data-i18n]")) {
    element.textContent = t(element.dataset.i18n);
  }
  for (const element of root.querySelectorAll("[data-i18n-title]")) {
    element.title = t(element.dataset.i18nTitle);
  }
  for (const element of root.querySelectorAll("[data-i18n-aria-label]")) {
    element.setAttribute("aria-label", t(element.dataset.i18nAriaLabel));
  }
}

/** Convenience: current language of the page, set on <html lang>. */
export function currentDocumentLang() {
  return document.documentElement.lang === "en" ? "en" : "zh";
}
