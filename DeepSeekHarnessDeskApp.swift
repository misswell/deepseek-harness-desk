import SwiftUI

@main
struct DeepSeekHarnessDeskApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var appState = AppState()

    var body: some Scene {
        WindowGroup("DeepSeek Harness Desk") {
            MainView()
                .environmentObject(appState)
                .environmentObject(appState.harnessManager)
                .environmentObject(appState.webViewController)
                .onAppear {
                    appDelegate.register(harnessManager: appState.harnessManager)
                }
                .task {
                    await appState.startIfNeeded()
                }
        }
        .commands {
            CommandMenu("View") {
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
        }

        Settings {
            SettingsView()
                .environmentObject(appState)
                .environmentObject(appState.harnessManager)
        }
    }
}
