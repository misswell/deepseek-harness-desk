import AppKit
import Foundation
import WebKit

enum WebViewZoom {
    static let minimumScale = 0.5
    static let maximumScale = 2.0
    static let step = 0.1

    static func clamped(_ scale: Double) -> Double {
        min(max(scale, minimumScale), maximumScale)
    }

    static func adjusted(_ scale: Double, by direction: Int) -> Double {
        let adjusted = scale + (Double(direction) * step)
        return (clamped(adjusted) * 10).rounded() / 10
    }
}

enum WebViewLoadRecovery {
    static let maxRetryCount = 3

    static func shouldRetry(_ error: Error, attempt: Int) -> Bool {
        guard attempt < maxRetryCount else { return false }

        let nsError = error as NSError
        guard nsError.domain == NSURLErrorDomain else { return false }

        switch nsError.code {
        case NSURLErrorCannotConnectToHost,
             NSURLErrorNetworkConnectionLost,
             NSURLErrorTimedOut,
             NSURLErrorCannotFindHost,
             NSURLErrorDNSLookupFailed:
            return true
        default:
            return false
        }
    }

    static func delay(for attempt: Int) -> TimeInterval {
        switch attempt {
        case 0: return 0.5
        case 1: return 1
        default: return 2
        }
    }
}

@MainActor
final class WebViewController: NSObject, ObservableObject {
    private static let zoomScaleKey = "webViewZoomScale"

    weak var webView: WKWebView?

    @Published private(set) var loadErrorMessage: String?
    @Published private(set) var zoomScale: Double

    private var pendingURL: URL?
    private var retryTask: Task<Void, Never>?
    private var retryCount = 0

    override init() {
        if UserDefaults.standard.object(forKey: Self.zoomScaleKey) == nil {
            zoomScale = 1.0
        } else {
            zoomScale = WebViewZoom.clamped(
                UserDefaults.standard.double(forKey: Self.zoomScaleKey)
            )
        }
        super.init()
    }

    deinit {
        retryTask?.cancel()
    }

    func attach(_ webView: WKWebView) {
        if self.webView !== webView {
            self.webView = webView
        }
        webView.pageZoom = CGFloat(zoomScale)
        loadPendingURLIfNeeded()
    }

    func load(_ url: URL) {
        beginLoading(url)
        webView?.load(URLRequest(url: url))
    }

    func restore(url: URL?, forceReload: Bool = false) {
        guard let url else { return }
        if pendingURL != url {
            beginLoading(url)
        } else {
            pendingURL = url
        }
        loadPendingURLIfNeeded(forceReload: forceReload)
    }

    func retryLoading() {
        guard let pendingURL else { return }
        beginLoading(pendingURL)
        webView?.load(URLRequest(url: pendingURL))
    }

    func zoomIn() {
        setZoom(WebViewZoom.adjusted(zoomScale, by: 1))
    }

    func zoomOut() {
        setZoom(WebViewZoom.adjusted(zoomScale, by: -1))
    }

    func resetZoom() {
        setZoom(1.0)
    }

    func navigationDidStart() {
        loadErrorMessage = nil
    }

    func navigationDidFinish() {
        retryTask?.cancel()
        retryTask = nil
        retryCount = 0
        loadErrorMessage = nil
    }

    func navigationDidFail(with error: Error) {
        guard (error as NSError).code != NSURLErrorCancelled,
              let pendingURL,
              let webView else {
            return
        }

        let attempt = retryCount
        guard WebViewLoadRecovery.shouldRetry(error, attempt: attempt) else {
            retryTask = nil
            loadErrorMessage = "无法加载 Harness 页面：\(error.localizedDescription)"
            return
        }

        retryCount += 1
        retryTask?.cancel()
        let delay = WebViewLoadRecovery.delay(for: attempt)
        retryTask = Task { @MainActor [weak self, weak webView] in
            do {
                try await Task.sleep(for: .seconds(delay))
            } catch {
                return
            }

            guard let self,
                  let webView,
                  self.pendingURL == pendingURL else { return }

            self.retryTask = nil
            webView.load(URLRequest(url: pendingURL))
        }
    }

    func reload() {
        webView?.reload()
    }

    func goBack() {
        webView?.goBack()
    }

    func goForward() {
        webView?.goForward()
    }

    private func setZoom(_ scale: Double) {
        let clampedScale = WebViewZoom.clamped(scale)
        zoomScale = clampedScale
        UserDefaults.standard.set(clampedScale, forKey: Self.zoomScaleKey)
        webView?.pageZoom = CGFloat(clampedScale)
    }

    private func beginLoading(_ url: URL) {
        retryTask?.cancel()
        retryTask = nil
        retryCount = 0
        pendingURL = url
        loadErrorMessage = nil
    }

    private func loadPendingURLIfNeeded(forceReload: Bool = false) {
        guard let webView, let pendingURL else { return }

        if forceReload {
            beginLoading(pendingURL)
            webView.stopLoading()
            if let currentURL = webView.url,
               currentURL.scheme == pendingURL.scheme,
               currentURL.host == pendingURL.host,
               currentURL.port == pendingURL.port {
                webView.reload()
            } else {
                webView.load(URLRequest(url: pendingURL))
            }
            return
        }

        guard !webView.isLoading else { return }

        if let currentURL = webView.url,
           currentURL.scheme == pendingURL.scheme,
           currentURL.host == pendingURL.host,
           currentURL.port == pendingURL.port {
            return
        }

        webView.load(URLRequest(url: pendingURL))
    }
}
