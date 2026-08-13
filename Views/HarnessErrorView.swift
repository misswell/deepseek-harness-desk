import SwiftUI

struct HarnessErrorView: View {
    @EnvironmentObject private var harnessManager: HarnessManager

    var body: some View {
        VStack(spacing: 18) {
            Image(systemName: "exclamationmark.triangle")
                .font(.system(size: 42))
                .foregroundStyle(.orange)

            Text("DeepSeek Harness 无法启动")
                .font(.title2)
                .bold()

            Text(harnessManager.lastError ?? "发生未知错误。")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 560)

            HStack(spacing: 12) {
                Button("重新启动") {
                    Task { await harnessManager.restart() }
                }
                .keyboardShortcut(.defaultAction)

                Button("查看日志") {
                    NSWorkspace.shared.open(PathUtils.logsDirectory)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
    }
}
