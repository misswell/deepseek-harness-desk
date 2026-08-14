# DeepSeek Harness Desk

当前桌面版正在迁移到 Tauri v2，入口位于 [`tauri-app`](/Users/guofeng/Code/solo/deepseek-harness-desk/tauri-app)。Tauri 版本使用单一固定窗口、原生拖动区域和系统托盘，解决旧 SwiftUI/AppKit 版本在 macOS 15 上遇到的重复窗口、顶栏拖动和隐藏后无法唤醒问题。

## Tauri 版

```bash
cd tauri-app
npm install
npm run tauri dev
```

需要 Node.js 18+、Rust 和 Cargo。发布构建使用 `npm run tauri build`，会生成 macOS、Windows 和 Linux 对应的安装包/可执行产物。当前 Tauri 版优先使用系统 `dsh`，也会兼容旧版 managed dsh；找不到时可通过 `DSH_BIN` 指定路径。

Tauri 版功能包括：启动、停止、重启 Harness；自动选择 `3080–3099` 端口；异步 HTTP 健康检查；stdout/stderr 日志；固定单窗口；透明自定义顶栏；菜单栏/系统托盘唤醒；Dock 图标开关；macOS、Windows、Linux 窗口控制。

## 旧版工程

根目录的 `DeepSeekHarnessDesk.xcodeproj` 是迁移前的 SwiftUI macOS 工程，仅用于兼容旧版本和参考。它仍保留 managed Node.js、更新器等旧功能，Tauri 版的后续迁移会逐项补齐。

DeepSeek Harness 由 DeepSeek AI 开发。DeepSeek Harness Desk 是独立的第三方项目，不隶属于 DeepSeek AI，也未获得其认可或赞助。
