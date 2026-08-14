import AppKit
import SwiftUI

fileprivate enum WindowChromeMetrics {
    static let titlebarHeight: CGFloat = 36
    // Leave enough room for the three AppKit traffic-light buttons and a
    // small hit slop so the drag layer cannot steal their clicks.
    static let trafficLightReservedWidth: CGFloat = 92
}

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

/// A native strip attached directly to the window content view. Keeping this
/// outside SwiftUI/WKWebView is important: WebKit otherwise consumes the
/// mouse-down before AppKit can start a window drag.
final class WindowDragOverlayView: NSView {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool {
        true
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        bounds.contains(point) ? self : nil
    }

    override func mouseDown(with event: NSEvent) {
        window?.performDrag(with: event)
    }
}

final class WindowChromeView: NSView {
    private let isMainWindow: Bool
    private var windowDragOverlay: WindowDragOverlayView?

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
        // Keep the native `.titled` mask so AppKit supplies adaptive rounded
        // corners. Full-size content lets the WebView extend beneath the
        // hidden title-bar area without turning the window into a borderless
        // rectangle.
        if isMainWindow {
            window.styleMask.formUnion([.titled, .closable, .miniaturizable, .resizable])
        }
        window.styleMask.insert(.fullSizeContentView)
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.titlebarSeparatorStyle = .none
        for buttonType: NSWindow.ButtonType in [.closeButton, .miniaturizeButton, .zoomButton] {
            window.standardWindowButton(buttonType)?.isHidden = !isMainWindow
        }
        if !isMainWindow {
            window.title = "设置"
            window.standardWindowButton(.closeButton)?.isHidden = false
        }
        window.isMovableByWindowBackground = true

        if isMainWindow {
            installWindowDragOverlay(in: window)
        }
    }

    private func installWindowDragOverlay(in window: NSWindow) {
        guard let contentView = window.contentView else { return }

        let overlay = windowDragOverlay ?? WindowDragOverlayView()
        windowDragOverlay = overlay
        if overlay.superview !== contentView {
            overlay.removeFromSuperview()
            contentView.addSubview(overlay, positioned: .above, relativeTo: nil)
        }

        // NSView coordinates start at the bottom-left. Keep this strip pinned
        // to the top edge while allowing it to follow window resizing.
        overlay.autoresizingMask = [.width, .minYMargin]
        overlay.frame = NSRect(
            x: WindowChromeMetrics.trafficLightReservedWidth,
            y: max(0, contentView.bounds.height - WindowChromeMetrics.titlebarHeight),
            width: max(0, contentView.bounds.width - WindowChromeMetrics.trafficLightReservedWidth),
            height: min(WindowChromeMetrics.titlebarHeight, contentView.bounds.height)
        )
    }
}

@MainActor
final class ApplicationTerminationCoordinator {
    typealias AsyncAction = @MainActor () async -> Void
    typealias SyncAction = @MainActor () -> Void

    private let gracefulTimeoutNanoseconds: UInt64
    private var terminationInProgress = false
    private var replyIssued = false
    private var gracefulStopTask: Task<Void, Never>?
    private var timeoutTask: Task<Void, Never>?

    init(gracefulTimeout: TimeInterval = 8) {
        gracefulTimeoutNanoseconds = UInt64(
            max(0, gracefulTimeout) * 1_000_000_000
        )
    }

    func requestTermination(
        hasRunningProcess: Bool,
        stop: @escaping AsyncAction,
        forceStop: @escaping SyncAction,
        reply: @escaping SyncAction
    ) -> NSApplication.TerminateReply {
        guard !replyIssued else {
            return .terminateLater
        }
        guard hasRunningProcess else {
            return .terminateNow
        }
        guard !terminationInProgress else {
            return .terminateLater
        }

        terminationInProgress = true
        gracefulStopTask = Task { @MainActor [weak self] in
            await stop()
            guard let self, self.terminationInProgress else { return }
            self.finish(reply: reply)
        }
        timeoutTask = Task { @MainActor [weak self, forceStop, reply] in
            do {
                try await Task.sleep(nanoseconds: self?.gracefulTimeoutNanoseconds ?? 0)
            } catch {
                return
            }
            guard !Task.isCancelled, let self else { return }
            guard self.terminationInProgress else { return }

            self.gracefulStopTask?.cancel()
            self.gracefulStopTask = nil
            forceStop()
            self.finish(reply: reply)
        }
        return .terminateLater
    }

    private func finish(reply: @escaping SyncAction) {
        guard terminationInProgress else { return }
        terminationInProgress = false
        replyIssued = true
        timeoutTask?.cancel()
        timeoutTask = nil
        gracefulStopTask = nil
        reply()
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, ObservableObject {
    static let dockIconPreferenceKey = "showDockIcon"
    static let openMainWindowNotification = Notification.Name("com.deepseek-harness-desk.open-main-window")

    private weak var harnessManager: HarnessManager?
    private weak var webViewController: WebViewController?
    private var mainWindow: NSWindow?
    private var statusItem: NSStatusItem?
    private var statusMenu: NSMenu?
    private weak var dockIconMenuItem: NSMenuItem?
    private var windowDragEventMonitor: Any?
    private let terminationCoordinator = ApplicationTerminationCoordinator()
    private var terminatingForUpdate = false

    @Published private(set) var showsDockIcon: Bool

    override init() {
        if UserDefaults.standard.object(forKey: Self.dockIconPreferenceKey) == nil {
            UserDefaults.standard.set(true, forKey: Self.dockIconPreferenceKey)
        }
        showsDockIcon = UserDefaults.standard.bool(forKey: Self.dockIconPreferenceKey)
        super.init()
    }

    deinit {
        if let windowDragEventMonitor {
            NSEvent.removeMonitor(windowDragEventMonitor)
        }
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        configureStatusItem()
        applyDockIconPolicy()
        installWindowDragEventMonitor()
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

    /// Prepares the App for the update replacement path.
    ///
    /// The updater launches an external replacement script and then asks the
    /// App to terminate. The normal graceful termination coordinator returns
    /// `.terminateLater` and waits for a main-actor reply, but that reply can
    /// never run because AppKit blocks the main thread inside the termination
    /// event loop. For updates we bypass the coordinator entirely: stop the
    /// Harness immediately and let `applicationShouldTerminate` answer
    /// `.terminateNow`.
    func prepareForUpdateTermination() {
        terminatingForUpdate = true
        harnessManager?.forceStopImmediately()
    }

    private func installWindowDragEventMonitor() {
        guard windowDragEventMonitor == nil else { return }

        windowDragEventMonitor = NSEvent.addLocalMonitorForEvents(
            matching: [.leftMouseDown]
        ) { [weak self] event in
            guard let self,
                  let window = event.window,
                  window === self.mainWindow,
                  window.styleMask.contains(.titled),
                  window.isMovable else {
                return event
            }

            guard let contentView = window.contentView else { return event }
            let point = contentView.convert(event.locationInWindow, from: nil)
            let distanceFromTop = contentView.bounds.maxY - point.y
            guard distanceFromTop >= 0,
                  distanceFromTop <= WindowChromeMetrics.titlebarHeight,
                  point.x >= WindowChromeMetrics.trafficLightReservedWidth else {
                return event
            }

            window.performDrag(with: event)
            return nil
        }
    }

    func setShowsDockIcon(_ visible: Bool) {
        guard showsDockIcon != visible else { return }
        showsDockIcon = visible
        UserDefaults.standard.set(visible, forKey: Self.dockIconPreferenceKey)
        applyDockIconPolicy()
        updateStatusMenuState()
    }

    func openMainWindow(forceReload: Bool = true) {
        guard let window = mainWindow, NSApp.windows.contains(window) else {
            // The SwiftUI main window is gone (usually closed). Ask SwiftUI to
            // recreate it instead of retrying forever on a stale reference.
            NotificationCenter.default.post(name: Self.openMainWindowNotification, object: nil)
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
        hasVisibleWindows _: Bool
    ) -> Bool {
        // `hasVisibleWindows` also counts the Settings scene. A Dock click
        // must still bring the registered main window forward when Settings
        // is the only visible window.
        openMainWindow(forceReload: mainWindow?.isVisible != true)
        return false
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if terminatingForUpdate {
            return .terminateNow
        }
        return terminationCoordinator.requestTermination(
            hasRunningProcess: harnessManager?.hasRunningProcess == true,
            stop: { @MainActor [weak self] in
                await self?.harnessManager?.stop()
            },
            forceStop: { [weak self] in
                self?.harnessManager?.forceStopImmediately()
            },
            reply: {
                NSApp.reply(toApplicationShouldTerminate: true)
            }
        )
    }

    private func configureStatusItem() {
        guard statusItem == nil else { return }

        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem = item
        guard let button = item.button else { return }

        let icon: NSImage
        if let statusIcon = NSImage(named: "StatusBarIcon") {
            icon = statusIcon
        } else if let fallbackIcon = NSImage(
            systemSymbolName: "shippingbox.fill",
            accessibilityDescription: "DeepSeek Harness Desk"
        ) {
            icon = fallbackIcon
        } else {
            icon = NSApp.applicationIconImage
        }
        icon.size = NSSize(width: 18, height: 18)
        icon.isTemplate = true
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
