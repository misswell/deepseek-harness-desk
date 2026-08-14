import SwiftUI

struct SettingsView: View {
    @EnvironmentObject private var appState: AppState
    @EnvironmentObject private var harnessManager: HarnessManager
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Spacer()
                Button("关闭") {
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)
            }
            .padding(.bottom, 8)

            TabView {
                GeneralSettingsView()
                    .tabItem { Label("通用", systemImage: "gear") }

                UpdatesSettingsView()
                    .tabItem { Label("更新", systemImage: "arrow.triangle.2.circlepath") }

                HarnessSettingsView()
                    .tabItem { Label("Harness", systemImage: "shippingbox") }

                AdvancedSettingsView()
                    .tabItem { Label("高级", systemImage: "slider.horizontal.3") }

                AboutSettingsView()
                    .tabItem { Label("关于", systemImage: "info.circle") }
            }
        }
        .padding(20)
        .frame(width: 640, height: 540)
    }
}

private struct UpdatesSettingsView: View {
    @EnvironmentObject private var updateManager: UpdateManager
    @EnvironmentObject private var runtimeManager: RuntimeManager

    var body: some View {
        Form {
            Section("App 壳子") {
                LabeledContent("当前版本", value: UpdateManager.currentVersion)
                LabeledContent("最新版本", value: updateManager.latestReleaseVersion ?? "未检查")
                LabeledContent("状态", value: updateManager.status.isEmpty ? "未检查" : updateManager.status)
                Picker(
                    "自动检查频率",
                    selection: Binding(
                        get: { updateManager.automaticCheckInterval },
                        set: { updateManager.setAutomaticCheckInterval($0) }
                    )
                ) {
                    ForEach(UpdateManager.AutomaticCheckInterval.allCases) { interval in
                        Text(interval.title).tag(interval)
                    }
                }
                .disabled(!updateManager.automaticallyChecksForUpdates)
                Text("后台静默检查；发现新版本后会在主界面右上角提示。")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                if updateManager.showsInstallProgress {
                    InstallationProgressCard(
                        title: "App 更新",
                        step: updateManager.installStep,
                        progress: updateManager.installProgress,
                        progressLabel: updateManager.installProgressLabel,
                        logs: updateManager.installLogs,
                        isActive: updateManager.isInstalling,
                        hasError: updateManager.status.contains("失败"),
                        onClose: { updateManager.dismissInstallProgress() }
                    )
                    .frame(maxHeight: 240)
                }
                Toggle(
                    "自动检查 App 更新",
                    isOn: Binding(
                        get: { updateManager.automaticallyChecksForUpdates },
                        set: { updateManager.setAutomaticChecks($0) }
                    )
                )
                Button("检查 App 更新…") {
                    Task { await updateManager.checkForUpdates() }
                }
                .disabled(updateManager.isChecking || updateManager.isInstalling)
            }

            Section("内置 DeepSeek Harness / dsh") {
                LabeledContent("当前版本", value: runtimeManager.isUsingManagedRuntime ? runtimeManager.managedHarnessVersion : "未使用内置 Runtime")
                LabeledContent("最新版本", value: runtimeManager.latestHarnessVersion ?? "未检查")
                LabeledContent("状态", value: runtimeManager.updateStatus.isEmpty ? "未检查" : runtimeManager.updateStatus)
                if runtimeManager.showsInstallProgress {
                    InstallationProgressCard(
                        title: "内置 Runtime",
                        step: runtimeManager.installStep,
                        progress: runtimeManager.installProgress,
                        progressLabel: runtimeManager.installProgressLabel,
                        logs: runtimeManager.installLogs,
                        isActive: runtimeManager.isInstalling,
                        hasError: {
                            if case .failed = runtimeManager.installState { return true }
                            return false
                        }(),
                        onClose: { runtimeManager.dismissInstallProgress() }
                    )
                    .frame(maxHeight: 240)
                }
                Toggle(
                    "自动检查内置 dsh 更新",
                    isOn: Binding(
                        get: { runtimeManager.automaticallyChecksForUpdates },
                        set: { runtimeManager.setAutomaticChecks($0) }
                    )
                )
                Toggle(
                    "自动安装内置 dsh 更新",
                    isOn: Binding(
                        get: { runtimeManager.automaticallyInstallsUpdates },
                        set: { runtimeManager.setAutomaticInstallation($0) }
                    )
                )
                Button("检查内置 dsh 更新…") {
                    Task { await runtimeManager.checkForUpdates() }
                }
                .disabled(
                    runtimeManager.isCheckingForUpdates ||
                    runtimeManager.isUpdating ||
                    runtimeManager.isInstalling
                )
            }
        }
        .formStyle(.grouped)
    }
}

private struct GeneralSettingsView: View {
    @EnvironmentObject private var appDelegate: AppDelegate
    @AppStorage("launchAtLogin") private var launchAtLogin = false
    @AppStorage("restoreLastWindow") private var restoreLastWindow = true

    var body: some View {
        Form {
            Toggle("登录时启动", isOn: $launchAtLogin)
            Toggle("恢复上次窗口", isOn: $restoreLastWindow)
            Toggle(
                "在 Dock 中显示图标",
                isOn: Binding(
                    get: { appDelegate.showsDockIcon },
                    set: { appDelegate.setShowsDockIcon($0) }
                )
            )
            Text("关闭后应用会继续运行，并通过菜单栏图标打开。")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
        .formStyle(.grouped)
    }
}

private struct HarnessSettingsView: View {
    @EnvironmentObject private var harnessManager: HarnessManager

    var body: some View {
        Form {
            LabeledContent("状态", value: harnessManager.state.title)
            LabeledContent("版本", value: harnessManager.runtimeVersion)
            LabeledContent("PID", value: harnessManager.pid.map(String.init) ?? "—")
            LabeledContent("端口", value: harnessManager.port.map(String.init) ?? "—")

            HStack {
                Button("重启 Harness") {
                    Task { await harnessManager.restart() }
                }
                Button("停止 Harness") {
                    Task { await harnessManager.stop() }
                }
                .disabled(!harnessManager.hasRunningProcess)
            }
        }
        .formStyle(.grouped)
    }
}

private struct AdvancedSettingsView: View {
    var body: some View {
        Form {
            LabeledContent("Runtime 目录", value: PathUtils.applicationSupportDirectory.path)
            LabeledContent("日志目录", value: PathUtils.logsDirectory.path)

            Button("打开日志目录") {
                NSWorkspace.shared.open(PathUtils.logsDirectory)
            }
            Button("打开 Runtime 目录") {
                try? FileManager.default.createDirectory(
                    at: PathUtils.applicationSupportDirectory,
                    withIntermediateDirectories: true
                )
                NSWorkspace.shared.open(PathUtils.applicationSupportDirectory)
            }
        }
        .formStyle(.grouped)
    }
}

private struct AboutSettingsView: View {
    var body: some View {
        VStack(spacing: 10) {
            Image(nsImage: NSApplication.shared.applicationIconImage)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(width: 72, height: 72)
            Text("DeepSeek Harness Desk")
                .font(.title2)
                .bold()
            Text("原生 macOS DeepSeek Harness 桌面客户端")
                .foregroundStyle(.secondary)
            Text("DeepSeek Harness 由 DeepSeek AI 开发。DeepSeek Harness Desk 是独立的第三方项目，不隶属于 DeepSeek AI，也未获得其认可或赞助。")
                .font(.footnote)
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 420)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
