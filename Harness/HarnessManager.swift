import Foundation
import SwiftUI

@MainActor
final class HarnessManager: ObservableObject {
    @Published private(set) var state: HarnessState = .stopped
    @Published private(set) var pid: Int32?
    @Published private(set) var port: UInt16?
    @Published private(set) var serverURL: URL?
    @Published private(set) var runtimeVersion = "Development PATH"
    @Published private(set) var startTime: Date?
    @Published private(set) var lastExitCode: Int32?
    @Published private(set) var lastError: String?
    @Published private(set) var restartCount = 0

    var hasRunningProcess: Bool {
        process?.isRunning == true
    }

    private let runtimeManager: RuntimeManager
    private let logger: LogManager
    private let crashRecoveryPolicy: CrashRecoveryPolicy

    private var process: Process?
    private var stdoutPipe: Pipe?
    private var stderrPipe: Pipe?
    private var activeGeneration = UUID()
    private var intentionalStop = false
    private var recoveryTask: Task<Void, Never>?
    private var recentCrashDates: [Date] = []

    init(
        runtimeManager: RuntimeManager,
        logger: LogManager,
        crashRecoveryPolicy: CrashRecoveryPolicy = .default
    ) {
        self.runtimeManager = runtimeManager
        self.logger = logger
        self.crashRecoveryPolicy = crashRecoveryPolicy
    }

    deinit {
        stdoutPipe?.fileHandleForReading.readabilityHandler = nil
        stderrPipe?.fileHandleForReading.readabilityHandler = nil
    }

    func start() async {
        guard process?.isRunning != true else {
            state = .running
            return
        }
        guard !state.isBusy else { return }

        recoveryTask?.cancel()
        recoveryTask = nil
        intentionalStop = false
        lastError = nil
        state = .starting
        runtimeManager.refresh()

        guard let executableURL = runtimeManager.dshExecutableURL else {
            fail(with: "尚未安装 DeepSeek Harness 运行时。请点击“一键安装运行时”完成安装。")
            return
        }

        guard let selectedPort = PortScanner.firstAvailable() else {
            fail(with: "3080–3099 端口均不可用。请关闭占用这些端口的应用后重试。")
            return
        }

        let generation = UUID()
        activeGeneration = generation
        let candidate = Process()
        let outputPipe = Pipe()
        let errorPipe = Pipe()

        candidate.executableURL = executableURL
        candidate.arguments = ["web", "--port", String(selectedPort)]
        candidate.currentDirectoryURL = FileManager.default.homeDirectoryForCurrentUser
        candidate.environment = runtimeManager.processEnvironment()
        candidate.standardOutput = outputPipe
        candidate.standardError = errorPipe

        outputPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty else { return }
            self?.logger.append(data: data, to: .harness)
        }
        errorPipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty else { return }
            self?.logger.append(data: data, to: .harness)
        }

        candidate.terminationHandler = { [weak self] terminatedProcess in
            let exitCode = terminatedProcess.terminationStatus
            Task { @MainActor [weak self] in
                await self?.handleTermination(
                    of: terminatedProcess,
                    exitCode: exitCode,
                    generation: generation
                )
            }
        }

        process = candidate
        stdoutPipe = outputPipe
        stderrPipe = errorPipe
        port = selectedPort
        serverURL = URL(string: "http://127.0.0.1:\(selectedPort)")
        runtimeVersion = runtimeManager.isUsingManagedRuntime
            ? "Managed Runtime · \(runtimeManager.managedHarnessVersion)"
            : "系统 PATH · \(executableURL.lastPathComponent)"
        lastExitCode = nil

        logger.append(
            "Launching \(executableURL.path) web --port \(selectedPort)",
            to: .desk
        )

        do {
            try candidate.run()
            pid = candidate.processIdentifier
            startTime = Date()
        } catch {
            cleanupProcessReferences()
            fail(with: "启动 dsh 失败：\(error.localizedDescription)")
            return
        }

        guard let url = serverURL else {
            await stop()
            fail(with: "无法构造 Harness 服务地址。")
            return
        }

        do {
            try await HarnessHealthMonitor.waitUntilHealthy(at: url)
        } catch is CancellationError {
            return
        } catch {
            guard activeGeneration == generation, process === candidate else { return }
            logger.append("Health check failed: \(error.localizedDescription)", to: .desk)
            await stop()
            fail(with: "DeepSeek Harness 启动超时：\(error.localizedDescription)")
            return
        }

        guard activeGeneration == generation, process === candidate, candidate.isRunning else {
            return
        }

        state = .running
        recentCrashDates.removeAll()
        logger.append("Harness is healthy at \(url.absoluteString)", to: .desk)
    }

    func stop() async {
        recoveryTask?.cancel()
        recoveryTask = nil

        guard let currentProcess = process else {
            state = .stopped
            clearRuntimeMetadata()
            return
        }

        intentionalStop = true
        state = .stopping
        logger.append("Stopping Harness pid=\(currentProcess.processIdentifier)", to: .desk)
        await Task.detached(priority: .userInitiated) {
            ProcessUtils.terminateProcessTree(rootPID: currentProcess.processIdentifier)
        }.value

        let deadline = Date().addingTimeInterval(5)
        while currentProcess.isRunning, Date() < deadline {
            try? await Task.sleep(nanoseconds: 100_000_000)
        }

        if currentProcess.isRunning {
            logger.append("Force-stopping Harness pid=\(currentProcess.processIdentifier)", to: .desk)
            await Task.detached(priority: .userInitiated) {
                ProcessUtils.terminateProcessTree(
                    rootPID: currentProcess.processIdentifier,
                    force: true
                )
            }.value
            while currentProcess.isRunning {
                try? await Task.sleep(nanoseconds: 100_000_000)
            }
        }

        if process === currentProcess {
            cleanupProcessReferences()
            state = .stopped
            intentionalStop = false
        }
    }

    func restart() async {
        restartCount += 1
        await stop()
        await start()
    }

    func forceStop() async {
        guard let currentProcess = process else {
            state = .stopped
            return
        }
        intentionalStop = true
        await Task.detached(priority: .userInitiated) {
            ProcessUtils.terminateProcessTree(
                rootPID: currentProcess.processIdentifier,
                force: true
            )
        }.value
        while currentProcess.isRunning {
            try? await Task.sleep(nanoseconds: 50_000_000)
        }
        cleanupProcessReferences()
        state = .stopped
        intentionalStop = false
    }

    private func handleTermination(
        of terminatedProcess: Process,
        exitCode: Int32,
        generation: UUID
    ) async {
        guard generation == activeGeneration, process === terminatedProcess else { return }

        let wasIntentional = intentionalStop || state == .stopping
        lastExitCode = exitCode
        cleanupProcessReferences()

        if wasIntentional {
            intentionalStop = false
            state = .stopped
            logger.append("Harness stopped with exit code \(exitCode)", to: .desk)
            return
        }

        state = .crashed
        logger.append("Harness exited unexpectedly with exit code \(exitCode)", to: .desk)
        scheduleCrashRecovery()
    }

    private func scheduleCrashRecovery() {
        let now = Date()
        recentCrashDates = recentCrashDates.filter {
            now.timeIntervalSince($0) <= crashRecoveryPolicy.window
        }
        recentCrashDates.append(now)

        guard let delay = crashRecoveryPolicy.delay(for: recentCrashDates.count) else {
            fail(with: "DeepSeek Harness 在 60 秒内多次崩溃，已停止自动重启。")
            return
        }

        let failureNumber = recentCrashDates.count
        recoveryTask = Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            guard !Task.isCancelled, let self else { return }
            self.logger.append("Crash recovery attempt \(failureNumber)", to: .desk)
            await self.start()
        }
    }

    private func fail(with message: String) {
        lastError = message
        state = .failed(message)
        logger.append(message, to: .desk)
    }

    private func cleanupProcessReferences() {
        stdoutPipe?.fileHandleForReading.readabilityHandler = nil
        stderrPipe?.fileHandleForReading.readabilityHandler = nil
        stdoutPipe = nil
        stderrPipe = nil
        process = nil
        pid = nil
        clearRuntimeMetadata()
        activeGeneration = UUID()
    }

    private func clearRuntimeMetadata() {
        port = nil
        serverURL = nil
        startTime = nil
    }
}
