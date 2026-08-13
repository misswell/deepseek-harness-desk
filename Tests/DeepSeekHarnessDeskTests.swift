import Foundation
import XCTest
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

    func testUpdateManagerComparesVersionsNumerically() {
        XCTAssertTrue(UpdateManager.isNewer("0.2.0", than: "0.1.0"))
        XCTAssertTrue(UpdateManager.isNewer("1.0.0", than: "0.99.9"))
        XCTAssertFalse(UpdateManager.isNewer("0.2.0", than: "0.2"))
        XCTAssertFalse(UpdateManager.isNewer("v0.2.0", than: "0.2.0"))
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
}
