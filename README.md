# DeepSeek Harness Desk

原生 SwiftUI macOS 桌面客户端，用于启动并承载官方 DeepSeek Harness Web UI。

当前版本：0.2.2

- 无需预装 Node.js 或 `dsh`：首次启动时可一键下载并安装受校验的 Managed Node.js 与 DeepSeek Harness Runtime
- 自动选择 `3080–3099` 的本地端口
- 等待 HTTP 健康检查通过后加载 `WKWebView`
- 支持启动、停止、重启和有限次数的崩溃恢复
- 捕获 Harness stdout/stderr，并写入 `~/Library/Logs/DeepSeek Harness Desk/`
- 退出 App 时清理 Harness 进程树
- 使用 DeepSeek Harness 原版蓝色鲸鱼 Logo
- 主窗口内容延伸到最上边缘，完全移除 macOS 标题栏、工具栏和窗口控制点；窗口仍可拖动、调整大小，使用 `⌘W` 关闭
- 启动后分别检查 App 壳子和内置 `dsh`，发现新版本后默认自动安装；App 壳子会替换并重启，`dsh` 会重启 Harness 使新版本立即生效

如果系统中已经安装 `dsh`，App 会优先使用它；否则在启动失败页点击“一键安装运行时”即可完成安装。Managed Runtime 安装在 `~/Library/Application Support/DeepSeek Harness Desk/runtime`，不会修改用户的 Shell 配置。

更新包发布在本仓库的 GitHub Releases。设置菜单的“更新”页分别管理 App 壳子与内置 `dsh`：可以独立查看当前/最新版本、手动检查，并分别关闭自动检查或自动安装。当前公开包使用本机 Apple Development 证书签名，未使用 Developer ID Application 证书公证；这不是面向任意 Mac 用户的 Developer ID 分发签名，首次打开时可能需要在“系统设置 → 隐私与安全性”中允许打开。安装更新前请将 App 放在可写位置（例如“应用程序”目录）。

项目要求 macOS 14+，代码使用 Swift、SwiftUI、AppKit、WebKit 和 Foundation.Process，不 Fork 或修改 DeepSeek Harness 源码。

DeepSeek Harness 由 DeepSeek AI 开发。DeepSeek Harness Desk 是独立的第三方项目，不隶属于 DeepSeek AI，也未获得其认可或赞助。
