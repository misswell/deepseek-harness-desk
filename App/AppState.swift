import Foundation
import SwiftUI

@MainActor
final class AppState: ObservableObject {
    let logManager: LogManager
    let runtimeManager: RuntimeManager
    let harnessManager: HarnessManager
    let webViewController: WebViewController
    let updateManager: UpdateManager

    private var hasStarted = false
    private var startupTask: Task<Void, Never>?

    init() {
        let logManager = LogManager()
        let runtimeManager = RuntimeManager(logger: logManager)
        self.logManager = logManager
        self.runtimeManager = runtimeManager
        let harnessManager = HarnessManager(
            runtimeManager: runtimeManager,
            logger: logManager
        )
        self.harnessManager = harnessManager
        runtimeManager.setHarnessRestartHandler { [weak harnessManager] in
            guard let harnessManager, harnessManager.hasRunningProcess else { return false }
            await harnessManager.restart()
            return true
        }
        self.webViewController = WebViewController()
        self.updateManager = UpdateManager(logger: logManager)
    }

    deinit {
        startupTask?.cancel()
    }

    func startIfNeeded() {
        guard !hasStarted else { return }
        hasStarted = true
        startupTask = Task { @MainActor [weak self] in
            guard let self else { return }
            updateManager.start()
            await harnessManager.start()
            runtimeManager.startUpdateChecks()
            startupTask = nil
        }
    }

    func restoreWindowContent(forceReload: Bool = false) {
        webViewController.restore(
            url: harnessManager.serverURL,
            forceReload: forceReload
        )
    }

    func openLogs() {
        NSWorkspace.shared.open(logManager.logsDirectory)
    }
}
