# DeepSeek Harness Desk

原生 SwiftUI macOS 桌面客户端，用于启动并承载官方 DeepSeek Harness Web UI。

当前版本：0.2.10

- 无需预装 Node.js 或 `dsh`：首次启动时可一键下载并安装受校验的 Managed Node.js 与 DeepSeek Harness Runtime
- 自动选择 `3080–3099` 的本地端口
- 等待 HTTP 健康检查通过后加载 `WKWebView`
- 支持启动、停止、重启和有限次数的崩溃恢复
- 捕获 Harness stdout/stderr，并写入 `~/Library/Logs/DeepSeek Harness Desk/`
- 退出 App 时清理 Harness 进程树
- 使用 DeepSeek Harness 原版蓝色鲸鱼 Logo
- 主窗口内容延伸到最上边缘，完全移除 macOS 标题栏、工具栏和窗口控制点；窗口仍可拖动、调整大小，使用 `⌘W` 关闭
- 关闭主窗口后应用继续驻留 Dock，再次点击 Dock 图标会恢复窗口并重新加载 Harness 页面
- 菜单栏常驻图标支持单击打开主窗口；设置中可关闭 Dock 图标，关闭后通过菜单栏图标继续使用
- 强制退出后再次打开会自动清理上次遗留的 Harness 进程和端口，不会一直停留在“正在启动”
- 更新下载具备超时、完整性、版本和签名校验；替换失败会自动回滚，不会留下损坏的 App
- 启动后分别检查 App 壳子和内置 `dsh`，发现新版本后默认自动安装；App 壳子会替换并重启，`dsh` 会重启 Harness 使新版本立即生效

如果系统中已经安装 `dsh`，App 会优先使用它；否则在启动失败页点击“一键安装运行时”即可完成安装。Managed Runtime 安装在 `~/Library/Application Support/DeepSeek Harness Desk/runtime`，不会修改用户的 Shell 配置。

更新包发布在本仓库的 GitHub Releases。设置菜单的“更新”页分别管理 App 壳子与内置 `dsh`：可以独立查看当前/最新版本、手动检查，并分别关闭自动检查或自动安装。公开发布包必须使用 Developer ID Application 证书签名并完成 Apple 公证；开发构建仍可使用 Apple Development 证书。安装更新前请将 App 放在可写位置（例如“应用程序”目录）。

项目要求 macOS 14+，代码使用 Swift、SwiftUI、AppKit、WebKit 和 Foundation.Process，不 Fork 或修改 DeepSeek Harness 源码。

DeepSeek Harness 由 DeepSeek AI 开发。DeepSeek Harness Desk 是独立的第三方项目，不隶属于 DeepSeek AI，也未获得其认可或赞助。
