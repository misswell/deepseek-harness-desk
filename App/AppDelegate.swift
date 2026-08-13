import AppKit

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private weak var harnessManager: HarnessManager?

    func register(harnessManager: HarnessManager) {
        self.harnessManager = harnessManager
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        guard let harnessManager, harnessManager.hasRunningProcess else {
            return .terminateNow
        }

        Task { @MainActor [weak self] in
            await self?.harnessManager?.stop()
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}
