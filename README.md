# DeepSeek Harness Desk

DeepSeek Harness Desk 当前正式版是基于 **Tauri v2** 构建的跨平台桌面客户端，不是 SwiftUI 原生应用。应用代码位于 [`tauri-app`](tauri-app)，使用 Rust、WebView 和系统原生能力，支持 macOS、Windows 与 Linux。

当前版本：0.3.18。安装包请从 [GitHub Releases](https://github.com/misswell/deepseek-harness-desk/releases/latest) 下载。

## 开发

```bash
cd tauri-app
npm install
npm run tauri dev
```

需要 Node.js 18+、Rust 和 Cargo。测试构建使用 `npm run build:debug`，发布构建使用 `npm run build`；每次构建前会自动把历史本地测试包移入系统废纸篓，再生成 macOS、Windows 和 Linux 对应的安装包/可执行产物，因此构建完成后只保留最新输出。当前 Tauri 版优先使用系统 `dsh`，也会兼容旧版 managed dsh；找不到时点击“安装并启动”会自动下载隔离的 Node.js 和 Harness 运行时，也可通过 `DSH_BIN` 指定已有路径。

Tauri 版功能包括：启动、停止、重启 Harness；自动选择 `3080–3099` 端口；异步 HTTP 健康检查；stdout/stderr 日志；固定单窗口；透明自定义顶栏；菜单栏/系统托盘唤醒；Dock 图标开关；macOS、Windows、Linux 窗口控制。

为降低内存占用，应用默认开启“窗口隐藏时释放 Harness 页面内存”：关闭窗口到菜单栏/托盘时，会卸载渲染中的 Harness 网页（WebView 进程最多可释放数百 MB 内存），Harness 后台服务保持运行；重新打开窗口时页面自动重新加载。可在“设置 → 高级 → 内存与性能”中关闭此选项。

应用内置任务提醒：通过订阅 Harness 的实时事件流，当 Harness 完成任务、向你提问或请求批准时，会在应用图标上显示角标（macOS Dock 角标 / Linux 启动器角标）并发送系统通知；仅当窗口未聚焦时才提醒，回到窗口后角标自动清除。可在“设置 → 高级 → 通知提醒”中分别开关。

主窗口支持快捷键缩放：macOS 使用 `⌘ +` / `⌘ -` / `⌘ 0`，Windows 和 Linux 使用 `Ctrl +` / `Ctrl -` / `Ctrl 0`，缩放范围为 75%–175%，设置会自动保存。

macOS 会分别发布 Apple Silicon（arm64）和 Intel（x86_64）安装包，用户可按处理器架构下载；Windows 和 Linux 也会随版本提供对应安装包。

## 旧版 SwiftUI 工程

根目录的 `DeepSeekHarnessDesk.xcodeproj` 是迁移前的 SwiftUI/AppKit macOS 工程，仅用于兼容旧版本和代码参考，不再是当前正式发布实现。

DeepSeek Harness 由 DeepSeek AI 开发。DeepSeek Harness Desk 是独立的第三方项目，不隶属于 DeepSeek AI，也未获得其认可或赞助。
