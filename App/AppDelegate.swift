import AppKit
import SwiftUI

struct WindowChromeConfigurator: NSViewRepresentable {
    func makeNSView(context: Context) -> WindowChromeView {
        WindowChromeView()
    }

    func updateNSView(_ nsView: WindowChromeView, context: Context) {
        nsView.configureWindow()
    }
}

final class WindowChromeView: NSView {
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        configureWindow()
    }

    func configureWindow() {
        guard let window else { return }

        window.title = ""
        window.toolbar = nil
        window.styleMask.remove(.titled)
        window.styleMask.remove(.fullSizeContentView)
        window.styleMask.insert(.resizable)
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.titlebarSeparatorStyle = .none
        for buttonType: NSWindow.ButtonType in [
            .closeButton,
            .miniaturizeButton,
            .zoomButton
        ] {
            window.standardWindowButton(buttonType)?.isHidden = true
        }
        window.isMovableByWindowBackground = true
    }
}

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
