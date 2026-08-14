import Foundation
import Network
import XCTest
import WebKit
@testable import DeepSeek_Harness_Desk

final class DeepSeekHarnessDeskTests: XCTestCase {
    func testCrashRecoveryPolicyUsesBoundedBackoff() {
        let policy = CrashRecoveryPolicy.default

        XCTAssertEqual(policy.delay(for: 1), 1)
        XCTAssertEqual(policy.delay(for: 2), 2)
        XCTAssertEqual(policy.delay(for: 3), 5)
        XCTAssertNil(policy.delay(for: 0))
        XCTAssertNil(policy.delay(for: 4))
    }

    func testFindExecutableUsesProvidedPath() throws {
        let temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: temporaryDirectory,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: temporaryDirectory) }

        let executable = temporaryDirectory.appendingPathComponent("fake-dsh")
        try Data("#!/bin/sh\n".utf8).write(to: executable)
        try FileManager.default.setAttributes(
            [.posixPermissions: NSNumber(value: 0o755)],
            ofItemAtPath: executable.path
        )

        let result = PathUtils.findExecutable(
            named: "fake-dsh",
            environment: ["PATH": temporaryDirectory.path]
        )

        XCTAssertEqual(result?.path, executable.path)
    }

    func testPortScannerRejectsInvalidRange() {
        XCTAssertNil(PortScanner.firstAvailable(startingAt: 3099, endingAt: 3080))
    }

    func testManagedHarnessCommandMatchesOnlyExpectedPortRange() {
        let executablePath = "/Users/example/Library/Application Support/DeepSeek Harness Desk/runtime/dsh/.bin/dsh"
        let portRange: ClosedRange<UInt16> = 3080...3099

        XCTAssertTrue(
            ProcessUtils.isManagedHarnessCommand(
                "node \(executablePath) web --port 3092",
                executablePath: executablePath,
                portRange: portRange
            )
        )
        XCTAssertFalse(
            ProcessUtils.isManagedHarnessCommand(
                "node \(executablePath) web --port 3079",
                executablePath: executablePath,
                portRange: portRange
            )
        )
        XCTAssertFalse(
            ProcessUtils.isManagedHarnessCommand(
                "node /tmp/other-dsh web --port 3092",
                executablePath: executablePath,
                portRange: portRange
            )
        )
        XCTAssertFalse(
            ProcessUtils.isManagedHarnessCommand(
                "node \(executablePath) web --port 30920",
                executablePath: executablePath,
                portRange: portRange
            )
        )
        XCTAssertFalse(
            ProcessUtils.isManagedHarnessCommand(
                "node \(executablePath)-other web --port 3092",
                executablePath: executablePath,
                portRange: portRange
            )
        )
        XCTAssertFalse(
            ProcessUtils.isManagedHarnessCommand(
                "node \(executablePath) web --port 3092 --verbose",
                executablePath: executablePath,
                portRange: portRange
            )
        )
    }

    func testUpdateManagerComparesVersionsNumerically() {
        XCTAssertTrue(UpdateManager.isNewer("0.2.0", than: "0.1.0"))
        XCTAssertTrue(UpdateManager.isNewer("1.0.0", than: "0.99.9"))
        XCTAssertTrue(UpdateManager.isNewer("0.1.0", than: "0.1.0-rc.6"))
        XCTAssertTrue(UpdateManager.isNewer("0.1.0-rc.10", than: "0.1.0-rc.6"))
        XCTAssertFalse(UpdateManager.isNewer("0.1.0-rc.6", than: "0.1.0"))
        XCTAssertFalse(UpdateManager.isNewer("0.2.0", than: "0.2"))
        XCTAssertFalse(UpdateManager.isNewer("v0.2.0", than: "0.2.0"))
    }

    func testAutomaticUpdateCheckIntervalsMatchSettings() {
        XCTAssertEqual(UpdateManager.AutomaticCheckInterval.hourly.seconds, 60 * 60)
        XCTAssertEqual(UpdateManager.AutomaticCheckInterval.daily.seconds, 24 * 60 * 60)
        XCTAssertEqual(UpdateManager.AutomaticCheckInterval.weekly.seconds, 7 * 24 * 60 * 60)
    }

    @MainActor
    func testTerminationCoordinatorRepliesWhenGracefulStopHangs() async {
        let coordinator = ApplicationTerminationCoordinator(gracefulTimeout: 0.05)
        let replyExpectation = expectation(description: "termination reply")
        var forceStopCount = 0
        var replyCount = 0

        let decision = coordinator.requestTermination(
            hasRunningProcess: true,
            stop: {
                try? await Task.sleep(nanoseconds: 500_000_000)
            },
            forceStop: {
                forceStopCount += 1
            },
            reply: {
                replyCount += 1
                replyExpectation.fulfill()
            }
        )

        XCTAssertEqual(decision, .terminateLater)
        await fulfillment(of: [replyExpectation], timeout: 0.25)
        XCTAssertEqual(forceStopCount, 1)
        XCTAssertEqual(replyCount, 1)

        let duplicateDecision = coordinator.requestTermination(
            hasRunningProcess: true,
            stop: {},
            forceStop: {
                forceStopCount += 1
            },
            reply: {
                replyCount += 1
            }
        )

        XCTAssertEqual(duplicateDecision, .terminateLater)
        try? await Task.sleep(nanoseconds: 50_000_000)
        XCTAssertEqual(forceStopCount, 1)
        XCTAssertEqual(replyCount, 1)
    }

    @MainActor
    func testDockReopenHandlesReopenWhenOnlySettingsWindowIsVisible() {
        let delegate = AppDelegate()
        let mainWindow = NSWindow(
            contentRect: .zero,
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: true
        )
        delegate.register(mainWindow: mainWindow)

        XCTAssertFalse(
            delegate.applicationShouldHandleReopen(NSApp, hasVisibleWindows: true),
            "A visible Settings window must not prevent the Dock click from opening the main window."
        )
    }

    @MainActor
    func testWebKitPolicyCancellationDoesNotShowLoadError() {
        let controller = WebViewController()
        let webView = WKWebView(frame: .zero)
        controller.attach(webView)
        controller.load(URL(string: "http://127.0.0.1:3080/")!)

        let error = NSError(
            domain: WKErrorDomain,
            code: 102,
            userInfo: [NSLocalizedDescriptionKey: "Frame load interrupted"]
        )
        controller.navigationDidFail(with: error)

        XCTAssertNil(controller.loadErrorMessage)
    }

    func testWebViewRecoveryRetriesTransientNavigationFailuresWithinBound() {
        let transientError = NSError(
            domain: NSURLErrorDomain,
            code: NSURLErrorCannotConnectToHost
        )
        let permanentError = NSError(domain: "HarnessWebView", code: 1)

        XCTAssertTrue(WebViewLoadRecovery.shouldRetry(transientError, attempt: 0))
        XCTAssertTrue(WebViewLoadRecovery.shouldRetry(transientError, attempt: 2))
        XCTAssertFalse(WebViewLoadRecovery.shouldRetry(transientError, attempt: 3))
        XCTAssertFalse(WebViewLoadRecovery.shouldRetry(permanentError, attempt: 0))
    }

    func testWebViewZoomUsesTenPercentStepsAndClampsToSupportedRange() {
        XCTAssertEqual(WebViewZoom.adjusted(1.0, by: 1), 1.1, accuracy: 0.001)
        XCTAssertEqual(WebViewZoom.adjusted(1.0, by: -1), 0.9, accuracy: 0.001)
        XCTAssertEqual(WebViewZoom.adjusted(1.96, by: 1), 2.0, accuracy: 0.001)
        XCTAssertEqual(WebViewZoom.adjusted(0.51, by: -1), 0.5, accuracy: 0.001)
        XCTAssertEqual(WebViewZoom.clamped(9.0), 2.0, accuracy: 0.001)
        XCTAssertEqual(WebViewZoom.clamped(0.1), 0.5, accuracy: 0.001)
    }

    func testDownloadProgressReportsFractionWhenContentLengthIsKnown() {
        let progress = DownloadRunner.Progress(bytesWritten: 25, totalBytes: 100)

        XCTAssertEqual(progress.fractionCompleted, 0.25)
    }

    func testFindApplicationSkipsMetadataOnlyAppBundles() throws {
        let temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(
            at: temporaryDirectory,
            withIntermediateDirectories: true
        )
        defer { try? FileManager.default.removeItem(at: temporaryDirectory) }

        let metadataOnlyApp = temporaryDirectory
            .appendingPathComponent("Broken.app/Contents", isDirectory: true)
        try FileManager.default.createDirectory(
            at: metadataOnlyApp,
            withIntermediateDirectories: true
        )
        try Data().write(to: metadataOnlyApp.appendingPathComponent("._Info.plist"))

        let validApp = temporaryDirectory
            .appendingPathComponent("Valid.app/Contents/MacOS", isDirectory: true)
        try FileManager.default.createDirectory(
            at: validApp,
            withIntermediateDirectories: true
        )
        try Data("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict/></plist>".utf8)
            .write(to: validApp.deletingLastPathComponent().appendingPathComponent("Info.plist"))

        XCTAssertEqual(
            UpdateManager.findApplication(in: temporaryDirectory)?.lastPathComponent,
            "Valid.app"
        )
    }

    func testLogManagerRedactsSecrets() throws {
        let temporaryDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        let manager = LogManager(logsDirectory: temporaryDirectory)
        defer { try? FileManager.default.removeItem(at: temporaryDirectory) }

        manager.append(
            "Authorization: Bearer abc123 api_key=secret-value password=hunter2",
            to: .desk
        )

        let log = try String(
            contentsOf: temporaryDirectory.appendingPathComponent("desk.log"),
            encoding: .utf8
        )
        XCTAssertTrue(log.contains("Bearer [REDACTED]"))
        XCTAssertTrue(log.contains("api_key=[REDACTED]"))
        XCTAssertTrue(log.contains("password=[REDACTED]"))
        XCTAssertFalse(log.contains("abc123"))
        XCTAssertFalse(log.contains("hunter2"))
    }

    func testHealthCheckTimesOutForUnavailablePort() async {
        let url = URL(string: "http://127.0.0.1:9")!

        do {
            try await HarnessHealthMonitor.waitUntilHealthy(
                at: url,
                timeout: 0.15,
                interval: 0.01
            )
            XCTFail("Expected the health check to time out")
        } catch let error as HarnessHealthMonitor.HealthError {
            guard case .timeout = error else {
                return XCTFail("Unexpected health error: \(error)")
            }
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testHealthCheckSucceedsForLocalHTTPServer() async throws {
        let server = try LocalHTTPServer(statusCode: 200)

        try await HarnessHealthMonitor.waitUntilHealthy(
            at: server.url,
            timeout: 1,
            interval: 0.01
        )
    }

    func testHealthCheckTreatsNonSuccessResponseAsUnavailable() async throws {
        let server = try LocalHTTPServer(statusCode: 503)

        do {
            try await HarnessHealthMonitor.waitUntilHealthy(
                at: server.url,
                timeout: 0.2,
                interval: 0.01
            )
            XCTFail("Expected a non-success response to time out")
        } catch let error as HarnessHealthMonitor.HealthError {
            guard case .timeout = error else {
                return XCTFail("Unexpected health error: \(error)")
            }
        }
    }

    func testProcessRunnerCapturesStandardOutputAndError() async throws {
        let result = try await ProcessRunner.run(
            executableURL: URL(fileURLWithPath: "/bin/sh"),
            arguments: ["-c", "printf '%*s' 131072 ''; printf '%*s' 131072 '' >&2"]
        )

        XCTAssertTrue(result.succeeded)
        XCTAssertEqual(result.output.count, 262144)
    }

    func testProcessRunnerReportsLaunchFailure() async {
        do {
            _ = try await ProcessRunner.run(
                executableURL: URL(fileURLWithPath: "/tmp/deepseek-harness-desk-does-not-exist"),
                arguments: []
            )
            XCTFail("Expected ProcessRunner to report a launch failure")
        } catch let error as ProcessRunner.RunnerError {
            guard case .launchFailed = error else {
                return XCTFail("Unexpected runner error: \(error)")
            }
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testTerminateProcessTreeAndWaitStopsOwnedProcess() throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = ["-c", "sleep 30"]

        try process.run()
        defer {
            if process.isRunning {
                ProcessUtils.terminateProcessTree(rootPID: process.processIdentifier, force: true)
            }
            process.waitUntilExit()
        }

        XCTAssertTrue(
            ProcessUtils.terminateProcessTreeAndWait(
                rootPID: process.processIdentifier,
                timeout: 2
            )
        )
        XCTAssertFalse(process.isRunning)
    }
}

private final class LocalHTTPServer {
    enum ServerError: Error {
        case failedToStart
        case missingPort
    }

    private(set) var url: URL

    private let statusCode: Int
    private let listener: NWListener
    private let queue = DispatchQueue(label: "DeepSeekHarnessDeskTests.LocalHTTPServer")
    private let ready = DispatchSemaphore(value: 0)
    private var startupError: NWError?

    init(statusCode: Int) throws {
        self.url = URL(string: "http://127.0.0.1:0/")!
        self.statusCode = statusCode
        self.listener = try NWListener(using: .tcp, on: .any)
        self.listener.stateUpdateHandler = { [weak self] state in
            switch state {
            case .ready, .failed:
                if case let .failed(error) = state {
                    self?.startupError = error
                }
                self?.ready.signal()
            default:
                break
            }
        }
        self.listener.newConnectionHandler = { [weak self] connection in
            self?.handle(connection)
        }
        self.listener.start(queue: queue)

        guard ready.wait(timeout: .now() + 2) == .success else {
            listener.cancel()
            throw ServerError.failedToStart
        }
        if startupError != nil {
            listener.cancel()
            throw ServerError.failedToStart
        }
        guard let port = listener.port?.rawValue,
              let url = URL(string: "http://127.0.0.1:\(port)/") else {
            listener.cancel()
            throw ServerError.missingPort
        }
        self.url = url
    }

    deinit {
        listener.cancel()
    }

    private func handle(_ connection: NWConnection) {
        connection.start(queue: queue)
        connection.receive(minimumIncompleteLength: 1, maximumLength: 8192) {
            [weak self] _, _, _, _ in
            guard let self else {
                connection.cancel()
                return
            }

            let body = "health"
            let response = "HTTP/1.1 \(self.statusCode) Test\r\nContent-Length: \(body.utf8.count)\r\nConnection: close\r\n\r\n\(body)"
            connection.send(
                content: Data(response.utf8),
                completion: .contentProcessed { _ in connection.cancel() }
            )
        }
    }
}
