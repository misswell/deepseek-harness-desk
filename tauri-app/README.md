# DeepSeek Harness Desk（Tauri）

这是 DeepSeek Harness Desk 0.3.30 的跨平台 Tauri v2 客户端。它使用一个固定的 `main` 窗口承载 Harness Web UI，窗口顶栏由 Tauri 原生拖动区域处理，并通过菜单栏/系统托盘唤醒隐藏窗口。

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

主窗口支持快捷键缩放：macOS 使用 `⌘ +` / `⌘ -` / `⌘ 0`，Windows 和 Linux 使用 `Ctrl +` / `Ctrl -` / `Ctrl 0`，缩放范围为 75%–175%，设置会自动保存。

应用通过订阅 Harness 的实时事件流（`/api/events.mux` + `/api/events.host`）提供任务提醒：Harness 完成任务、向你提问或请求批准时，在应用图标上显示角标（macOS / Linux）并发送系统通知；仅当窗口未聚焦时提醒，回到窗口后角标自动清除。可在“设置 → 高级 → 通知提醒”中开关。

界面国际化：`src/i18n.js` 集中维护中文与英文文案（通过 `data-i18n` 属性与 `t()` 调用使用），默认跟随系统语言，可在“设置 → 通用 → 界面语言”切换；托盘菜单与系统通知随语言同步切换。运行 `npm run test:i18n` 可校验文案键完整性与一致性。
