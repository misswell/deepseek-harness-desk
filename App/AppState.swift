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

    init() {
        let logManager = LogManager()
        let runtimeManager = RuntimeManager()
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
        self.updateManager = UpdateManager()
    }

    func startIfNeeded() async {
        guard !hasStarted else { return }
        hasStarted = true
        updateManager.start()
        await harnessManager.start()
        runtimeManager.startUpdateChecks()
    }

    func openLogs() {
        NSWorkspace.shared.open(logManager.logsDirectory)
    }
}
