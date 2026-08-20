# DeepSeek Harness Desk 项目规则

## 项目与仓库

- 项目路径：`/Users/guofeng/Code/solo/deepseek-harness-desk`
- GitHub 仓库：`misswell/deepseek-harness-desk`
- 当前桌面客户端是 Tauri v2；旧版 SwiftUI 工程的发布事实只作为历史记录参考。

## 完成、验证与推送

- 用户偏好：本项目每次改动完成并验证通过后，自动提交并推送到 GitHub；推送完成后在回复中附上 commit 或分支链接。
- 普通改动不自动创建 Release 或 tag；只有用户明确要求发布时才执行正式发布流程。
- 正式发布必须使用认证签名和 Apple 公证后的包，不能把本地未公证 ZIP 当作正式发布包。
- 每次本地编译前，删除项目根目录旧的 `build-test-*`、`build-debug-*`、`build-ui-debug`；不要删除正式发布产物目录。

## 正式发布与公证

- 正式发布走 `.github/workflows/release.yml`：tag push 或手动指定 tag → Archive → Developer ID Export → zip → Apple notarization → staple/validate → 创建或更新 Release。
- 工作流优先使用 `APPLE_API_KEY_ID`、`APPLE_API_ISSUER_ID`、`APPLE_API_PRIVATE_KEY`；三者均未配置时回退 `APPLE_ID`、`APPLE_APP_SPECIFIC_PASSWORD`、`APPLE_TEAM_ID`。
- GitHub Actions runner 无法读取本机 Keychain profile；认证缺失时必须明确指出缺少的 Secret，不得绕过公证。
- 正式构建必须走 `xcodebuild archive` + `xcodebuild -exportArchive -exportOptionsPlist ExportOptions-DeveloperID.plist`；普通 `xcodebuild build` 产物带 `get-task-allow` 且没有 secure timestamp，不能用于公证。
- 本机曾验证的签名身份：`Developer ID Application: Guofeng Liu (U8U443D7ZL)`。只有在当前会话实际验证成功后才能复用 `notarytool` profile，不能凭历史名称猜测可用性。
- 不输出或提交 Apple 密码、API 私钥、Sparkle 私钥或其他签名秘密。

## 已发布版本记录

- Tauri v2 发布资产为分架构 DMG/tar.gz（macOS arm64+x64）、exe/msi（Windows）、deb/AppImage/rpm（Linux），共 9 个资产。
- `v0.3.12`：修复 macOS 菜单栏替换导致 Edit 菜单和 Cmd+C/V/X 消失，改为在 Tauri 默认菜单上追加缩放项。
- `v0.3.13`：补充 `core:webview:allow-set-webview-zoom` ACL，并恢复设置面板顶部的分隔线与辅助文字。
- `v0.3.14`：放大快捷键改为 `CmdOrCtrl+=`，最小缩放改为 0.5；发布工作流改为 arm64 优先发布、其他平台后台追加资产。
- `v0.3.15`：修复设置页更新下载链接。
- `v0.3.16`：修复 `dsh: command not found`，通过 `~/.local/bin/dsh` wrapper 指向内置 Node 与最新 managed dsh；arm64 DMG 已验证 Developer ID 签名和公证。
- 历史正式版本 `v0.2.11` 至 `v0.2.17` 均以认证签名、公证、staple 和 `spctl` 校验为准；历史 Release 链接和 digest 以 GitHub 记录为准，不以单次 Actions 状态推断公证结果。

## Tauri v2 生命周期与内存

- 开启“窗口隐藏时释放 Harness 页面内存”时，关闭主窗口必须销毁主 WebView；仅移除 iframe 不会销毁共享的 WebKit `WebContent` 进程。
- 关闭窗口时保留 Harness 后端和托盘；重新打开时异步重建主 WebView，避免 Tauri Windows 同步建窗死锁，并处理快速重开竞态。
- 主动退出时必须同时验证 App 进程与 Harness 子进程消失；优雅停止不能无限等待，超时应兜底强制停止。
- 重新打开时必须清除“保持托盘存活”标志，不能让该标志反过来阻塞新 WebView 创建。
- 窗口失焦释放、空闲页面回收和隐藏释放都只针对页面/渲染进程；Harness 后端会话是否保持运行由对应功能设计决定。

## Tauri v2 沙盒与 IPC

- `tauri://localhost` 在 App Sandbox 下正常工作，不要因误判而引入本地 HTTP 服务器。
- 白屏常见根因是 `visible: false` 配合错误的延迟 `show()`，不是 tauri 协议被沙盒阻止。
- 使用远程 `http://` origin 会触发 Tauri v2 ACL 的 remote-origin 检查；本地 `tauri://localhost` origin 不触发该检查。
- 沙盒 App 的 IPC 命令必须在 capabilities 中显式声明，并通过 `permissions/commands.toml` 定义权限。

## Harness 页面与缩放规则

- Harness 后端 `http://127.0.0.1:3080/` 返回 200 时，`Frame load interrupted` 通常是 WebKit 策略取消（`WKErrorDomain` code `102`），不是浏览器版本不兼容。
- 仅把用户点击的主页面外链交给系统浏览器；允许内部或非用户触发导航继续，并忽略预期的策略取消。
- Harness 页面位于跨域 iframe，外层 window 收不到 iframe 焦点内的快捷键；缩放必须依靠 macOS 菜单加速键，放大使用 `CmdOrCtrl+=`。
- `WebviewWindow.setZoom()` 需要 `core:webview:allow-set-webview-zoom` 权限；`core:webview:default` 不包含该权限。

## dsh 与 workspace

- managed dsh 安装在 App 私有数据目录，launcher 使用 `#!/usr/bin/env node`；App 通过 `~/.local/bin/dsh` wrapper 让终端可直接使用。
- 仅在 active dsh 是 managed 版本时创建 wrapper；`DSH_BIN` 或系统 PATH 中已有的 dsh 不覆盖。Windows 跳过该 wrapper。
- `~/.dsh/profiles/<name>/pnpm-workspace.yaml` 声明 `packages: [.]` 后，profile 是 pnpm workspace root；在其中 install/add 依赖需加 `-w`，或在 `.npmrc` 设置 `ignore-workspace-root-check=true`。
- git 仓库依赖可能还需要在 `pnpm-workspace.yaml` 的 `allowBuilds` 中放行 prepare 脚本。

## 退出与在线更新

- macOS 退出使用 `ApplicationTerminationCoordinator`，优雅停止最多等待 8 秒，超时调用 `forceStopImmediately()`；`NSApp.reply(toApplicationShouldTerminate:)` 只能调用一次。
- 在线更新替换脚本必须通过 `nohup` 脱离旧 App 生命周期，使用 `open -n` 启动新实例，写入 `~/Library/Logs/DeepSeek Harness Desk/update.log`，校验真实 PID 和路径；启动失败必须回滚旧 App。

## 构建缓存

- `tauri-app/src-tauri/target` 是本项目最大的可清理构建缓存；任务结束时检查并清理，移动到废纸篓即可，下次构建会自动重建。
- 历史上曾清理过约 7.6G 的 `/Users/guofeng/Code/solo/deepseek-harness-desk/tauri-app/src-tauri/target`；不要把正式发布产物目录误删。
