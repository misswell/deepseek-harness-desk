# DeepSeek Harness Desk

DeepSeek Harness Desk 当前正式版是基于 **Tauri v2** 构建的跨平台桌面客户端，不是 SwiftUI 原生应用。当前版本为 **0.3.8**，支持 macOS、Windows 和 Linux。

## 下载

请从 [GitHub Releases](https://github.com/misswell/deepseek-harness-desk/releases/latest) 下载对应平台的最新安装包。macOS 通用包支持 Apple Silicon 和 Intel Mac，并使用 Developer ID Application 证书签名、完成 Apple 公证。

## 当前源码

当前 Tauri v2 实现位于 [`codex/release-v0.3.8` 分支的 `tauri-app`](https://github.com/misswell/deepseek-harness-desk/tree/codex/release-v0.3.8/tauri-app)，技术栈为 Rust、WebView 和 Tauri。它提供 Harness 启停与重启、端口选择、健康检查、日志、单窗口、系统托盘、Dock 图标控制、快捷键缩放以及跨平台窗口控制。

```bash
git switch codex/release-v0.3.8
cd tauri-app
npm install
npm run tauri dev
```

测试构建使用 `npm run build:debug`，发布构建使用 `npm run build`；构建前会自动清理旧的本地测试包。

## 旧版 SwiftUI 工程

默认分支根目录中的 `DeepSeekHarnessDesk.xcodeproj` 是迁移前的 SwiftUI/AppKit macOS 工程，仅用于兼容旧版本和代码参考，不再是当前正式发布实现。

DeepSeek Harness 由 DeepSeek AI 开发。DeepSeek Harness Desk 是独立的第三方项目，不隶属于 DeepSeek AI，也未获得其认可或赞助。
