# DeepSeek Harness Desk（Tauri）

这是 DeepSeek Harness Desk 0.3.10 的跨平台 Tauri v2 客户端。它使用一个固定的 `main` 窗口承载 Harness Web UI，窗口顶栏由 Tauri 原生拖动区域处理，并通过菜单栏/系统托盘唤醒隐藏窗口。

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

测试包构建使用 `npm run build:debug`。这两个构建命令都会先清理仓库 `dist/` 和 Tauri 本地 bundle 目录中的旧测试包，不会删除 GitHub Release 上的正式资产。

图标由仓库中的 `Assets/DeepSeekHarnessIcon-Prepared-1024.png` 生成，macOS、Windows 和 Linux 包使用同一套品牌资源。macOS 发布时分别构建 Apple Silicon（arm64）和 Intel（x86_64）安装包。
