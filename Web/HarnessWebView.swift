import AppKit
import SwiftUI
import WebKit

struct HarnessWebView: NSViewRepresentable {
    let controller: WebViewController
    let localHost: String
    let allowsDeveloperTools: Bool

    func makeCoordinator() -> Coordinator {
        Coordinator(controller: controller, localHost: localHost)
    }

    func makeNSView(context: Context) -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = true
        configuration.websiteDataStore = .default()

        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = context.coordinator
        webView.uiDelegate = context.coordinator
        webView.allowsBackForwardNavigationGestures = true
        webView.setValue(false, forKey: "drawsBackground")

        if allowsDeveloperTools {
            webView.configuration.preferences.setValue(true, forKey: "developerExtrasEnabled")
        }

        controller.attach(webView)
        return webView
    }

    func updateNSView(_ webView: WKWebView, context: Context) {
        controller.attach(webView)
    }

    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate {
        private let controller: WebViewController
        private let localHost: String

        init(controller: WebViewController, localHost: String) {
            self.controller = controller
            self.localHost = localHost
        }

        func webView(
            _ webView: WKWebView,
            decidePolicyFor navigationAction: WKNavigationAction,
            decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
        ) {
            guard let url = navigationAction.request.url else {
                decisionHandler(.cancel)
                return
            }

            if url.host == localHost || url.host == "127.0.0.1" || url.host == "localhost" {
                decisionHandler(.allow)
            } else if url.scheme == "http" || url.scheme == "https" {
                NSWorkspace.shared.open(url)
                decisionHandler(.cancel)
            } else {
                decisionHandler(.cancel)
            }
        }

        func webView(
            _ webView: WKWebView,
            createWebViewWith configuration: WKWebViewConfiguration,
            for navigationAction: WKNavigationAction,
            windowFeatures: WKWindowFeatures
        ) -> WKWebView? {
            guard let url = navigationAction.request.url else { return nil }
            if url.host == localHost || url.host == "127.0.0.1" || url.host == "localhost" {
                webView.load(navigationAction.request)
            } else {
                NSWorkspace.shared.open(url)
            }
            return nil
        }

        func webView(
            _ webView: WKWebView,
            runJavaScriptAlertPanelWithMessage message: String,
            initiatedByFrame frame: WKFrameInfo,
            completionHandler: @escaping () -> Void
        ) {
            let alert = NSAlert()
            alert.messageText = message
            alert.addButton(withTitle: "OK")
            alert.runModal()
            completionHandler()
        }
    }
}
