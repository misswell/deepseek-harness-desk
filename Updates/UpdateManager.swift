import AppKit
import Foundation
import SwiftUI

@MainActor
final class UpdateManager: ObservableObject {
    enum AutomaticCheckInterval: String, CaseIterable, Identifiable, Sendable {
        case hourly
        case daily
        case weekly

        var id: String { rawValue }

        var title: String {
            switch self {
            case .hourly: return "每小时"
            case .daily: return "每天"
            case .weekly: return "每周"
            }
        }

        var seconds: TimeInterval {
            switch self {
            case .hourly: return 60 * 60
            case .daily: return 24 * 60 * 60
            case .weekly: return 7 * 24 * 60 * 60
            }
        }
    }

    struct ReleaseAsset: Decodable {
        let name: String
        let browserDownloadURL: URL
        let digest: String?

        enum CodingKeys: String, CodingKey {
            case name
            case browserDownloadURL = "browser_download_url"
            case digest
        }
    }

    struct Release: Decodable {
        let tagName: String
        let name: String
        let body: String
        let htmlURL: URL
        let assets: [ReleaseAsset]

        enum CodingKeys: String, CodingKey {
            case tagName = "tag_name"
            case name
            case body
            case htmlURL = "html_url"
            case assets
        }

        var version: String {
            tagName.trimmingCharacters(in: CharacterSet(charactersIn: "vV"))
        }

        var applicationArchive: ReleaseAsset? {
            assets.first {
                $0.name.lowercased().hasSuffix(".zip") &&
                $0.name.lowercased().contains("deepseekharnessdesk")
            }
        }
    }

    static let releasesURL = URL(string: "https://api.github.com/repos/misswell/deepseek-harness-desk/releases/latest")!
    static let currentVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.2.12"

    private static let metadataRequestTimeout: TimeInterval = 20
    private static let archiveDownloadTimeout: TimeInterval = 120

    @Published private(set) var isChecking = false
    @Published private(set) var isInstalling = false
    @Published private(set) var status = ""
    @Published private(set) var automaticallyChecksForUpdates: Bool
    @Published private(set) var automaticCheckInterval: AutomaticCheckInterval
    @Published private(set) var latestReleaseVersion: String?
    @Published private(set) var availableRelease: Release?
    @Published private(set) var installProgress: Double?
    @Published private(set) var installStep = ""
    @Published private(set) var installProgressLabel = ""
    @Published private(set) var installLogs: [String] = []

    private let fileManager = FileManager.default
    private let logger: LogManager
    private let updateSession: URLSession
    private var hasStarted = false
    private var automaticCheckTask: Task<Void, Never>?

    init(logger: LogManager = LogManager()) {
        self.logger = logger
        let sessionConfiguration = URLSessionConfiguration.ephemeral
        sessionConfiguration.waitsForConnectivity = false
        sessionConfiguration.timeoutIntervalForRequest = Self.metadataRequestTimeout
        sessionConfiguration.timeoutIntervalForResource = Self.archiveDownloadTimeout
        self.updateSession = URLSession(configuration: sessionConfiguration)

        if UserDefaults.standard.object(forKey: "autoCheckForUpdates") == nil {
            UserDefaults.standard.set(true, forKey: "autoCheckForUpdates")
        }
        let intervalRawValue = UserDefaults.standard.string(forKey: "autoCheckInterval")
        automaticCheckInterval = AutomaticCheckInterval(rawValue: intervalRawValue ?? "") ?? .hourly
        automaticallyChecksForUpdates = UserDefaults.standard.bool(forKey: "autoCheckForUpdates")
    }

    var showsInstallProgress: Bool {
        isInstalling || !installLogs.isEmpty
    }

    var hasAvailableUpdate: Bool {
        availableRelease != nil && !isInstalling
    }

    deinit {
        automaticCheckTask?.cancel()
        updateSession.invalidateAndCancel()
    }

    func start() {
        guard !hasStarted else { return }
        hasStarted = true
        scheduleAutomaticCheck()
    }

    private func scheduleAutomaticCheck() {
        guard automaticallyChecksForUpdates, automaticCheckTask == nil else { return }

        automaticCheckTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled, let self else { return }
            await self.checkForUpdates(interactive: false)

            while !Task.isCancelled {
                do {
                    try await Task.sleep(for: .seconds(self.automaticCheckInterval.seconds))
                } catch {
                    break
                }
                guard !Task.isCancelled else { break }
                await self.checkForUpdates(interactive: false)
            }
            self.automaticCheckTask = nil
        }
    }

    func setAutomaticChecks(_ enabled: Bool) {
        automaticallyChecksForUpdates = enabled
        UserDefaults.standard.set(enabled, forKey: "autoCheckForUpdates")
        if enabled {
            scheduleAutomaticCheck()
        } else {
            automaticCheckTask?.cancel()
            automaticCheckTask = nil
        }
    }

    func setAutomaticCheckInterval(_ interval: AutomaticCheckInterval) {
        guard automaticCheckInterval != interval else { return }
        automaticCheckInterval = interval
        UserDefaults.standard.set(interval.rawValue, forKey: "autoCheckInterval")
        automaticCheckTask?.cancel()
        automaticCheckTask = nil
        scheduleAutomaticCheck()
    }

    func checkForUpdates(interactive: Bool = true) async {
        guard !isChecking, !isInstalling else { return }
        isChecking = true
        status = "正在检查更新…"
        defer { isChecking = false }

        do {
            var request = URLRequest(
                url: Self.releasesURL,
                cachePolicy: .reloadIgnoringLocalCacheData,
                timeoutInterval: Self.metadataRequestTimeout
            )
            request.setValue("DeepSeek Harness Desk/\(Self.currentVersion)", forHTTPHeaderField: "User-Agent")
            request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
            let (data, response) = try await updateSession.data(for: request)
            if let httpResponse = response as? HTTPURLResponse,
               !(200..<300).contains(httpResponse.statusCode) {
                throw UpdateError.server("GitHub 返回 HTTP \(httpResponse.statusCode)")
            }

            let release = try JSONDecoder().decode(Release.self, from: data)
            latestReleaseVersion = release.version
            guard Self.isNewer(release.version, than: Self.currentVersion) else {
                availableRelease = nil
                status = "已是最新版本 \(Self.currentVersion)"
                if interactive {
                    showAlert(
                        title: "已是最新版本",
                        message: "DeepSeek Harness Desk \(Self.currentVersion) 已是最新版本。",
                        buttons: [("好", .alertFirstButtonReturn)]
                    )
                }
                return
            }

            guard release.applicationArchive != nil else {
                throw UpdateError.server("最新 Release 没有可下载的 macOS App 压缩包")
            }
            availableRelease = release
            status = "发现新版本 \(release.version)"
            if interactive {
                showUpdateAlert(for: release)
            }
        } catch is CancellationError {
            status = "更新检查已取消"
        } catch {
            status = "检查更新失败：\(error.localizedDescription)"
            if interactive {
                showAlert(
                    title: "检查更新失败",
                    message: error.localizedDescription,
                    buttons: [("好", .alertFirstButtonReturn)]
                )
            }
        }
    }

    func installAvailableUpdate(automatically: Bool = false) async {
        guard !isInstalling, let release = availableRelease,
              let asset = release.applicationArchive else { return }

        isInstalling = true
        beginInstallation(for: release.version)
        defer { isInstalling = false }

        do {
            let updateRoot = fileManager.temporaryDirectory
                .appendingPathComponent("DeepSeekHarnessDesk-Update-\(UUID().uuidString)", isDirectory: true)
            try fileManager.createDirectory(at: updateRoot, withIntermediateDirectories: true)
            defer { try? fileManager.removeItem(at: updateRoot) }

            let archiveURL = updateRoot.appendingPathComponent(asset.name)
            setInstallStep("正在下载 DeepSeek Harness Desk \(release.version)…")
            let response = try await downloadArchive(from: asset.browserDownloadURL, to: archiveURL)
            if let httpResponse = response as? HTTPURLResponse,
               !(200..<300).contains(httpResponse.statusCode) {
                throw UpdateError.server("下载更新失败（HTTP \(httpResponse.statusCode)）")
            }
            appendInstallLog("下载完成（\(formatBytes(fileSize(at: archiveURL)))）")

            setInstallStep("正在校验更新包…")
            try await validateArchive(at: archiveURL, expectedDigest: asset.digest)
            appendInstallLog("更新包校验通过")

            let extractionURL = updateRoot.appendingPathComponent("extracted", isDirectory: true)
            try fileManager.createDirectory(at: extractionURL, withIntermediateDirectories: true)
            setInstallStep("正在解压更新包…")
            let result = try await ProcessRunner.run(
                executableURL: URL(fileURLWithPath: "/usr/bin/unzip"),
                arguments: ["-q", "-o", archiveURL.path, "-d", extractionURL.path],
                onOutput: { [weak self] output in
                    Task { @MainActor [weak self] in
                        self?.appendInstallLogChunk(output)
                    }
                }
            )
            guard result.succeeded else {
                throw UpdateError.server("解压更新失败：\(result.output.trimmingCharacters(in: .whitespacesAndNewlines))")
            }
            appendInstallLog("更新包解压完成")

            guard let updatedApp = Self.findApplication(in: extractionURL) else {
                throw UpdateError.server("更新包中没有找到 App")
            }
            setInstallStep("正在验证更新 App…")
            try await validateUpdatedApp(updatedApp, expectedVersion: release.version)
            appendInstallLog("版本、Bundle ID 和签名校验通过")

            setInstallStep("正在安装 DeepSeek Harness Desk \(release.version)…")
            try launchReplacementScript(
                currentApp: Bundle.main.bundleURL,
                updatedApp: updatedApp,
                oldProcessID: ProcessInfo.processInfo.processIdentifier
            )
            installProgress = 1
            installProgressLabel = "已完成"
            appendInstallLog("替换程序已启动，App 即将重启")
            status = automatically ? "更新完成，App 即将重启" : "更新已准备，App 即将重启"
            availableRelease = nil
            NSApp.terminate(nil)
        } catch is CancellationError {
            setInstallStep("更新下载已取消")
            appendInstallLog("更新已取消")
            status = "更新下载已取消"
        } catch {
            setInstallStep("安装更新失败")
            appendInstallLog("失败：\(error.localizedDescription)")
            status = "安装更新失败：\(error.localizedDescription)"
            if automatically {
                showAlert(
                    title: "自动更新失败",
                    message: "已保留当前版本。你可以稍后从“帮助 → 检查更新…”重试。\n\n\(error.localizedDescription)",
                    buttons: [("好", .alertFirstButtonReturn)]
                )
            } else {
                showAlert(
                    title: "安装更新失败",
                    message: error.localizedDescription,
                    buttons: [("好", .alertFirstButtonReturn)]
                )
            }
        }
    }

    private func downloadArchive(from url: URL, to destination: URL) async throws -> URLResponse {
        var request = URLRequest(
            url: url,
            cachePolicy: .reloadIgnoringLocalCacheData,
            timeoutInterval: Self.archiveDownloadTimeout
        )
        request.setValue("DeepSeek Harness Desk/\(Self.currentVersion)", forHTTPHeaderField: "User-Agent")
        request.setValue("application/octet-stream", forHTTPHeaderField: "Accept")
        return try await DownloadRunner.download(
            request: request,
            using: updateSession,
            to: destination,
            onProgress: { [weak self] progress in
                Task { @MainActor [weak self] in
                    self?.updateDownloadProgress(progress)
                }
            }
        )
    }

    private func validateArchive(
        at archiveURL: URL,
        expectedDigest: String?
    ) async throws {
        let result = try await ProcessRunner.run(
            executableURL: URL(fileURLWithPath: "/usr/bin/unzip"),
            arguments: ["-tq", archiveURL.path]
        )
        guard result.succeeded else {
            let details = result.output.trimmingCharacters(in: .whitespacesAndNewlines)
            throw UpdateError.server(
                details.isEmpty ? "更新压缩包校验失败" : "更新压缩包校验失败：\(details)"
            )
        }

        guard let expectedDigest else { return }
        let normalizedExpectedDigest = expectedDigest
            .split(separator: ":", maxSplits: 1, omittingEmptySubsequences: false)
            .last
            .map(String.init) ?? expectedDigest
        let digestResult = try await ProcessRunner.run(
            executableURL: URL(fileURLWithPath: "/usr/bin/shasum"),
            arguments: ["-a", "256", archiveURL.path]
        )
        guard digestResult.succeeded,
              let actualDigest = digestResult.output
                .split(whereSeparator: \.isWhitespace)
                .first,
              actualDigest.caseInsensitiveCompare(normalizedExpectedDigest) == .orderedSame else {
            throw UpdateError.server("更新包完整性校验失败，请重新检查更新")
        }
    }

    private func validateUpdatedApp(
        _ updatedApp: URL,
        expectedVersion: String
    ) async throws {
        let infoPlist = updatedApp.appendingPathComponent("Contents/Info.plist")
        guard fileManager.fileExists(atPath: infoPlist.path),
              let bundle = Bundle(url: updatedApp),
              let updatedVersion = bundle.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String,
              updatedVersion == expectedVersion,
              (bundle.object(forInfoDictionaryKey: "CFBundleIdentifier") as? String) == "com.deepseek-harness-desk.app" else {
            throw UpdateError.server("更新包中的 App 版本与 Release 不一致")
        }

        guard let executableName = bundle.object(forInfoDictionaryKey: "CFBundleExecutable") as? String else {
            throw UpdateError.server("更新包中的 App 缺少可执行文件信息")
        }
        let executableURL = updatedApp.appendingPathComponent("Contents/MacOS").appendingPathComponent(executableName)
        guard fileManager.isExecutableFile(atPath: executableURL.path) else {
            throw UpdateError.server("更新包中的 App 可执行文件不完整")
        }

        let result = try await ProcessRunner.run(
            executableURL: URL(fileURLWithPath: "/usr/bin/codesign"),
            arguments: ["--verify", "--deep", "--strict", updatedApp.path]
        )
        guard result.succeeded else {
            let details = result.output.trimmingCharacters(in: .whitespacesAndNewlines)
            throw UpdateError.server(
                details.isEmpty ? "更新包签名校验失败" : "更新包签名校验失败：\(details)"
            )
        }
    }

    private func beginInstallation(for version: String) {
        installProgress = nil
        installStep = ""
        installProgressLabel = ""
        installLogs.removeAll(keepingCapacity: true)
        appendInstallLog("开始安装 App 更新 \(version)")
    }

    private func setInstallStep(_ message: String) {
        installStep = message
        installProgress = nil
        installProgressLabel = ""
        status = message
        if installLogs.last != message {
            appendInstallLog(message)
        }
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
        logger.append("[App 更新] \(normalized)", to: .update)
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

    private func showUpdateAlert(for release: Release) {
        let alert = NSAlert()
        alert.messageText = "发现 DeepSeek Harness Desk 新版本"
        alert.informativeText = "版本 \(release.version) 已发布。现在下载并安装吗？"
        alert.alertStyle = .informational
        alert.addButton(withTitle: "更新")
        alert.addButton(withTitle: "稍后")
        let response = alert.runModal()
        if response == .alertFirstButtonReturn {
            Task { await installAvailableUpdate() }
        }
    }

    private func showAlert(
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

    nonisolated static func findApplication(in directory: URL) -> URL? {
        let fileManager = FileManager.default
        guard let enumerator = fileManager.enumerator(
            at: directory,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        ) else { return nil }

        for case let url as URL in enumerator where url.pathExtension == "app" {
            let infoPlist = url.appendingPathComponent("Contents/Info.plist")
            let executableDirectory = url.appendingPathComponent("Contents/MacOS")
            guard fileManager.fileExists(atPath: infoPlist.path),
                  fileManager.fileExists(atPath: executableDirectory.path) else {
                continue
            }
            return url
        }
        return nil
    }

    private func launchReplacementScript(
        currentApp: URL,
        updatedApp: URL,
        oldProcessID: Int32
    ) throws {
        let scriptURL = fileManager.temporaryDirectory
            .appendingPathComponent("deepseek-harness-desk-update-\(UUID().uuidString).sh")
        let script = """
        #!/bin/sh
        set -eu
        current_app="$1"
        updated_app="$2"
        old_pid="$3"
        script_path="$0"
        backup_app="${current_app}.backup.${old_pid}.$$"

        wait_count=0
        while kill -0 "$old_pid" 2>/dev/null; do
            if [ "$wait_count" -ge 240 ]; then
                exit 1
            fi
            sleep 0.25
            wait_count=$((wait_count + 1))
        done

        if [ ! -f "$updated_app/Contents/Info.plist" ]; then
            exit 1
        fi
        if ! /usr/bin/codesign --verify --deep --strict "$updated_app" >/dev/null 2>&1; then
            exit 1
        fi

        /bin/mv "$current_app" "$backup_app"
        if ! /bin/mv "$updated_app" "$current_app"; then
            /bin/mv "$backup_app" "$current_app"
            exit 1
        fi

        if ! /usr/bin/open "$current_app"; then
            /bin/mv "$current_app" "${current_app}.failed.${old_pid}.$$"
            /bin/mv "$backup_app" "$current_app"
            /usr/bin/open "$current_app" >/dev/null 2>&1 || true
            exit 1
        fi

        new_pid=""
        launch_count=0
        while [ "$launch_count" -lt 40 ]; do
            new_pid=$(/usr/bin/pgrep -x "DeepSeek Harness Desk" | /usr/bin/awk -v old_pid="$old_pid" '$1 != old_pid { print $1; exit }' || true)
            if [ -n "$new_pid" ]; then
                break
            fi
            sleep 0.5
            launch_count=$((launch_count + 1))
        done

        if [ -z "$new_pid" ]; then
            /bin/mv "$current_app" "${current_app}.failed.${old_pid}.$$"
            /bin/mv "$backup_app" "$current_app"
            /usr/bin/open "$current_app" >/dev/null 2>&1 || true
            exit 1
        fi

        (
            sleep 10
            /bin/rm -rf "$backup_app"
            /bin/rm -f "$script_path"
        ) >/dev/null 2>&1 &
        """
        try Data(script.utf8).write(to: scriptURL, options: .atomic)
        try fileManager.setAttributes(
            [.posixPermissions: NSNumber(value: 0o700)],
            ofItemAtPath: scriptURL.path
        )

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = [scriptURL.path, currentApp.path, updatedApp.path, String(oldProcessID)]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
    }

    nonisolated static func isNewer(_ candidate: String, than current: String) -> Bool {
        func parse(_ value: String) -> (core: [Int], prerelease: [String]) {
            var normalized = value.trimmingCharacters(in: .whitespacesAndNewlines)
            if normalized.first == "v" || normalized.first == "V" {
                normalized.removeFirst()
            }
            normalized = normalized.split(separator: "+", maxSplits: 1, omittingEmptySubsequences: false)
                .first.map(String.init) ?? normalized

            let versionParts = normalized.split(
                separator: "-",
                maxSplits: 1,
                omittingEmptySubsequences: false
            )
            let core = versionParts.first?.split(separator: ".").map { Int($0) ?? 0 } ?? []
            let prerelease = versionParts.count > 1
                ? versionParts[1].split(separator: ".").map(String.init)
                : []
            return (core, prerelease)
        }

        let candidateVersion = parse(candidate)
        let currentVersion = parse(current)
        for index in 0..<max(candidateVersion.core.count, currentVersion.core.count) {
            let candidateValue = index < candidateVersion.core.count ? candidateVersion.core[index] : 0
            let currentValue = index < currentVersion.core.count ? currentVersion.core[index] : 0
            if candidateValue != currentValue {
                return candidateValue > currentValue
            }
        }

        if candidateVersion.prerelease.isEmpty != currentVersion.prerelease.isEmpty {
            return candidateVersion.prerelease.isEmpty
        }

        for index in 0..<max(candidateVersion.prerelease.count, currentVersion.prerelease.count) {
            guard index < candidateVersion.prerelease.count else { return false }
            guard index < currentVersion.prerelease.count else { return true }

            let candidateIdentifier = candidateVersion.prerelease[index]
            let currentIdentifier = currentVersion.prerelease[index]
            if candidateIdentifier == currentIdentifier { continue }

            let candidateNumber = Int(candidateIdentifier)
            let currentNumber = Int(currentIdentifier)
            switch (candidateNumber, currentNumber) {
            case let (candidateNumber?, currentNumber?):
                return candidateNumber > currentNumber
            case (_?, nil):
                return false
            case (nil, _?):
                return true
            case (nil, nil):
                return candidateIdentifier > currentIdentifier
            }
        }
        return false
    }
}

private enum UpdateError: LocalizedError {
    case server(String)

    var errorDescription: String? {
        switch self {
        case let .server(message): return message
        }
    }
}
