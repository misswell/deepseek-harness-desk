# DeepSeek Harness Desk

原生 SwiftUI macOS 桌面客户端，用于启动并承载官方 DeepSeek Harness Web UI。

当前版本是 MVP：

- 通过 `Foundation.Process` 启动 PATH 中已有的 `dsh`
- 自动选择 `3080–3099` 的本地端口
- 等待 HTTP 健康检查通过后加载 `WKWebView`
- 支持启动、停止、重启和有限次数的崩溃恢复
- 捕获 Harness stdout/stderr，并写入 `~/Library/Logs/DeepSeek Harness Desk/`
- 退出 App 时清理 Harness 进程树

开发版前置条件：请先按 DeepSeek Harness 官方文档安装运行时，并确认 `dsh` 命令在 `PATH` 中。启动 App 后不需要手动打开 Terminal 或先启动 Web 服务，Desk 会负责启动 `dsh web`。Managed Node Runtime、Harness 版本更新、回滚、Sparkle 和发布打包属于后续阶段，尚未在 MVP 中实现。

项目要求 macOS 14+，代码使用 Swift、SwiftUI、AppKit、WebKit 和 Foundation.Process，不 Fork 或修改 DeepSeek Harness 源码。

DeepSeek Harness 由 DeepSeek AI 开发。DeepSeek Harness Desk 是独立的第三方项目，不隶属于 DeepSeek AI，也未获得其认可或赞助。
