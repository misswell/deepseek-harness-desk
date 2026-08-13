import AppKit
import Foundation
import SwiftUI

@MainActor
final class UpdateManager: ObservableObject {
    struct ReleaseAsset: Decodable {
        let name: String
        let browserDownloadURL: URL

        enum CodingKeys: String, CodingKey {
            case name
            case browserDownloadURL = "browser_download_url"
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
    static let currentVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0.2.0"

    @Published private(set) var isChecking = false
    @Published private(set) var isInstalling = false
    @Published private(set) var status = ""
    @Published private(set) var automaticallyChecksForUpdates: Bool
    @Published private(set) var availableRelease: Release?

    private let fileManager = FileManager.default
    private var hasStarted = false

    init() {
        if UserDefaults.standard.object(forKey: "autoCheckForUpdates") == nil {
            UserDefaults.standard.set(true, forKey: "autoCheckForUpdates")
        }
        automaticallyChecksForUpdates = UserDefaults.standard.bool(forKey: "autoCheckForUpdates")
    }

    func start() {
        guard !hasStarted else { return }
        hasStarted = true
        guard automaticallyChecksForUpdates else { return }

        Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled else { return }
            await self?.checkForUpdates(interactive: false)
        }
    }

    func setAutomaticChecks(_ enabled: Bool) {
        automaticallyChecksForUpdates = enabled
        UserDefaults.standard.set(enabled, forKey: "autoCheckForUpdates")
        if enabled && !hasStarted {
            start()
        }
    }

    func checkForUpdates(interactive: Bool = true) async {
        guard !isChecking, !isInstalling else { return }
        isChecking = true
        status = "正在检查更新…"
        defer { isChecking = false }

        do {
            var request = URLRequest(url: Self.releasesURL)
            request.setValue("DeepSeek Harness Desk/\(Self.currentVersion)", forHTTPHeaderField: "User-Agent")
            request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
            let (data, response) = try await URLSession.shared.data(for: request)
            if let httpResponse = response as? HTTPURLResponse,
               !(200..<300).contains(httpResponse.statusCode) {
                throw UpdateError.server("GitHub 返回 HTTP \(httpResponse.statusCode)")
            }

            let release = try JSONDecoder().decode(Release.self, from: data)
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

    func installAvailableUpdate() async {
        guard !isInstalling, let release = availableRelease,
              let asset = release.applicationArchive else { return }

        isInstalling = true
        status = "正在下载 DeepSeek Harness Desk \(release.version)…"
        defer { isInstalling = false }

        do {
            let (temporaryURL, response) = try await URLSession.shared.download(from: asset.browserDownloadURL)
            if let httpResponse = response as? HTTPURLResponse,
               !(200..<300).contains(httpResponse.statusCode) {
                throw UpdateError.server("下载更新失败（HTTP \(httpResponse.statusCode)）")
            }

            let updateRoot = fileManager.temporaryDirectory
                .appendingPathComponent("DeepSeekHarnessDesk-Update-\(UUID().uuidString)", isDirectory: true)
            try fileManager.createDirectory(at: updateRoot, withIntermediateDirectories: true)
            let archiveURL = updateRoot.appendingPathComponent(asset.name)
            try fileManager.moveItem(at: temporaryURL, to: archiveURL)

            let extractionURL = updateRoot.appendingPathComponent("extracted", isDirectory: true)
            try fileManager.createDirectory(at: extractionURL, withIntermediateDirectories: true)
            let result = try await ProcessRunner.run(
                executableURL: URL(fileURLWithPath: "/usr/bin/unzip"),
                arguments: ["-q", "-o", archiveURL.path, "-d", extractionURL.path]
            )
            guard result.succeeded else {
                throw UpdateError.server("解压更新失败：\(result.output.trimmingCharacters(in: .whitespacesAndNewlines))")
            }

            guard let updatedApp = findApplication(in: extractionURL) else {
                throw UpdateError.server("更新包中没有找到 App")
            }
            try launchReplacementScript(currentApp: Bundle.main.bundleURL, updatedApp: updatedApp)
            status = "更新已准备，App 即将重启"
            availableRelease = nil
            NSApp.terminate(nil)
        } catch is CancellationError {
            status = "更新下载已取消"
        } catch {
            status = "安装更新失败：\(error.localizedDescription)"
            showAlert(
                title: "安装更新失败",
                message: error.localizedDescription,
                buttons: [("好", .alertFirstButtonReturn)]
            )
        }
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

    private func findApplication(in directory: URL) -> URL? {
        guard let enumerator = fileManager.enumerator(
            at: directory,
            includingPropertiesForKeys: [.isDirectoryKey],
            options: [.skipsHiddenFiles]
        ) else { return nil }

        for case let url as URL in enumerator where url.pathExtension == "app" {
            return url
        }
        return nil
    }

    private func launchReplacementScript(currentApp: URL, updatedApp: URL) throws {
        let scriptURL = fileManager.temporaryDirectory
            .appendingPathComponent("deepseek-harness-desk-update-\(UUID().uuidString).sh")
        let script = """
        #!/bin/sh
        set -eu
        sleep 1
        current_app="$1"
        updated_app="$2"
        rm -rf "$current_app"
        mv "$updated_app" "$current_app"
        open "$current_app"
        rm -f "$0"
        """
        try Data(script.utf8).write(to: scriptURL, options: .atomic)
        try fileManager.setAttributes(
            [.posixPermissions: NSNumber(value: 0o700)],
            ofItemAtPath: scriptURL.path
        )

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = [scriptURL.path, currentApp.path, updatedApp.path]
        try process.run()
    }

    nonisolated static func isNewer(_ candidate: String, than current: String) -> Bool {
        func components(_ value: String) -> [Int] {
            value.split(separator: ".").map { part in
                Int(part.prefix { $0.isNumber }) ?? 0
            }
        }
        let candidateComponents = components(candidate)
        let currentComponents = components(current)
        for index in 0..<max(candidateComponents.count, currentComponents.count) {
            let candidateValue = index < candidateComponents.count ? candidateComponents[index] : 0
            let currentValue = index < currentComponents.count ? currentComponents[index] : 0
            if candidateValue != currentValue { return candidateValue > currentValue }
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
