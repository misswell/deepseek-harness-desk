import CryptoKit
import Darwin
import Foundation
import SwiftUI

enum RuntimeInstallState: Equatable, Sendable {
    case ready
    case installing(String)
    case installed
    case failed(String)

    var isInstalling: Bool {
        if case .installing = self { return true }
        return false
    }
}

@MainActor
final class RuntimeManager: ObservableObject {
    static let packageName = "@deepseek-ai/dsh"
    static let harnessVersion = "0.1.0-rc.6"
    static let nodeVersion = "24.19.0"
    static let npmMetadataURL = URL(string: "https://registry.npmjs.org/@deepseek-ai%2fdsh")!

    @Published private(set) var dshExecutableURL: URL?
    @Published private(set) var nodeExecutableURL: URL?
    @Published private(set) var status: String
    @Published private(set) var installState: RuntimeInstallState = .ready
    @Published private(set) var installationMessage = ""
    @Published private(set) var isUsingManagedRuntime = false
    @Published private(set) var managedHarnessVersion: String
    @Published private(set) var latestHarnessVersion: String?
    @Published private(set) var updateStatus = ""
    @Published private(set) var isCheckingForUpdates = false
    @Published private(set) var isUpdating = false
    @Published private(set) var automaticallyChecksForUpdates: Bool
    @Published private(set) var automaticallyInstallsUpdates: Bool
    @Published private(set) var installProgress: Double?
    @Published private(set) var installStep = ""
    @Published private(set) var installProgressLabel = ""
    @Published private(set) var installLogs: [String] = []
    @Published private(set) var isInstallProgressDismissed = false

    var needsInstallation: Bool {
        dshExecutableURL == nil
    }

    var isInstalling: Bool {
        installState.isInstalling
    }

    var showsInstallProgress: Bool {
        guard !isInstallProgressDismissed else { return false }
        if isInstalling { return true }
        if case .failed = installState { return true }
        return false
    }

    func dismissInstallProgress() {
        isInstallProgressDismissed = true
    }

    private let fileManager = FileManager.default
    private let logger: LogManager
    private var installationTask: Task<Void, Never>?
    private var updateCheckTask: Task<Void, Never>?
    private var harnessRestartHandler: (() async -> Bool)?

    init(logger: LogManager = LogManager()) {
        self.logger = logger
        self.status = "正在检查 DeepSeek Harness"
        self.managedHarnessVersion = UserDefaults.standard.string(forKey: "managedHarnessVersion") ?? Self.harnessVersion
        if UserDefaults.standard.object(forKey: "autoCheckHarnessUpdates") == nil {
            UserDefaults.standard.set(true, forKey: "autoCheckHarnessUpdates")
        }
        if UserDefaults.standard.object(forKey: "autoInstallHarnessUpdates") == nil {
            UserDefaults.standard.set(true, forKey: "autoInstallHarnessUpdates")
        }
        automaticallyChecksForUpdates = UserDefaults.standard.bool(forKey: "autoCheckHarnessUpdates")
        automaticallyInstallsUpdates = UserDefaults.standard.bool(forKey: "autoInstallHarnessUpdates")
        managedHarnessVersion = activeHarnessVersion
        refresh()
    }

    deinit {
        installationTask?.cancel()
        updateCheckTask?.cancel()
    }

    func refresh() {
        let managedDsh = managedDshExecutableURL
        if fileManager.isExecutableFile(atPath: managedDsh.path) {
            dshExecutableURL = managedDsh
            nodeExecutableURL = managedNodeExecutableURL
            managedHarnessVersion = activeHarnessVersion
            status = "Managed Harness \(managedHarnessVersion)"
            isUsingManagedRuntime = true
            if !isInstalling {
                installState = .installed
            }
            return
        }

        let systemDsh = PathUtils.findExecutable(named: "dsh")
        dshExecutableURL = systemDsh
        nodeExecutableURL = PathUtils.findExecutable(named: "node")
        isUsingManagedRuntime = false
        if let systemDsh {
            status = systemDsh.path
            if !isInstalling {
                installState = .ready
            }
        } else {
            status = "尚未安装 DeepSeek Harness"
            if !isInstalling {
                installState = .ready
            }
        }
    }

    func startUpdateChecks() {
        guard updateCheckTask == nil, automaticallyChecksForUpdates else { return }

        updateCheckTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(4))
            guard !Task.isCancelled, let self else { return }
            await self.checkForUpdates(interactive: false, automaticallyInstall: true)
            self.updateCheckTask = nil
        }
    }

    func setHarnessRestartHandler(_ handler: @escaping () async -> Bool) {
        harnessRestartHandler = handler
    }

    func install() async {
        guard !isInstalling else { return }

        refresh()
        guard needsInstallation else { return }

        installationTask = Task { @MainActor [weak self] in
            await self?.performInstallation()
        }
        await installationTask?.value
        installationTask = nil
    }

    func setAutomaticChecks(_ enabled: Bool) {
        automaticallyChecksForUpdates = enabled
        UserDefaults.standard.set(enabled, forKey: "autoCheckHarnessUpdates")
        if enabled {
            startUpdateChecks()
        } else {
            updateCheckTask?.cancel()
            updateCheckTask = nil
        }
    }

    func setAutomaticInstallation(_ enabled: Bool) {
        automaticallyInstallsUpdates = enabled
        UserDefaults.standard.set(enabled, forKey: "autoInstallHarnessUpdates")
    }

    func checkForUpdates(
        interactive: Bool = true,
        automaticallyInstall: Bool = false
    ) async {
        guard !isCheckingForUpdates, !isUpdating, !isInstalling else { return }
        refresh()
        guard isUsingManagedRuntime else {
            updateStatus = needsInstallation
                ? "尚未安装内置 dsh，完成一键安装后可检查更新。"
                : "当前使用系统 PATH 中的 dsh，不管理系统安装。"
            if interactive {
                showUpdateAlert(
                    title: "无法检查内置 dsh",
                    message: updateStatus,
                    buttons: [("好", .alertFirstButtonReturn)]
                )
            }
            return
        }

        isCheckingForUpdates = true
        updateStatus = "正在检查内置 DeepSeek Harness 更新…"
        defer { isCheckingForUpdates = false }

        do {
            var request = URLRequest(url: Self.npmMetadataURL)
            request.setValue("DeepSeek Harness Desk/\(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.2.24")", forHTTPHeaderField: "User-Agent")
            let (data, response) = try await URLSession.shared.data(for: request)
            if let httpResponse = response as? HTTPURLResponse,
               !(200..<300).contains(httpResponse.statusCode) {
                throw RuntimeError.updateFailed("npm 返回 HTTP \(httpResponse.statusCode)")
            }

            let metadata = try JSONDecoder().decode(NPMPackageMetadata.self, from: data)
            guard let latest = metadata.distTags.latest, !latest.isEmpty else {
                throw RuntimeError.updateFailed("npm 未返回 \(Self.packageName) 的 latest 版本")
            }
            latestHarnessVersion = latest

            guard UpdateManager.isNewer(latest, than: managedHarnessVersion) else {
                updateStatus = "内置 dsh 已是最新版本 \(managedHarnessVersion)"
                if interactive {
                    showUpdateAlert(
                        title: "内置 dsh 已是最新版本",
                        message: "当前版本：\(managedHarnessVersion)\n最新版本：\(latest)",
                        buttons: [("好", .alertFirstButtonReturn)]
                    )
                }
                return
            }

            updateStatus = "发现内置 dsh 新版本 \(latest)"
            if automaticallyInstall && automaticallyInstallsUpdates {
                updateStatus = "发现内置 dsh 新版本 \(latest)，准备自动安装…"
                await installAvailableUpdate(version: latest, automatically: true)
            } else if interactive {
                showUpdateAlert(for: latest)
            }
        } catch is CancellationError {
            updateStatus = "内置 dsh 更新检查已取消"
        } catch {
            updateStatus = "内置 dsh 检查失败：\(error.localizedDescription)"
            if interactive {
                showUpdateAlert(
                    title: "检查内置 dsh 更新失败",
                    message: error.localizedDescription,
                    buttons: [("好", .alertFirstButtonReturn)]
                )
            }
        }
    }

    func installAvailableUpdate() async {
        guard let latestHarnessVersion,
              UpdateManager.isNewer(latestHarnessVersion, than: managedHarnessVersion) else { return }
        await installAvailableUpdate(version: latestHarnessVersion, automatically: false)
    }

    func processEnvironment() -> [String: String] {
        var environment = ProcessInfo.processInfo.environment
        var pathEntries: [String] = []

        if let nodeExecutableURL {
            pathEntries.append(nodeExecutableURL.deletingLastPathComponent().path)
        }
        if isUsingManagedRuntime {
            pathEntries.append(managedDshDirectory.appendingPathComponent("node_modules/.bin").path)
        }
        if let dshExecutableURL {
            pathEntries.append(dshExecutableURL.deletingLastPathComponent().path)
        }
        if let existingPath = environment["PATH"], !existingPath.isEmpty {
            pathEntries.append(contentsOf: existingPath.split(separator: ":").map(String.init))
        }

        let fallbackEntries = [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin"
        ]
        pathEntries.append(contentsOf: fallbackEntries)

        var seen = Set<String>()
        environment["PATH"] = pathEntries.filter { seen.insert($0).inserted }.joined(separator: ":")
        return environment
    }

    private func performInstallation() async {
        beginInstallation(title: "安装内置 DeepSeek Harness")
        let runtimeDirectory = PathUtils.applicationSupportDirectory
            .appendingPathComponent("runtime", isDirectory: true)
        let stagingDirectory = runtimeDirectory
            .appendingPathComponent(".staging-\(UUID().uuidString)", isDirectory: true)

        do {
            try fileManager.createDirectory(
                at: runtimeDirectory,
                withIntermediateDirectories: true
            )
            try fileManager.createDirectory(
                at: stagingDirectory,
                withIntermediateDirectories: true
            )
            defer { try? fileManager.removeItem(at: stagingDirectory) }

            let nodeRoot = runtimeDirectory
                .appendingPathComponent("node", isDirectory: true)
                .appendingPathComponent(Self.nodeVersion, isDirectory: true)
            let dshRoot = runtimeDirectory
                .appendingPathComponent("dsh", isDirectory: true)
                .appendingPathComponent(Self.harnessVersion, isDirectory: true)

            if !fileManager.isExecutableFile(atPath: nodeExecutable(at: nodeRoot).path) {
                try await installNode(
                    into: nodeRoot,
                    stagingDirectory: stagingDirectory
                )
            }

            guard fileManager.isExecutableFile(atPath: nodeExecutable(at: nodeRoot).path) else {
                throw RuntimeError.installationFailed("Node Runtime 安装后未找到可执行文件。")
            }

            if !fileManager.isExecutableFile(atPath: dshExecutable(at: dshRoot).path) {
                try await installHarness(
                    into: dshRoot,
                    nodeRoot: nodeRoot,
                    stagingDirectory: stagingDirectory,
                    version: Self.harnessVersion
                )
            }

            guard fileManager.isExecutableFile(atPath: dshExecutable(at: dshRoot).path) else {
                throw RuntimeError.installationFailed("DeepSeek Harness 安装后未找到 dsh 命令。")
            }

            UserDefaults.standard.set(Self.harnessVersion, forKey: "managedHarnessVersion")
            refresh()
            installationMessage = "安装完成"
            installProgress = 1
            installProgressLabel = "已完成"
            appendInstallLog("内置 Node.js 和 DeepSeek Harness 安装完成")
            installState = .installed
        } catch is CancellationError {
            installationMessage = "安装已取消"
            setInstalling("安装已取消")
            appendInstallLog("安装已取消")
            installState = .ready
        } catch {
            let message = error.localizedDescription
            installationMessage = message
            setInstalling("安装失败")
            appendInstallLog("失败：\(message)")
            installState = .failed(message)
            status = "安装失败"
        }
    }

    private func installNode(into nodeRoot: URL, stagingDirectory: URL) async throws {
        let architecture = currentArchitecture()
        let archiveName = "node-v\(Self.nodeVersion)-darwin-\(architecture).tar.gz"
        let downloadURL = URL(string: "https://nodejs.org/dist/v\(Self.nodeVersion)/\(archiveName)")!
        let expectedSHA256 = architecture == "arm64"
            ? "8294b7aa9b03997481c06babf1e8b270c859358f27da57a11509afe537ac381d"
            : "d1b5e999db158c62fe8f7267a4476b035d8bd93b1a605bac24a3f0dd166e3316"
        let archiveURL = stagingDirectory.appendingPathComponent(archiveName)

        setInstalling("正在下载 Node.js \(Self.nodeVersion)…")
        try await download(downloadURL, to: archiveURL)
        appendInstallLog("Node.js 下载完成（\(formatBytes(fileSize(at: archiveURL)))）")
        try verifySHA256(of: archiveURL, expected: expectedSHA256)
        appendInstallLog("Node.js SHA-256 校验通过")

        let extractionDirectory = stagingDirectory.appendingPathComponent("node-extracted", isDirectory: true)
        try fileManager.createDirectory(at: extractionDirectory, withIntermediateDirectories: true)
        setInstalling("正在解压 Node.js…")
        let result = try await ProcessRunner.run(
            executableURL: URL(fileURLWithPath: "/usr/bin/tar"),
            arguments: ["-xzf", archiveURL.path, "-C", extractionDirectory.path]
        )
        guard result.succeeded else {
            throw RuntimeError.installationFailed("解压 Node.js 失败：\(result.output.trimmingCharacters(in: .whitespacesAndNewlines))")
        }

        let extractedRoot = extractionDirectory
            .appendingPathComponent("node-v\(Self.nodeVersion)-darwin-\(currentArchitecture())", isDirectory: true)
        guard fileManager.fileExists(atPath: extractedRoot.path) else {
            throw RuntimeError.installationFailed("Node.js 安装包内容不完整。")
        }

        try fileManager.createDirectory(at: nodeRoot.deletingLastPathComponent(), withIntermediateDirectories: true)
        if fileManager.fileExists(atPath: nodeRoot.path) {
            try fileManager.removeItem(at: nodeRoot)
        }
        try fileManager.moveItem(at: extractedRoot, to: nodeRoot)
    }

    private func installHarness(
        into dshRoot: URL,
        nodeRoot: URL,
        stagingDirectory: URL,
        version: String
    ) async throws {
        let npmURL = nodeRoot.appendingPathComponent("bin/npm")
        guard fileManager.isExecutableFile(atPath: npmURL.path) else {
            throw RuntimeError.installationFailed("Node.js 安装完成，但没有找到 npm。")
        }

        let dshStaging = stagingDirectory.appendingPathComponent("dsh", isDirectory: true)
        try fileManager.createDirectory(at: dshStaging, withIntermediateDirectories: true)
        setInstalling("正在安装 DeepSeek Harness \(version)…")
        let result = try await ProcessRunner.run(
            executableURL: npmURL,
            arguments: [
                "install",
                "--prefix", dshStaging.path,
                "--no-audit",
                "--no-fund",
                "--no-update-notifier",
                "--no-package-lock",
                "\(Self.packageName)@\(version)"
            ],
            environment: processEnvironmentForNode(nodeRoot: nodeRoot),
            currentDirectoryURL: dshStaging,
            onOutput: { [weak self] output in
                Task { @MainActor [weak self] in
                    self?.appendInstallLogChunk(output)
                }
            }
        )
        guard result.succeeded else {
            let details = result.output
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .suffix(1800)
            throw RuntimeError.installationFailed("安装 DeepSeek Harness 失败：\(details)")
        }

        guard fileManager.isExecutableFile(
            atPath: dshStaging.appendingPathComponent("node_modules/.bin/dsh").path
        ) else {
            throw RuntimeError.installationFailed("npm 安装完成，但没有生成 dsh 命令。")
        }

        try fileManager.createDirectory(at: dshRoot.deletingLastPathComponent(), withIntermediateDirectories: true)
        if fileManager.fileExists(atPath: dshRoot.path) {
            try fileManager.removeItem(at: dshRoot)
        }
        try fileManager.moveItem(at: dshStaging, to: dshRoot)
    }

    private func processEnvironmentForNode(nodeRoot: URL) -> [String: String] {
        var environment = ProcessInfo.processInfo.environment
        let nodeBin = nodeRoot.appendingPathComponent("bin").path
        let currentPath = environment["PATH"] ?? ""
        environment["PATH"] = [nodeBin, currentPath]
            .filter { !$0.isEmpty }
            .joined(separator: ":")
        return environment
    }

    private func download(_ url: URL, to destination: URL) async throws {
        var request = URLRequest(
            url: url,
            cachePolicy: .reloadIgnoringLocalCacheData,
            timeoutInterval: 300
        )
        request.setValue("DeepSeek Harness Desk", forHTTPHeaderField: "User-Agent")
        let response = try await DownloadRunner.download(
            request: request,
            using: .shared,
            to: destination,
            onProgress: { [weak self] progress in
                Task { @MainActor [weak self] in
                    self?.updateDownloadProgress(progress)
                }
            }
        )
        if !(200..<300).contains(response.statusCode) {
            throw RuntimeError.installationFailed("下载失败（HTTP \(response.statusCode)）。")
        }
    }

    private func verifySHA256(of file: URL, expected: String) throws {
        setInstalling("正在校验下载内容…")
        let data = try Data(contentsOf: file)
        let actual = SHA256.hash(data: data)
            .map { String(format: "%02x", $0) }
            .joined()
        guard actual == expected else {
            throw RuntimeError.installationFailed("Node.js 下载校验失败，请重试。")
        }
    }

    private func setInstalling(_ message: String) {
        installationMessage = message
        installStep = message
        installProgress = nil
        installProgressLabel = ""
        installState = .installing(message)
        status = message
        if installLogs.last != message {
            appendInstallLog(message)
        }
    }

    private func beginInstallation(title: String) {
        isInstallProgressDismissed = false
        installProgress = nil
        installStep = ""
        installProgressLabel = ""
        installLogs.removeAll(keepingCapacity: true)
        appendInstallLog(title)
    }

    private func updateDownloadProgress(_ progress: DownloadRunner.Progress) {
        installProgress = progress.fractionCompleted
        if let totalBytes = progress.totalBytes {
            installProgressLabel = "\(formatBytes(progress.bytesWritten)) / \(formatBytes(totalBytes))"
        } else {
            installProgressLabel = formatBytes(progress.bytesWritten)
        }
    }

    private func appendInstallLogChunk(_ output: String) {
        for line in output
            .replacingOccurrences(of: "\r", with: "\n")
            .split(whereSeparator: \.isNewline)
            .map(String.init)
            .map({ $0.trimmingCharacters(in: .whitespacesAndNewlines) })
            where !line.isEmpty {
            appendInstallLog(line)
        }
    }

    private func appendInstallLog(_ message: String) {
        let normalized = message.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalized.isEmpty else { return }
        installLogs.append(normalized)
        if installLogs.count > 120 {
            installLogs.removeFirst(installLogs.count - 120)
        }
        logger.append("[Runtime] \(normalized)", to: .update)
    }

    private func fileSize(at url: URL) -> Int64 {
        guard let attributes = try? fileManager.attributesOfItem(atPath: url.path),
              let size = attributes[.size] as? NSNumber else {
            return 0
        }
        return size.int64Value
    }

    private func formatBytes(_ bytes: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)
    }

    private var managedNodeExecutableURL: URL {
        nodeExecutable(at: managedNodeDirectory)
    }

    private var managedDshExecutableURL: URL {
        dshExecutable(at: managedDshDirectory)
    }

    private var activeHarnessVersion: String {
        UserDefaults.standard.string(forKey: "managedHarnessVersion") ?? Self.harnessVersion
    }

    private var managedNodeDirectory: URL {
        PathUtils.applicationSupportDirectory
            .appendingPathComponent("runtime/node/\(Self.nodeVersion)", isDirectory: true)
    }

    private var managedDshDirectory: URL {
        PathUtils.applicationSupportDirectory
            .appendingPathComponent("runtime/dsh/\(activeHarnessVersion)", isDirectory: true)
    }

    private func installAvailableUpdate(version: String, automatically: Bool) async {
        guard !isUpdating, isUsingManagedRuntime,
              let nodeExecutableURL else { return }

        isUpdating = true
        let restartHarness = harnessRestartHandler
        beginInstallation(title: "开始更新内置 dsh \(version)")
        setInstalling("正在下载并安装内置 dsh \(version)…")
        let runtimeDirectory = PathUtils.applicationSupportDirectory
            .appendingPathComponent("runtime", isDirectory: true)
        let stagingDirectory = runtimeDirectory
            .appendingPathComponent(".dsh-update-\(UUID().uuidString)", isDirectory: true)
        let dshRoot = runtimeDirectory
            .appendingPathComponent("dsh", isDirectory: true)
            .appendingPathComponent(version, isDirectory: true)
        var didInstall = false

        do {
            try fileManager.createDirectory(at: stagingDirectory, withIntermediateDirectories: true)
            defer { try? fileManager.removeItem(at: stagingDirectory) }
            let nodeRoot = nodeExecutableURL.deletingLastPathComponent().deletingLastPathComponent()
            try await installHarness(
                into: dshRoot,
                nodeRoot: nodeRoot,
                stagingDirectory: stagingDirectory,
                version: version
            )
            UserDefaults.standard.set(version, forKey: "managedHarnessVersion")
            latestHarnessVersion = version
            managedHarnessVersion = version
            refresh()
            installationMessage = "内置 dsh 更新完成"
            updateStatus = automatically
                ? "内置 dsh 已更新到 \(version)"
                : "内置 dsh 更新完成：\(version)"
            installProgress = 1
            installProgressLabel = "已完成"
            appendInstallLog("内置 dsh \(version) 安装完成")
            installState = .installed
            didInstall = true
        } catch is CancellationError {
            updateStatus = "内置 dsh 更新已取消"
            installationMessage = updateStatus
            setInstalling("内置 dsh 更新已取消")
            appendInstallLog("更新已取消")
            installState = .ready
        } catch {
            let message = error.localizedDescription
            updateStatus = "内置 dsh 更新失败：\(message)"
            installationMessage = message
            setInstalling("内置 dsh 更新失败")
            appendInstallLog("失败：\(message)")
            installState = .failed(message)
            if automatically {
                showUpdateAlert(
                    title: "内置 dsh 自动更新失败",
                    message: "已保留当前版本 \(managedHarnessVersion)。\n\n\(message)",
                    buttons: [("好", .alertFirstButtonReturn)]
                )
            } else {
                showUpdateAlert(
                    title: "内置 dsh 更新失败",
                    message: message,
                    buttons: [("好", .alertFirstButtonReturn)]
                )
            }
        }
        isUpdating = false

        if didInstall, let restartHarness {
            updateStatus = "内置 dsh 已更新到 \(version)，正在重启 Harness…"
            if await restartHarness() {
                updateStatus = automatically
                    ? "内置 dsh 已更新到 \(version)"
                    : "内置 dsh 更新完成：\(version)"
            }
        }
    }

    private func showUpdateAlert(for version: String) {
        let alert = NSAlert()
        alert.messageText = "发现内置 dsh 新版本"
        alert.informativeText = "当前版本：\(managedHarnessVersion)\n最新版本：\(version)\n\n现在下载并安装吗？"
        alert.alertStyle = .informational
        alert.addButton(withTitle: "更新")
        alert.addButton(withTitle: "稍后")
        if alert.runModal() == .alertFirstButtonReturn {
            Task { await installAvailableUpdate(version: version, automatically: false) }
        }
    }

    private func showUpdateAlert(
        title: String,
        message: String,
        buttons: [(String, NSApplication.ModalResponse)]
    ) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.alertStyle = .warning
        for (title, _) in buttons {
            alert.addButton(withTitle: title)
        }
        _ = alert.runModal()
    }

    private func nodeExecutable(at root: URL) -> URL {
        root.appendingPathComponent("bin/node")
    }

    private func dshExecutable(at root: URL) -> URL {
        root.appendingPathComponent("node_modules/.bin/dsh")
    }

    private func currentArchitecture() -> String {
        var size = 0
        sysctlbyname("hw.machine", nil, &size, nil, 0)
        var machine = [CChar](repeating: 0, count: size)
        sysctlbyname("hw.machine", &machine, &size, nil, 0)
        let value = String(cString: machine)
        return value == "arm64" ? "arm64" : "x64"
    }
}

enum RuntimeError: LocalizedError {
    case installationFailed(String)
    case updateFailed(String)

    var errorDescription: String? {
        switch self {
        case let .installationFailed(message):
            return message
        case let .updateFailed(message):
            return message
        }
    }
}

private struct NPMPackageMetadata: Decodable {
    struct DistTags: Decodable {
        let latest: String?
    }

    let distTags: DistTags

    enum CodingKeys: String, CodingKey {
        case distTags = "dist-tags"
    }
}
