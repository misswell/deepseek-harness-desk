import SwiftUI

struct StartingView: View {
    @EnvironmentObject private var harnessManager: HarnessManager

    var body: some View {
        VStack(spacing: 16) {
            ProgressView()
                .controlSize(.large)

            Text("正在启动 DeepSeek Harness…")
                .font(.title3)

            VStack(alignment: .leading, spacing: 8) {
                statusRow("Runtime", value: harnessManager.runtimeVersion)
                statusRow("Harness", value: harnessManager.state.title)
                if let port = harnessManager.port {
                    statusRow("Port", value: String(port))
                }
            }
            .font(.callout)
            .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
    }

    private func statusRow(_ title: String, value: String) -> some View {
        HStack {
            Text(title)
                .frame(width: 70, alignment: .trailing)
            Text(value)
        }
    }
}
