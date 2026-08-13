# DeepSeek Harness Desk

原生 SwiftUI macOS 桌面客户端，用于启动并承载官方 DeepSeek Harness Web UI。

当前版本：0.2.0

- 无需预装 Node.js 或 `dsh`：首次启动时可一键下载并安装受校验的 Managed Node.js 与 DeepSeek Harness Runtime
- 自动选择 `3080–3099` 的本地端口
- 等待 HTTP 健康检查通过后加载 `WKWebView`
- 支持启动、停止、重启和有限次数的崩溃恢复
- 捕获 Harness stdout/stderr，并写入 `~/Library/Logs/DeepSeek Harness Desk/`
- 退出 App 时清理 Harness 进程树
- 使用 DeepSeek Harness 原版蓝色鲸鱼 Logo
- 启动后自动检查 GitHub Releases，并支持从“帮助 → 检查更新…”下载、替换和重启 App

如果系统中已经安装 `dsh`，App 会优先使用它；否则在启动失败页点击“一键安装运行时”即可完成安装。Managed Runtime 安装在 `~/Library/Application Support/DeepSeek Harness Desk/runtime`，不会修改用户的 Shell 配置。

更新包发布在本仓库的 GitHub Releases。当前公开包使用本机 Apple Development 证书签名，未使用 Developer ID Application 证书公证；首次打开时 macOS 可能显示安全提示。安装更新前请将 App 放在可写位置（例如“应用程序”目录）。

项目要求 macOS 14+，代码使用 Swift、SwiftUI、AppKit、WebKit 和 Foundation.Process，不 Fork 或修改 DeepSeek Harness 源码。

DeepSeek Harness 由 DeepSeek AI 开发。DeepSeek Harness Desk 是独立的第三方项目，不隶属于 DeepSeek AI，也未获得其认可或赞助。
