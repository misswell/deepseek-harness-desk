import AppKit
import SwiftUI

fileprivate enum WindowChromeMetrics {
    static let titlebarHeight: CGFloat = 44
    // Leave enough room for the three AppKit traffic-light buttons. The
    // event monitor and the view overlay use the same boundary.
    static let trafficLightReservedWidth: CGFloat = 92
}

/// Geometry for the window's top drag strip.
///
/// The window content view of a SwiftUI scene is a flipped `NSHostingView`
/// (origin at the top-left, y grows downward), while a plain AppKit content
/// view measures from the bottom-left. All strip math must honor `isFlipped`,
/// otherwise the strip ends up at the bottom of the window.
enum WindowDragStrip {
    static func frame(in contentView: NSView) -> NSRect {
        let y = contentView.isFlipped
            ? 0
            : contentView.bounds.maxY - WindowChromeMetrics.titlebarHeight
        return NSRect(
            x: WindowChromeMetrics.trafficLightReservedWidth,
            y: y,
            width: max(0, contentView.bounds.width - WindowChromeMetrics.trafficLightReservedWidth),
            height: min(WindowChromeMetrics.titlebarHeight, contentView.bounds.height)
        )
    }

    /// Whether a point (already in the content view's coordinate space) lies
    /// inside the drag strip.
    static func contains(point: NSPoint, in contentView: NSView) -> Bool {
        let distanceFromTop = contentView.isFlipped
            ? point.y
            : contentView.bounds.maxY - point.y
        return distanceFromTop >= 0 &&
            distanceFromTop <= WindowChromeMetrics.titlebarHeight &&
            point.x >= WindowChromeMetrics.trafficLightReservedWidth
    }

    /// Keeps the strip pinned to the top edge as the window resizes: the
    /// margin opposite the pinned edge flexes.
    static func autoresizingMask(for contentView: NSView) -> NSView.AutoresizingMask {
        contentView.isFlipped ? [.width, .maxYMargin] : [.width, .minYMargin]
    }
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
    override var mouseDownCanMoveWindow: Bool {
        true
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool {
        true
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        // The frame already leaves the leading traffic-light region free, so
        // simply answering for the whole strip never steals those buttons.
        bounds.contains(point) ? self : nil
    }

    override func mouseDown(with event: NSEvent) {
        guard event.clickCount == 1 else {
            super.mouseDown(with: event)
            return
        }
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

    override func layout() {
        super.layout()
        if let window {
            installWindowDragOverlay(in: window)
        }
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
        window.styleMask.insert(.titled)
        window.styleMask.insert(.fullSizeContentView)
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.titlebarSeparatorStyle = .none
        window.isOpaque = false
        window.backgroundColor = .clear
        for buttonType: NSWindow.ButtonType in [.closeButton, .miniaturizeButton, .zoomButton] {
            window.standardWindowButton(buttonType)?.isHidden = !isMainWindow
        }
        if !isMainWindow {
            window.title = "设置"
            window.standardWindowButton(.closeButton)?.isHidden = false
        }
        window.isMovable = true
        window.isMovableByWindowBackground = true

        installWindowDragOverlay(in: window)
        if isMainWindow {
            if let closeButton = window.standardWindowButton(.closeButton) {
                closeButton.target = self
                closeButton.action = #selector(hideMainWindowFromCloseButton(_:))
            }
        }
    }

    @objc private func hideMainWindowFromCloseButton(_ sender: Any?) {
        window?.orderOut(nil)
    }

    private func installWindowDragOverlay(in window: NSWindow) {
        guard let contentView = window.contentView else { return }

        let overlay = windowDragOverlay ?? WindowDragOverlayView()
        windowDragOverlay = overlay
        if overlay.superview !== contentView {
            overlay.removeFromSuperview()
            contentView.addSubview(overlay, positioned: .above, relativeTo: nil)
        } else if contentView.subviews.last !== overlay {
            // SwiftUI/WKWebView can insert views after the representable has
            // been attached. Re-adding keeps the drag strip above the web view.
            contentView.addSubview(overlay, positioned: .above, relativeTo: nil)
        }

        updateWindowDragOverlayFrame()
    }

    private func updateWindowDragOverlayFrame() {
        guard let contentView = window?.contentView,
              let overlay = windowDragOverlay else { return }

        // Pin the strip to the top edge while letting it follow window
        // resizing. The window content view is a flipped NSHostingView in the
        // real app, so the top edge is y = 0 there.
        overlay.autoresizingMask = WindowDragStrip.autoresizingMask(for: contentView)
        overlay.frame = WindowDragStrip.frame(in: contentView)
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
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate, ObservableObject {
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
    private static var terminatingForUpdate = false

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
        mainWindow.delegate = self
    }

    /// Hide the main window instead of closing it. Keeping the singleton Window
    /// alive lets Dock and status-item clicks reveal the same window object.
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard sender === mainWindow else { return true }
        sender.orderOut(nil)
        return false
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
        Self.terminatingForUpdate = true
        harnessManager?.forceStopImmediately()
    }

    /// Static entry point used by the updater so it does not depend on the
    /// runtime delegate cast succeeding.
    static func requestUpdateTermination() {
        terminatingForUpdate = true
    }

    static func resetUpdateTerminationForTesting() {
        terminatingForUpdate = false
    }

    deinit {
        if let windowDragEventMonitor {
            NSEvent.removeMonitor(windowDragEventMonitor)
        }
    }

    private func installWindowDragEventMonitor() {
        guard windowDragEventMonitor == nil else { return }

        windowDragEventMonitor = NSEvent.addLocalMonitorForEvents(
            matching: [.leftMouseDown]
        ) { [weak self] event in
            guard let self,
                  let window = event.window,
                  self.isWindowDragStart(event, in: window) else {
                return event
            }

            if event.clickCount == 2 {
                window.zoom(nil)
                return nil
            }

            // Prefer the overlay's native drag path when it is the topmost
            // subview: `mouseDownCanMoveWindow` lets AppKit own the gesture.
            // Fall back to performDrag only when a WebKit/SwiftUI view sits
            // above the strip and would otherwise swallow the click.
            if let contentView = window.contentView,
               contentView.subviews.last is WindowDragOverlayView {
                return event
            }

            window.performDrag(with: event)
            return nil
        }
    }

    private func isWindowDragStart(_ event: NSEvent, in window: NSWindow) -> Bool {
        guard window.isMovable,
              let contentView = window.contentView else {
            return false
        }

        let point = contentView.convert(event.locationInWindow, from: nil)
        return WindowDragStrip.contains(point: point, in: contentView)
    }

    func setShowsDockIcon(_ visible: Bool) {
        guard showsDockIcon != visible else { return }
        let windowToRestore = NSApp.keyWindow ?? mainWindow
        showsDockIcon = visible
        UserDefaults.standard.set(visible, forKey: Self.dockIconPreferenceKey)
        applyDockIconPolicy()
        updateStatusMenuState()

        // Changing from `.regular` to `.accessory` can deactivate the app and
        // hide its current window. Re-activate on the next run-loop turn so
        // switching this preference never strands the UI behind other apps.
        if !visible {
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.activateApplication()
                self.restoreWindowAfterActivationPolicyChange(preferred: windowToRestore)
            }
        }
    }

    func openMainWindow(forceReload: Bool = true) {
        guard let window = mainWindow, NSApp.windows.contains(window) else {
            // The SwiftUI main window is gone (usually closed). Ask SwiftUI to
            // recreate it instead of retrying forever on a stale reference.
            NotificationCenter.default.post(name: Self.openMainWindowNotification, object: nil)
            return
        }

        let shouldReload = forceReload || !window.isVisible
        activateAndBringWindowToFront(window)
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
        if Self.terminatingForUpdate {
            harnessManager?.forceStopImmediately()
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
        activateApplication()
    }

    private func activateApplication() {
        NSApp.unhide(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    /// Brings the main window forward and makes the app the active app.
    ///
    /// Ordering matters in `.accessory` (Dock icon hidden) mode: AppKit can
    /// ignore `NSApp.activate` for a menu-bar app unless the window is already
    /// visible. Show and order the window first, then activate the running
    /// app with the same options Dock clicks use, then make the window key.
    private func activateAndBringWindowToFront(_ window: NSWindow) {
        if window.isMiniaturized {
            window.deminiaturize(nil)
        }
        window.orderFrontRegardless()

        NSApp.unhide(nil)
        NSApp.activate(ignoringOtherApps: true)

        if NSApp.activationPolicy() == .accessory {
            NSRunningApplication.current.activate(
                options: [.activateAllWindows, .activateIgnoringOtherApps]
            )
        }

        window.makeKeyAndOrderFront(nil)
        window.orderFrontRegardless()
    }

    private func restoreWindowAfterActivationPolicyChange(preferred: NSWindow?) {
        let window = [preferred, mainWindow]
            .compactMap { $0 }
            .first { NSApp.windows.contains($0) }
        guard let window else { return }
        activateAndBringWindowToFront(window)
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
