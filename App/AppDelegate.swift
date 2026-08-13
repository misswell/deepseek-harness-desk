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
final class AppDelegate: NSObject, NSApplicationDelegate, ObservableObject {
    static let dockIconPreferenceKey = "showDockIcon"

    private weak var harnessManager: HarnessManager?
    private weak var webViewController: WebViewController?
    private var mainWindow: NSWindow?
    private var statusItem: NSStatusItem?
    private var statusMenu: NSMenu?
    private weak var dockIconMenuItem: NSMenuItem?
    private var pendingWindowOpen = false

    @Published private(set) var showsDockIcon: Bool

    override init() {
        if UserDefaults.standard.object(forKey: Self.dockIconPreferenceKey) == nil {
            UserDefaults.standard.set(true, forKey: Self.dockIconPreferenceKey)
        }
        showsDockIcon = UserDefaults.standard.bool(forKey: Self.dockIconPreferenceKey)
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        configureStatusItem()
        applyDockIconPolicy()
    }

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

    func setShowsDockIcon(_ visible: Bool) {
        guard showsDockIcon != visible else { return }
        showsDockIcon = visible
        UserDefaults.standard.set(visible, forKey: Self.dockIconPreferenceKey)
        applyDockIconPolicy()
        updateStatusMenuState()
    }

    func openMainWindow(forceReload: Bool = true) {
        guard let window = mainWindow ?? NSApp.windows.first(where: { $0.canBecomeKey }) else {
            guard !pendingWindowOpen else { return }
            pendingWindowOpen = true
            DispatchQueue.main.async { [weak self] in
                self?.pendingWindowOpen = false
                self?.openMainWindow(forceReload: forceReload)
            }
            return
        }

        let shouldReload = forceReload || !window.isVisible
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        webViewController?.restore(
            url: harnessManager?.serverURL,
            forceReload: shouldReload
        )
    }

    func applicationShouldHandleReopen(
        _ sender: NSApplication,
        hasVisibleWindows flag: Bool
    ) -> Bool {
        if !flag {
            openMainWindow(forceReload: true)
            return false
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

    private func configureStatusItem() {
        guard statusItem == nil else { return }

        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem = item
        guard let button = item.button else { return }

        let icon = (NSApp.applicationIconImage.copy() as? NSImage) ?? NSImage(
            systemSymbolName: "shippingbox.fill",
            accessibilityDescription: "DeepSeek Harness Desk"
        )
        icon?.size = NSSize(width: 18, height: 18)
        icon?.isTemplate = false
        button.image = icon
        button.imageScaling = .scaleProportionallyDown
        button.toolTip = "打开 DeepSeek Harness Desk"
        button.target = self
        button.action = #selector(handleStatusItemClick(_:))
        button.sendAction(on: [.leftMouseUp, .rightMouseUp])

        let menu = NSMenu()
        let openItem = NSMenuItem(
            title: "打开 DeepSeek Harness Desk",
            action: #selector(openMainWindowFromStatusMenu),
            keyEquivalent: ""
        )
        openItem.target = self
        menu.addItem(openItem)

        let settingsItem = NSMenuItem(
            title: "设置…",
            action: #selector(openSettingsFromStatusMenu),
            keyEquivalent: ","
        )
        settingsItem.target = self
        menu.addItem(settingsItem)
        menu.addItem(.separator())

        let dockIconItem = NSMenuItem(
            title: "在 Dock 中显示图标",
            action: #selector(toggleDockIconFromStatusMenu),
            keyEquivalent: ""
        )
        dockIconItem.target = self
        dockIconMenuItem = dockIconItem
        menu.addItem(dockIconItem)
        menu.addItem(.separator())

        let quitItem = NSMenuItem(
            title: "退出 DeepSeek Harness Desk",
            action: #selector(terminateFromStatusMenu),
            keyEquivalent: "q"
        )
        quitItem.target = self
        menu.addItem(quitItem)

        statusMenu = menu
        updateStatusMenuState()
    }

    private func applyDockIconPolicy() {
        let policy: NSApplication.ActivationPolicy = showsDockIcon ? .regular : .accessory
        guard NSApp.activationPolicy() != policy else { return }
        _ = NSApp.setActivationPolicy(policy)
        if showsDockIcon {
            NSApp.activate(ignoringOtherApps: true)
        }
    }

    private func updateStatusMenuState() {
        dockIconMenuItem?.state = showsDockIcon ? .on : .off
    }

    @objc private func handleStatusItemClick(_ sender: NSStatusBarButton) {
        if NSApp.currentEvent?.type == .rightMouseUp {
            statusMenu?.popUp(
                positioning: nil,
                at: NSPoint(x: sender.bounds.midX, y: sender.bounds.minY),
                in: sender
            )
        } else {
            openMainWindow(forceReload: mainWindow?.isVisible != true)
        }
    }

    @objc private func openMainWindowFromStatusMenu() {
        openMainWindow(forceReload: mainWindow?.isVisible != true)
    }

    @objc private func openSettingsFromStatusMenu() {
        NSApp.sendAction(
            Selector(("showSettingsWindow:")),
            to: nil,
            from: nil
        )
    }

    @objc private func toggleDockIconFromStatusMenu() {
        setShowsDockIcon(!showsDockIcon)
    }

    @objc private func terminateFromStatusMenu() {
        NSApp.terminate(nil)
    }
}
