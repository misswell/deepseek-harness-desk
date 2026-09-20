# DeepSeek Harness Desk 项目规则

## 项目与仓库

- 项目路径：`/Users/guofeng/Code/solo/deepseek-harness-desk`
- GitHub 仓库：`misswell/deepseek-harness-desk`
- 当前桌面客户端是 Tauri v2；旧版 SwiftUI 工程的发布事实只作为历史记录参考。

## 完成、验证与推送

- 用户偏好：本项目每次改动完成并验证通过后，自动提交并推送到 GitHub，创建新的 patch 版本 tag 和正式 GitHub Release；推送完成后在回复中附上 commit、Release 和相关资产链接。
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
- `v0.3.32`：修复窗口关闭 WebView 前未及时保存主题，避免深色主题重开时白屏或闪烁。
- `v0.3.33`：兼容 dsh 0.1.2+ token 鉴权：就绪检查改为任意 HTTP 响应即就绪（不再理解 dsh 登录流程），后台兑换 launch token 并把 cookie 注入 WKHTTPCookieStore（WebKit 会丢弃跨域 iframe 内的 Set-Cookie），事件监听迁移到 `/api/remote.mux` 并回退旧双端点，dsh 启动加 `--no-open`。
- `v0.3.38`：修复重开窗口白屏：v0.3.32 只修了 HTML 层（localStorage 首帧主题），WKWebView 原生层在页面首帧前仍是不透明白底。wry 的 `set_background_color`/创建期 `drawsBackground=false` 已随 tauri `macos-private-api` 启用（无独立 `transparent` tauri feature），但 post-build 的 `set_background_color` 走事件循环队列可能晚于首帧；必须在 `WebviewWindowBuilder::build()` 之前调用 `builder.background_color(...)`（按 `NSApp.effectiveAppearance` 判定），ThemeChanged 也要通过 `WebviewWindow::set_background_color` 同时刷窗口与 WebView 层。
- `v0.3.39`：修复深色主题下重载/卸载 Harness 页面时的白屏。v0.3.32/v0.3.38 只处理了外壳文档与 WKWebView 原生层，管不到跨域 `iframe` 自身：`removeAttribute("src")`（卸载/回收/重启）或导航中时，iframe 会画内层文档的默认白色底面，盖住深色外壳。现在 iframe 默认透明且 `opacity: 0`，只在真实 Harness 文档 `load` 后加 `.frame-ready` 才显示（用 `frameDocumentIsCurrent()` 排除 about:blank），并用 4s 兜底定时器防止 WebKit 取消 frame load 时页面永久隐藏；深色主题同时给 iframe 设 `color-scheme: dark` 兜底。
- `v0.3.40`：支持向 Harness 输入框粘贴图片和文件。Harness 网页自身的粘贴命令会抢下 paste 事件、阻止默认插入后把文件丢掉，所以原先只有纯文本能粘贴；而 Harness 页面在跨域 `iframe` 里，外壳既够不到输入框也拿不到文件负载。修复分两层：把 `src/paste-bridge.js` 注入每个 frame（`forMainFrameOnly: false`），在 Harness frame 的捕获阶段拦截文件粘贴，改走输入框自带的 `input[type=file]`（回形针按钮的同一条路径，也是唯一被 Harness 认账的附件入口），同时把窗口 `dragDropEnabled` 设为 `false`，让 WebKit 而不是 Tauri 处理拖拽文件；macOS 侧再用 `NSEvent` 本地监视器在 `Cmd+V` 时读 `NSPasteboard`（截图、复制的图片、访达里复制的文件），base64 后交给外壳入口 `window.__dshDeskPasteFiles`，再 postMessage 给 Harness frame。只有剪贴板里确实是文件/图片时才吞掉 `Cmd+V`，纯文本粘贴与 Edit 菜单保持原样；单次超过 32MB 跳过并提示，Harness 不接受的图片格式（如 TIFF）在页面内转成 PNG。已在“外壳页 + 跨域 iframe + 真实 Harness UI”的 WKWebView 里验证两条路径都会生成附件缩略图。
- `v0.3.41`：修复 Harness 页面里的链接点击后毫无反应。Harness 用 `target="_blank"` 渲染所有网页链接，而主窗口原先没有注册 `on_new_window`：wry 的 `createWebViewWithConfiguration` 在无 handler 时返回 nil，WebKit 直接把新窗口请求丢掉；同帧外链则会把整个外壳顶掉。现在 `create_main_window` 同时注册 `on_navigation` 与 `on_new_window`，由 `navigation_disposition()` 统一判定：`http(s)` 非回环地址与 `mailto` 交给系统默认浏览器并取消应用内导航，`tauri://` 外壳文档、`about:blank`/`blob`/`data` 以及回环 host（`localhost`/`*.localhost`/`127.0.0.0/8`/`::1`，Harness 端口每次启动都变，所以按 host 判断）继续在应用内导航；`127.0.0.1.example.com` 这类域名不算回环。打开失败时 emit `link-open-failed`，外壳弹 toast（`toast.linkFailed`）。已用一次性 wry 探针实测（真实 cliclick 点击）：`target="_blank"` 与同帧外链都只触发一次 `on_navigation`（`External`），导航被取消、页面不被顶掉，且**不会**再触发 `on_new_window`，因此不存在重复打开；`on_new_window` 仅作为 `window.open` 与链接右键菜单的兜底。回归测试：Rust 侧 `navigation_disposition`/`is_loopback_host`/`is_openable_web_url` 单测，前端 `tests/external-links.test.mjs`。
- 历史正式版本 `v0.2.11` 至 `v0.2.17` 均以认证签名、公证、staple 和 `spctl` 校验为准；历史 Release 链接和 digest 以 GitHub 记录为准，不以单次 Actions 状态推断公证结果。
- `v0.3.42`：设置 → 更新 增加“内置 dsh 更新通道”，可以检测并安装 npm 上的 beta/alpha 预览版。npm 上 `@deepseek-ai/dsh` 没有 `beta` dist-tag，预览流走 `alpha`（当前 `0.1.6-alpha.2`），普通流是 `latest`/`next`（当前 `0.1.5-rc.2`）；后端原来只读 `latest`/`next`，所以永远看不到预览版。现在 `check_dsh_update` 接受 `channel`（`stable`/`preview`，未知值一律回落 `stable`），`preview` 时把 `alpha`/`beta` 一起纳入比较；`NpmDistTags` 增加这两个字段，选择逻辑抽成 `newest_of_published_versions`。前端新增 `src/dsh-channel.js`（通道归一化、预览版识别、本地化状态句与“npm 上已有预览版 X”提示），设置页给出通道下拉、预览版标记与提示行。因为启动器永远选“版本号最高”的 managed 目录，装了预览版后即使切回稳定版也仍然生效，所以同时提供 `rollback_dsh_preview`：停 Harness、删除 runtime 下 `alpha`/`beta` 目录（只删受管 runtime 直属子目录且版本号通过校验）、刷新 `~/.local/bin/dsh` wrapper、按需重启。测试：Rust `newest_of_published_versions`/`update_channel_defaults_to_stable`/`preview_versions_are_recognized_by_prerelease_label`，前端 `tests/dsh-channel.test.mjs` 与跨语言契约 `tests/dsh-channel-wiring.test.mjs`。
- `v0.3.43`：内置 dsh 支持切换与降级。v0.3.42 只能“往前更新”或整目录删掉预览版——启动器永远取版本号最高的 managed 目录，装了新版就无法退回旧版。现在 `<runtime>/dsh/.active-version` 记录用户固定的版本号（只接受数字开头的合法版本串，损坏则忽略回落最新版），`select_active_dsh_version` 让固定版本优先、不可用时自动回落，`active_dsh_version_path` 同时驱动启动候选、`runtime_status.version` 与 `~/.local/bin/dsh` wrapper。新增 `list_dsh_versions`（npm 上全部版本 + 本地已安装标记，npm 不可达时退化为只列已安装）、`set_dsh_version`（已安装则直接固定并重启，未安装则先下载再固定）、`follow_latest_dsh_version`（清除固定）；`install_dsh_update` 改为安装后清除固定（“更新”即前进）。原先删目录的 `rollback_dsh_preview` 已移除，“回退到稳定版”改为把版本固定到后台返回的 `stable_version`，预览版仍留在磁盘上可随时切回。设置页新增「内置 dsh 版本」选择器（标记 预览/当前/已安装/未安装·需下载）与「跟随最新版」按钮；固定期间 `shouldAutoInstallDshUpdate` 会阻止自动安装覆盖用户选择。测试：Rust `pinned_version_only_accepts_a_plain_version`/`active_version_prefers_the_pin_over_the_newest_build`/`active_version_ignores_unusable_pins_and_builds`/`published_versions_keep_the_two_streams_apart`，前端 `tests/dsh-channel.test.mjs`（状态句、选项标签、固定提示、自动安装抑制）与契约测试 `tests/dsh-channel-wiring.test.mjs`（新命令注册、启动器走 pin、旧命令不再被调用）。
- `v0.3.44`：修正 v0.3.43 的一个边角问题与测试覆盖。装 dsh 预览版后外壳把“刚装上的版本”写进了 `stable_version`，于是“回退到稳定版”按钮会去固定预览版本身却提示已回退；现在 `stable_version` 只由后台检查结果提供，始终保持 npm 上最新的稳定版本。另加 Rust 测试 `npm_metadata_yields_tags_and_every_published_version`：用真实形状的 npm 元数据（含庞大的逐版本 manifest）验证 `versions` 能解析进占位结构并同时拿到 `latest`/`next`/`alpha` 两条流。
- `v0.3.45`：Dock 图标样式新增“头像”，并修复退出 App 后图标失效。原先只调用 `setApplicationIconImage`，那只影响运行中的 Dock tile；macOS 在 App 未运行时会读回 App 包内的图标，所以退出后又变回默认蓝色。现在 `DockIconVariant` 统一管理 blue / black / avatar 三套 1024 资源，`set_dock_icon_variant` 在主线程同时刷新运行中的 tile，并通过 `NSWorkspace.setIcon(_:forFile:options:)` 把所选样式写成 App 包的自定义图标（`Icon\r` + FinderInfo，位于 `Contents/` 之外，不进入资源封条，`codesign --verify --deep` 仍通过、`spctl` 仍判为 Notarized Developer ID）；选择 blue 时清除该自定义图标，回落到包内品牌资源。命令返回 `{applied, persisted}`，写入失败（例如 App 在只读卷上）时外壳提示“退出后会回到默认图标”。新增资源 `Assets/DeepSeekHarnessIcon-Avatar-Source.jpg` 与 `Assets/DeepSeekHarnessIcon-Avatar-Prepared-1024.png`，由 `scripts/make_prepared_dock_icon.swift` 按 846/1024 安全区与 0.22 圆角生成；前端新增 `src/dock-icon.js`（样式归一化、存储、文案键、临时生效判定）。测试：Rust `dock_icon_variants_match_the_shell_values` / `unknown_dock_icon_variants_are_rejected` / `only_the_blue_dock_icon_uses_the_bundled_artwork` / `dock_icon_persistence_writes_and_clears_the_bundle_icon`（在临时目录真实写入并清除 `Icon\r`），前端 `tests/dock-icon.test.mjs` 与跨语言契约 `tests/dock-icon-wiring.test.mjs`。
- `v0.3.46`：让 v0.3.45 的 Dock 图标样式在真实安装路径上真正生效。v0.3.45 只把样式写进 App 包（`NSWorkspace.setIcon`），但 Dock 对固定 App 的图标另有缓存，只有 Dock 进程重启才会重读包内图标，所以退出后固定图标仍可能是旧样式；现在写成功后 `killall Dock`（`restart_macos_dock`），设置页文案同步说明“切换时会短暂重启 Dock”。同时实测确认 `/Applications` 下的 App 包对应用本身完全不可写：包内新建文件与 `touch Icon\r` 都返回 `operation not permitted`，而 `NSWorkspace` 的图标写入由系统图标服务代做所以能成功（已安装的 0.3.45 包里 `Icon\r` 有 583KB resource fork、data fork 为 0 字节）。因此“已应用样式 + 版本”的标记不能放在 `Icon\r` 的 data fork，改为 `DockIconRecord` 落在 `~/Library/Application Support/com.deepseek.harnessdesk/dock-icon.json`，按 bundle 路径 + 版本号判定，App 更新后版本号变化会自动重写。是否已经就位不再看 `Icon\r` 是否存在，而是读它的 resource fork（`bundle_custom_icon_present`：图标像素只在 resource fork，空文件不算图标）；`persisted` 回到只由 `NSWorkspace` 返回值决定，不再因包内写标记失败而误报“退出后会回到默认图标”。测试：Rust `only_icon_data_counts_as_a_custom_bundle_icon` / `dock_icon_already_applied_only_skips_a_matching_bundle` / `dock_icon_record_round_trips_and_tolerates_a_missing_one`，`dock_icon_persistence_writes_and_clears_the_bundle_icon` 改为断言图标数据状态；契约 `tests/dock-icon-wiring.test.mjs` 新增“标记不得写在包内”“写成功必须重启 Dock”“记录必须落在 app_data_dir”三条约束。

- `v0.3.47`：修现代 macOS 上完全收不到系统通知。两个独立原因叠加：`tauri-plugin-notification` 依赖 `mac-notification-sys`，后者走 Apple 已移除的 `NSUserNotification`，在现代系统上发出去的通知被静默丢弃（本机用 `scripts/make_notification_probe.sh` 构建 `scripts/notification_probe.swift` 实投两种投递方式验证过：旧 API 已不在、新 API 可用）；dsh 0.1.2+ 的 `/api/remote.mux` 是被动多路复用器，不发一帧 `{"endpoint":"$events","streamId":"desk-events"}` 订阅报文就永远静默，而 v0.3.33 记录的“迁移到 remote.mux 并回退旧双端点”其实只做到了连接与回退，`$events` 订阅从未真正发出过，所以那次迁移并没有把提醒修好。现在 macOS 直接经 `objc2-user-notifications` 的 `UNUserNotificationCenter` 投递（`mod macos_notification`：进程内一次 `ensure_authorized`、固定 request id 让同一条待办不堆叠、`addNotificationRequest` 的 completion 必须等到才返回、`authorization_status` 按需读），插件只保留给 Windows/Linux；`catch()` 包住 `currentNotificationCenter`，因为 `tauri dev` 下没有 bundle 时该调用抛 ObjC 异常（`objc2` 因此开 `exception` feature）。投递失败/未授权不再假装送达：写日志页并在“设置 → 高级 → 通知提醒”显示系统权限提示（`notification_permission` + `shouldShowNotificationPermissionHint`），提供 `open_notification_settings`（`x-apple.systempreferences:com.apple.Notifications-Settings.extension`）与 `send_test_notification`（不受分类开关约束，用来验证链路并触发 macOS 首次授权弹窗）。报文分类抽成纯函数：`classify_mux_message` / `classify_legacy_mux_frame` / `classify_legacy_host_frame` 归一到 `HarnessInbound`，兼容 dsh 两代事件名（`user-questions/request`、`question/requested`、`approval/request(ed)`、`api-session/status`、`host/session-status`）；去重与 running 会话状态移到重连循环之外。测试：Rust `mux_*` / `legacy_*` / `interaction_*`（10 个），前端 `tests/notification-permission.test.mjs`、契约 `tests/notification-wiring.test.mjs`（open 帧必须发出、分类层存在、不得再用插件直投、三个命令都注册、权限提示有落点）。
- `v0.3.48`：修 v0.3.47 里“启动时没来得及点授权弹窗，之后这一整轮都不会再有通知”。`requestAuthorizationWithOptions` 只在用户回答后才回调，启动线程 5 秒 `AUTH_TIMEOUT` 到点时拿到的只是“还没人回答”，但 `AUTHORIZED: OnceLock<bool>` 用 `get_or_init` 把这个 `false` 永久缓存了，于是 `ensure_authorized` 在进程剩余时间里一律返回未授权：`post` 直接短路成 `PostOutcome::NotAuthorized`，连测试通知按钮也无效，而设置页的权限提示走的是另一个实时读取的 `authorization_status`，会显示“已开启”却什么都不发——弹窗常被窗口挡住、几十秒后才点“允许”是常态，所以这条路径几乎每次首启都会踩。现在只缓存“确实已授权”（`GRANTED: OnceLock<()>`），其余情况每次都回读 `authorization_status()`：已决定未授权 → `Denied`；仍未决定 → 由 `PROMPTED: AtomicBool` 保证只发一次弹窗请求，超时不缓存、下次继续回读（因此系统设置里改回来也不需要重启，`send_test_notification` 会 `ask_for_authorization_again()` 主动重发弹窗，误关弹窗可补救）。契约 `tests/notification-wiring.test.mjs` 新增三条：不得把未回答的授权请求当拒绝缓存（禁 `AUTHORIZED.get_or_init`）、授权前必须实时回读状态、测试按钮要能重发弹窗。

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
