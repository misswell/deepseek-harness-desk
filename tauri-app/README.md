# DeepSeek Harness Desk（Tauri）

这是 DeepSeek Harness Desk 的跨平台 Tauri v2 客户端。它使用一个固定的 `main` 窗口承载 Harness Web UI，窗口顶栏由 Tauri 原生拖动区域处理，并通过菜单栏/系统托盘唤醒隐藏窗口。

## 开发

需要 Node.js 18+、Rust 和 Cargo：

```bash
npm install
npm run tauri dev
```

当前版本优先查找系统中的 `dsh`，也会兼容旧 Swift 版安装在应用支持目录中的 managed dsh。找不到时，点击“安装并启动”会自动下载隔离的 Node.js 并安装 DeepSeek Harness；也可以设置 `DSH_BIN` 指向已有可执行文件。

## 构建

```bash
npm run tauri build
```

图标由仓库中的 `Assets/DeepSeekHarnessIcon-Prepared-1024.png` 生成，macOS、Windows 和 Linux 包使用同一套品牌资源。macOS 发布包为 Apple Silicon/Intel 通用包。
