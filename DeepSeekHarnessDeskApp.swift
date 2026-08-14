import SwiftUI

@main
struct DeepSeekHarnessDeskApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var appState = AppState()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            MainView()
                .environmentObject(appState)
                .environmentObject(appDelegate)
                .environmentObject(appState.harnessManager)
                .environmentObject(appState.runtimeManager)
                .environmentObject(appState.webViewController)
                .environmentObject(appState.updateManager)
                .onAppear {
                    appDelegate.register(
                        harnessManager: appState.harnessManager,
                        webViewController: appState.webViewController
                    )
                    appState.restoreWindowContent()
                }
                .onChange(of: scenePhase) { _, newPhase in
                    if newPhase == .active {
                        appState.restoreWindowContent()
                    }
                }
                .task {
                    appState.startIfNeeded()
                }
                .background(WindowChromeConfigurator())
        }
        .windowStyle(.hiddenTitleBar)
        .commands {
            CommandMenu("View") {
                Button("放大界面") {
                    appState.webViewController.zoomIn()
                }
                .keyboardShortcut("+", modifiers: [.command])

                Button("缩小界面") {
                    appState.webViewController.zoomOut()
                }
                .keyboardShortcut("-", modifiers: [.command])

                Button("恢复默认大小") {
                    appState.webViewController.resetZoom()
                }
                .keyboardShortcut("0", modifiers: [.command])

                Divider()

                Button("Reload") {
                    appState.webViewController.reload()
                }
                .keyboardShortcut("r", modifiers: [.command])

                Button("Back") {
                    appState.webViewController.goBack()
                }
                .keyboardShortcut("[", modifiers: [.command])

                Button("Forward") {
                    appState.webViewController.goForward()
                }
                .keyboardShortcut("]", modifiers: [.command])
            }

            CommandMenu("Harness") {
                Button("Start Harness") {
                    Task { await appState.harnessManager.start() }
                }
                .disabled(appState.harnessManager.state.isBusy)

                Button("Restart Harness") {
                    Task { await appState.harnessManager.restart() }
                }
                .disabled(appState.harnessManager.state.isBusy)

                Button("Stop Harness") {
                    Task { await appState.harnessManager.stop() }
                }
                .disabled(!appState.harnessManager.hasRunningProcess)

                Divider()

                Button("Open Harness Logs") {
                    appState.openLogs()
                }
            }

            CommandMenu("帮助") {
                Button("检查 App 更新…") {
                    Task { await appState.updateManager.checkForUpdates() }
                }
                .disabled(appState.updateManager.isChecking || appState.updateManager.isInstalling)

                Button("检查内置 dsh 更新…") {
                    Task { await appState.runtimeManager.checkForUpdates() }
                }
                .disabled(
                    appState.runtimeManager.isCheckingForUpdates ||
                    appState.runtimeManager.isUpdating ||
                    appState.runtimeManager.isInstalling
                )
            }
        }

        Settings {
            SettingsView()
                .environmentObject(appState)
                .environmentObject(appDelegate)
                .environmentObject(appState.harnessManager)
                .environmentObject(appState.runtimeManager)
                .environmentObject(appState.updateManager)
                .background(WindowChromeConfigurator(isMainWindow: false))
        }
        .windowStyle(.hiddenTitleBar)
    }
}
