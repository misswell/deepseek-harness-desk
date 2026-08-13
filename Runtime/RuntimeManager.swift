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
    static let harnessVersion = "0.1.0-rc.6"
    static let nodeVersion = "24.19.0"

    @Published private(set) var dshExecutableURL: URL?
    @Published private(set) var nodeExecutableURL: URL?
    @Published private(set) var status: String
    @Published private(set) var installState: RuntimeInstallState = .ready
    @Published private(set) var installationMessage = ""
    @Published private(set) var isUsingManagedRuntime = false

    var needsInstallation: Bool {
        dshExecutableURL == nil
    }

    var isInstalling: Bool {
        installState.isInstalling
    }

    private let fileManager = FileManager.default
    private var installationTask: Task<Void, Never>?

    init() {
        self.status = "正在检查 DeepSeek Harness"
        refresh()
    }

    deinit {
        installationTask?.cancel()
    }

    func refresh() {
        let managedDsh = managedDshExecutableURL
        if fileManager.isExecutableFile(atPath: managedDsh.path) {
            dshExecutableURL = managedDsh
            nodeExecutableURL = managedNodeExecutableURL
            status = "Managed Harness \(Self.harnessVersion)"
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
                    stagingDirectory: stagingDirectory
                )
            }

            guard fileManager.isExecutableFile(atPath: dshExecutable(at: dshRoot).path) else {
                throw RuntimeError.installationFailed("DeepSeek Harness 安装后未找到 dsh 命令。")
            }

            refresh()
            installationMessage = "安装完成"
            installState = .installed
        } catch is CancellationError {
            installationMessage = "安装已取消"
            installState = .ready
        } catch {
            let message = error.localizedDescription
            installationMessage = message
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
        try verifySHA256(of: archiveURL, expected: expectedSHA256)

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
        stagingDirectory: URL
    ) async throws {
        let npmURL = nodeRoot.appendingPathComponent("bin/npm")
        guard fileManager.isExecutableFile(atPath: npmURL.path) else {
            throw RuntimeError.installationFailed("Node.js 安装完成，但没有找到 npm。")
        }

        let dshStaging = stagingDirectory.appendingPathComponent("dsh", isDirectory: true)
        try fileManager.createDirectory(at: dshStaging, withIntermediateDirectories: true)
        setInstalling("正在安装 DeepSeek Harness \(Self.harnessVersion)…")
        let result = try await ProcessRunner.run(
            executableURL: npmURL,
            arguments: [
                "install",
                "--prefix", dshStaging.path,
                "--no-audit",
                "--no-fund",
                "--no-update-notifier",
                "--no-package-lock",
                "@deepseek-ai/dsh@\(Self.harnessVersion)"
            ],
            environment: processEnvironmentForNode(nodeRoot: nodeRoot),
            currentDirectoryURL: dshStaging
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
        setInstalling("正在下载运行时…")
        let (temporaryURL, response) = try await URLSession.shared.download(from: url)
        if let response = response as? HTTPURLResponse,
           !(200..<300).contains(response.statusCode) {
            throw RuntimeError.installationFailed("下载失败（HTTP \(response.statusCode)）。")
        }
        if fileManager.fileExists(atPath: destination.path) {
            try fileManager.removeItem(at: destination)
        }
        try fileManager.moveItem(at: temporaryURL, to: destination)
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
        installState = .installing(message)
        status = message
    }

    private var managedNodeExecutableURL: URL {
        nodeExecutable(at: managedNodeDirectory)
    }

    private var managedDshExecutableURL: URL {
        dshExecutable(at: managedDshDirectory)
    }

    private var managedNodeDirectory: URL {
        PathUtils.applicationSupportDirectory
            .appendingPathComponent("runtime/node/\(Self.nodeVersion)", isDirectory: true)
    }

    private var managedDshDirectory: URL {
        PathUtils.applicationSupportDirectory
            .appendingPathComponent("runtime/dsh/\(Self.harnessVersion)", isDirectory: true)
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

    var errorDescription: String? {
        switch self {
        case let .installationFailed(message):
            return message
        }
    }
}
