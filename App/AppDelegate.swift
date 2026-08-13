import AppKit
import SwiftUI

struct WindowChromeConfigurator: NSViewRepresentable {
    let isMainWindow: Bool

    init(isMainWindow: Bool = true) {
        self.isMainWindow = isMainWindow
    }

    func makeNSView(context: Context) -> WindowChromeView {
        WindowChromeView(isMainWindow: isMainWindow)
    }

    func updateNSView(_ nsView: WindowChromeView, context: Context) {
        nsView.configureWindow()
    }
}

final class WindowChromeView: NSView {
    private let isMainWindow: Bool

    init(isMainWindow: Bool) {
        self.isMainWindow = isMainWindow
        super.init(frame: .zero)
    }

    required init?(coder: NSCoder) {
        self.isMainWindow = true
        super.init(coder: coder)
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        configureWindow()
    }

    func configureWindow() {
        guard let window else { return }

        if isMainWindow {
            (NSApp.delegate as? AppDelegate)?.register(mainWindow: window)
        }

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
    private weak var webViewController: WebViewController?
    private var mainWindow: NSWindow?

    func register(
        harnessManager: HarnessManager,
        webViewController: WebViewController
    ) {
        self.harnessManager = harnessManager
        self.webViewController = webViewController
    }

    func register(mainWindow: NSWindow) {
        self.mainWindow = mainWindow
        mainWindow.isReleasedWhenClosed = false
    }

    func applicationShouldHandleReopen(
        _ sender: NSApplication,
        hasVisibleWindows flag: Bool
    ) -> Bool {
        if !flag {
            let window = mainWindow ?? sender.windows.first(where: { $0.canBecomeKey })
            if let window {
                window.makeKeyAndOrderFront(sender)
                webViewController?.restore(
                    url: harnessManager?.serverURL,
                    forceReload: true
                )
                NSApp.activate(ignoringOtherApps: true)
                return false
            }
        }
        return true
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
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
