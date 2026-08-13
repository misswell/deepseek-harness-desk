import SwiftUI

struct MainView: View {
    @EnvironmentObject private var harnessManager: HarnessManager
    @EnvironmentObject private var webViewController: WebViewController

    var body: some View {
        Group {
            switch harnessManager.state {
            case .running:
                if let serverURL = harnessManager.serverURL {
                    HarnessWebView(
                        controller: webViewController,
                        localHost: serverURL.host ?? "127.0.0.1",
                        allowsDeveloperTools: false
                    )
                    .id(serverURL)
                    .task(id: serverURL) {
                        webViewController.load(serverURL)
                    }
                } else {
                    StartingView()
                }
            case .starting, .stopping, .crashed:
                StartingView()
            case .failed:
                HarnessErrorView()
            case .stopped:
                VStack(spacing: 14) {
                    Image(nsImage: NSApplication.shared.applicationIconImage)
                        .resizable()
                        .aspectRatio(contentMode: .fit)
                        .frame(width: 88, height: 88)
                    Text("DeepSeek Harness Desk")
                        .font(.title2)
                    Button("启动 Harness") {
                        Task { await harnessManager.start() }
                    }
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 900, minHeight: 600)
    }
}
