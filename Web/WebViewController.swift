import AppKit
import Foundation
import WebKit

@MainActor
final class WebViewController: NSObject, ObservableObject {
    weak var webView: WKWebView?
    private var pendingURL: URL?

    func attach(_ webView: WKWebView) {
        if self.webView !== webView {
            self.webView = webView
        }
        loadPendingURLIfNeeded()
    }

    func load(_ url: URL) {
        pendingURL = url
        webView?.load(URLRequest(url: url))
    }

    func restore(url: URL?, forceReload: Bool = false) {
        guard let url else { return }
        pendingURL = url
        loadPendingURLIfNeeded(forceReload: forceReload)
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

    private func loadPendingURLIfNeeded(forceReload: Bool = false) {
        guard let webView, let pendingURL else { return }

        if forceReload {
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
