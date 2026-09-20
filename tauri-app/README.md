# DeepSeek Harness Desk（Tauri）

这是 DeepSeek Harness Desk 0.3.48 的跨平台 Tauri v2 客户端。它使用一个固定的 `main` 窗口承载 Harness Web UI，窗口顶栏由 Tauri 原生拖动区域处理，并通过菜单栏/系统托盘唤醒隐藏窗口。

## 开发

需要 Node.js 18+、Rust 和 Cargo：

```bash
npm install
npm run tauri dev
```

当前版本优先查找系统中的 `dsh`，也会兼容旧 Swift 版安装在应用支持目录中的 managed dsh。找不到时，点击“安装并启动”会自动下载隔离的 Node.js 并安装 DeepSeek Harness；也可以设置 `DSH_BIN` 指向已有可执行文件。

## 构建

```bash
npm run build
```

测试包构建使用 `npm run build:debug`。这两个构建命令都会先把仓库 `dist/` 和 Tauri 本地 bundle 目录中的历史测试包移入系统废纸篓，因此每次构建完成后只保留最新输出；不会删除 GitHub Release 上的正式资产。

图标由仓库中的 `Assets/DeepSeekHarnessIcon-Prepared-1024.png` 生成，macOS、Windows 和 Linux 包使用同一套品牌资源。macOS 发布时分别构建 Apple Silicon（arm64）和 Intel（x86_64）安装包。

macOS 可在“设置 → 通用 → Dock 图标样式”中选择蓝色、黑色或头像图标。只改运行中的 Dock tile 不够——macOS 在 App 退出后会回退到 App 包内的图标，而 Dock 对固定图标另有缓存、只有自身重启才会重读 App 包，所以选择会先通过 `NSWorkspace` 写进 App 包的自定义图标，再重启一次 Dock；系统拒绝写入时界面会提示该选择只在本次运行内有效。已应用的样式与版本记录在 `~/Library/Application Support/com.deepseek.harnessdesk/dock-icon.json`：App 包在 `/Applications` 下对应用本身完全不可写（连新建文件都会被拒绝），所以这个标记不能放在包内。启动时据此判断图标是否已经就位，避免每次启动都重启 Dock。自定义图标资源由 `scripts/make_prepared_dock_icon.swift` 按 846/1024 安全区与 0.22 圆角生成。

主窗口支持快捷键缩放：macOS 使用 `⌘ +` / `⌘ -` / `⌘ 0`，Windows 和 Linux 使用 `Ctrl +` / `Ctrl -` / `Ctrl 0`，缩放范围为 75%–175%，设置会自动保存。

应用通过订阅 Harness 的实时事件流提供任务提醒：dsh 0.1.2+ 只提供带鉴权的单一多路复用端点 `/api/remote.mux`，且必须先发送打开 `$events` 逻辑流的帧，否则连接会一直静默——这正是旧版提醒失效的原因之一；更早的发布版仍是未鉴权的 `/api/events.mux` + `/api/events.host` 双端点，两种代际的报文都由纯函数分类层识别。Harness 完成任务、向你提问或请求批准时，在应用图标上显示角标（macOS / Linux）并发送系统通知；仅当窗口未聚焦时提醒，回到窗口后角标自动清除。可在“设置 → 高级 → 通知提醒”中开关。

macOS 的通知直接走 `UNUserNotificationCenter`：`tauri-plugin-notification` 依赖的 `mac-notification-sys` 使用 Apple 已移除的 `NSUserNotification`，在现代系统上发出去的通知会被静默丢弃，因此该插件只保留给 Windows 与 Linux。启动时在一个后台线程申请一次 alert/badge/sound 权限；之后每条提醒只缓存“确实已授权”这一种结果，其余情况都实时回读系统状态（授权弹窗可能要几十秒才被回答，超时不是拒绝；在系统设置里改回来的也无需重启就生效）。“设置 → 高级 → 通知提醒”会实时显示系统权限状态（未开启时给出提示与“打开系统设置”按钮），并提供“发送测试通知”确认链路真的通——它会重新发起授权请求，所以误关弹窗也能补救。投递失败或未授权都会写入日志页，而不是假装已经送达。`scripts/make_notification_probe.sh` 构建 `scripts/notification_probe.swift`，把两种投递方式打到真机上对比，作为这段取舍的实证。

回归测试：Rust 侧为报文分类与去重的 `mux_*` / `legacy_*` / `interaction_*` 单测；前端 `src/notification-permission.js`（权限状态归一化与提示判定）对应 `tests/notification-permission.test.mjs`，跨语言契约 `tests/notification-wiring.test.mjs` 守住 open 帧、分类层、投递方式与命令注册。

界面国际化：`src/i18n.js` 集中维护中文与英文文案（通过 `data-i18n` 属性与 `t()` 调用使用），默认跟随系统语言，可在“设置 → 通用 → 界面语言”切换；托盘菜单与系统通知随语言同步切换。运行 `npm run test:i18n` 可校验文案键完整性与一致性。
