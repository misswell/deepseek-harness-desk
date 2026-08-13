import SwiftUI

struct HarnessErrorView: View {
    @EnvironmentObject private var harnessManager: HarnessManager
    @EnvironmentObject private var runtimeManager: RuntimeManager

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

            if runtimeManager.needsInstallation {
                VStack(spacing: 10) {
                    Text("首次使用需要安装运行时组件。安装完成后会自动启动 Harness。")
                        .font(.callout)
                        .foregroundStyle(.secondary)

                    if runtimeManager.isInstalling {
                        ProgressView()
                            .frame(width: 260)
                        Text(runtimeManager.installationMessage)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    } else {
                        Button("一键安装运行时") {
                            Task {
                                await runtimeManager.install()
                                if !runtimeManager.needsInstallation {
                                    await harnessManager.start()
                                }
                            }
                        }
                        .keyboardShortcut(.defaultAction)
                    }
                }
            }

            HStack(spacing: 12) {
                Button("重新启动") {
                    Task { await harnessManager.restart() }
                }
                .disabled(runtimeManager.isInstalling)

                Button("查看日志") {
                    NSWorkspace.shared.open(PathUtils.logsDirectory)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(32)
    }
}
