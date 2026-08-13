import Foundation
import SwiftUI

@MainActor
final class AppState: ObservableObject {
    let logManager: LogManager
    let runtimeManager: RuntimeManager
    let harnessManager: HarnessManager
    let webViewController: WebViewController

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
    }

    func startIfNeeded() async {
        guard !hasStarted else { return }
        hasStarted = true
        await harnessManager.start()
    }

    func openLogs() {
        NSWorkspace.shared.open(logManager.logsDirectory)
    }
}
