import SwiftUI

struct MainView: View {
    @EnvironmentObject private var harnessManager: HarnessManager
    @EnvironmentObject private var webViewController: WebViewController
    @EnvironmentObject private var updateManager: UpdateManager
    @EnvironmentObject private var runtimeManager: RuntimeManager
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        ZStack(alignment: .bottomTrailing) {
            Group {
                switch harnessManager.state {
                case .running:
                    if let serverURL = harnessManager.serverURL {
                        ZStack {
                            HarnessWebView(
                                controller: webViewController,
                                url: serverURL,
                                localHost: serverURL.host ?? "127.0.0.1",
                                allowsDeveloperTools: false
                            )
                            .id(serverURL)
                            .task(id: serverURL) {
                                webViewController.restore(url: serverURL)
                            }

                            if let message = webViewController.loadErrorMessage {
                                WebViewLoadErrorView(message: message) {
                                    webViewController.retryLoading()
                                }
                            }
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

            VStack(alignment: .trailing, spacing: 12) {
                if updateManager.showsInstallProgress {
                    InstallationProgressCard(
                        title: "App 更新",
                        step: updateManager.installStep,
                        progress: updateManager.installProgress,
                        progressLabel: updateManager.installProgressLabel,
                        logs: updateManager.installLogs,
                        isActive: updateManager.isInstalling,
                        hasError: updateManager.status.contains("失败"),
                        onClose: { updateManager.dismissInstallProgress() }
                    )
                }

                if runtimeManager.showsInstallProgress && !harnessManager.state.isFailed {
                    InstallationProgressCard(
                        title: "内置 Runtime",
                        step: runtimeManager.installStep,
                        progress: runtimeManager.installProgress,
                        progressLabel: runtimeManager.installProgressLabel,
                        logs: runtimeManager.installLogs,
                        isActive: runtimeManager.isInstalling,
                        hasError: {
                            if case .failed = runtimeManager.installState { return true }
                            return false
                        }(),
                        onClose: { runtimeManager.dismissInstallProgress() }
                    )
                }
            }
            .frame(width: 360, alignment: .trailing)
            .padding(18)
        }
        .overlay(alignment: .topTrailing) {
            if let release = updateManager.availableRelease, updateManager.hasAvailableUpdate {
                UpdateAvailableBanner(version: release.version)
                    .environmentObject(updateManager)
                    .padding(.top, 52)
                    .padding(.trailing, 18)
            }
        }
        .frame(minWidth: 900, minHeight: 600)
        .ignoresSafeArea(.container, edges: .top)
        .onReceive(
            NotificationCenter.default.publisher(for: AppDelegate.openMainWindowNotification)
        ) { _ in
            openWindow(id: "main")
        }
    }
}

struct WebViewLoadErrorView: View {
    let message: String
    let retry: () -> Void

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: "network.slash")
                .font(.system(size: 28, weight: .medium))
                .foregroundStyle(.orange)

            Text("Harness 页面加载失败")
                .font(.headline)

            Text(message)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .textSelection(.enabled)

            Button("重新加载", action: retry)
                .buttonStyle(.borderedProminent)
        }
        .padding(24)
        .frame(maxWidth: 420)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .strokeBorder(Color.orange.opacity(0.35), lineWidth: 1)
        }
        .shadow(color: .black.opacity(0.18), radius: 20, y: 8)
    }
}

struct UpdateAvailableBanner: View {
    @EnvironmentObject private var updateManager: UpdateManager
    let version: String

    var body: some View {
        HStack(spacing: 11) {
            Image(nsImage: NSApplication.shared.applicationIconImage)
                .resizable()
                .aspectRatio(contentMode: .fit)
                .frame(width: 34, height: 34)

            VStack(alignment: .leading, spacing: 2) {
                Text("DeepSeek Harness Desk")
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                Text("发现新版本 \(version)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Button {
                Task { await updateManager.installAvailableUpdate() }
            } label: {
                Image(systemName: "arrow.down")
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 30, height: 30)
                    .background(Color.accentColor, in: Circle())
            }
            .buttonStyle(.plain)
            .help("下载并安装更新")

            Button {
                updateManager.ignoreAvailableUpdate()
            } label: {
                Image(systemName: "bell.slash")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 24, height: 24)
            }
            .buttonStyle(.plain)
            .help("忽略此版本")

            Button {
                updateManager.dismissAvailableUpdate()
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .frame(width: 24, height: 24)
            }
            .buttonStyle(.plain)
            .help("关闭更新提醒")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(.ultraThickMaterial, in: RoundedRectangle(cornerRadius: 13, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 13, style: .continuous)
                .strokeBorder(Color.white.opacity(0.14), lineWidth: 1)
        }
        .shadow(color: .black.opacity(0.22), radius: 16, y: 7)
    }
}

struct InstallationProgressCard: View {
    @State private var isCollapsed = false

    let title: String
    let step: String
    let progress: Double?
    let progressLabel: String
    let logs: [String]
    let isActive: Bool
    let hasError: Bool
    let onClose: (() -> Void)?

    init(
        title: String,
        step: String,
        progress: Double?,
        progressLabel: String,
        logs: [String],
        isActive: Bool,
        hasError: Bool,
        onClose: (() -> Void)? = nil
    ) {
        self.title = title
        self.step = step
        self.progress = progress
        self.progressLabel = progressLabel
        self.logs = logs
        self.isActive = isActive
        self.hasError = hasError
        self.onClose = onClose
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: iconName)
                    .font(.title3)
                    .foregroundStyle(iconColor)
                    .frame(width: 24, height: 24)

                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(.headline)
                    Text(step.isEmpty ? "准备开始…" : step)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                }

                Spacer(minLength: 8)

                Button {
                    withAnimation(.easeOut(duration: 0.16)) {
                        isCollapsed.toggle()
                    }
                } label: {
                    Image(systemName: isCollapsed ? "chevron.up" : "chevron.down")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .frame(width: 22, height: 22)
                }
                .buttonStyle(.plain)
                .help(isCollapsed ? "展开进度" : "折叠进度")

                if let onClose {
                    Button(action: onClose) {
                        Image(systemName: "xmark")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .frame(width: 22, height: 22)
                    }
                    .buttonStyle(.plain)
                    .help("关闭进度窗口")
                }

                if isActive {
                    ProgressView()
                        .controlSize(.small)
                }
            }

            if !isCollapsed {
                if let progress {
                    ProgressView(value: progress)
                        .tint(.accentColor)
                } else {
                    ProgressView()
                        .tint(.accentColor)
                }

                HStack {
                    Text(progressLabel.isEmpty ? (isActive ? "处理中…" : "已停止") : progressLabel)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                    Spacer()
                    if let progress {
                        Text("\(Int(progress * 100))%")
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }

                Divider()

                HStack {
                    Text("实时日志")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    Spacer()
                    Text("\(logs.count) 条")
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(.tertiary)
                }

                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 4) {
                            if logs.isEmpty {
                                Text("等待安装日志…")
                                    .font(.caption.monospaced())
                                    .foregroundStyle(.tertiary)
                            } else {
                                ForEach(logs.indices, id: \.self) { index in
                                    Text(logs[index])
                                        .font(.caption2.monospaced())
                                        .foregroundStyle(.secondary)
                                        .textSelection(.enabled)
                                        .id(index)
                                }
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .frame(height: 96)
                    .onChange(of: logs.count) { _, _ in
                        guard let lastIndex = logs.indices.last else { return }
                        withAnimation(.easeOut(duration: 0.16)) {
                            proxy.scrollTo(lastIndex, anchor: .bottom)
                        }
                    }
                    .onAppear {
                        if let lastIndex = logs.indices.last {
                            proxy.scrollTo(lastIndex, anchor: .bottom)
                        }
                    }
                }
            }
        }
        .padding(16)
        .frame(width: 360)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .strokeBorder(borderColor.opacity(0.35), lineWidth: 1)
        }
        .shadow(color: .black.opacity(0.16), radius: 18, y: 8)
    }

    private var iconName: String {
        if hasError { return "exclamationmark.triangle.fill" }
        if isActive { return "arrow.down.circle.fill" }
        return "checkmark.circle.fill"
    }

    private var iconColor: Color {
        if hasError { return .orange }
        if isActive { return .accentColor }
        return .green
    }

    private var borderColor: Color {
        if hasError { return .orange }
        if isActive { return .accentColor }
        return .green
    }
}
