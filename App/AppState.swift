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
        self.harnessManager = HarnessManager(
            runtimeManager: runtimeManager,
            logger: logManager
        )
        self.webViewController = WebViewController()
        self.updateManager = UpdateManager()
    }

    func startIfNeeded() async {
        guard !hasStarted else { return }
        hasStarted = true
        updateManager.start()
        await harnessManager.start()
    }

    func openLogs() {
        NSWorkspace.shared.open(logManager.logsDirectory)
    }
}
