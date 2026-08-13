import AppKit
import Foundation
import WebKit

@MainActor
final class WebViewController: NSObject, ObservableObject {
    weak var webView: WKWebView?
    private var pendingURL: URL?

    func attach(_ webView: WKWebView) {
        guard self.webView !== webView else { return }
        self.webView = webView
        if let pendingURL {
            webView.load(URLRequest(url: pendingURL))
        }
    }

    func load(_ url: URL) {
        pendingURL = url
        webView?.load(URLRequest(url: url))
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
}
