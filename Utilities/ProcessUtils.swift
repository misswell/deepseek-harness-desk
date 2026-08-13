import Darwin
import Foundation

enum ProcessUtils {
    static func managedHarnessProcessIDs(
        executableURL: URL,
        portRange: ClosedRange<UInt16>
    ) -> [Int32] {
        guard let output = commandOutput(
            executableURL: URL(fileURLWithPath: "/bin/ps"),
            arguments: ["-axo", "pid=,command="]
        ) else {
            return []
        }

        return output
            .split(whereSeparator: \.isNewline)
            .compactMap { line -> Int32? in
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                guard let separator = trimmed.firstIndex(where: \.isWhitespace),
                      let pid = Int32(trimmed[..<separator]) else {
                    return nil
                }

                let command = trimmed[separator...].trimmingCharacters(in: .whitespaces)
                return isManagedHarnessCommand(
                    command,
                    executablePath: executableURL.path,
                    portRange: portRange
                ) ? pid : nil
            }
    }

    static func isManagedHarnessCommand(
        _ command: String,
        executablePath: String,
        portRange: ClosedRange<UInt16>
    ) -> Bool {
        guard command.contains(executablePath),
              let portMarker = command.range(of: "web --port ") else {
            return false
        }

        let portAndArguments = command[portMarker.upperBound...]
        let portText = portAndArguments.prefix { !$0.isWhitespace }
        guard let port = UInt16(portText), portRange.contains(port) else {
            return false
        }

        return portAndArguments.dropFirst(portText.count).isEmpty ||
            portAndArguments.dropFirst(portText.count).first?.isWhitespace == true
    }

    static func isProcessAlive(_ pid: Int32) -> Bool {
        guard pid > 0 else { return false }
        if kill(pid, 0) == 0 {
            return true
        }
        return errno == EPERM
    }

    static func terminateManagedHarnessProcesses(
        executableURL: URL,
        portRange: ClosedRange<UInt16>,
        timeout: TimeInterval = 5
    ) -> [Int32] {
        let processIDs = managedHarnessProcessIDs(
            executableURL: executableURL,
            portRange: portRange
        )
        guard !processIDs.isEmpty else { return [] }

        for pid in processIDs {
            terminateProcessTree(rootPID: pid)
        }
        waitForExit(processIDs, timeout: min(timeout, 3))

        let remaining = processIDs.filter(isProcessAlive)
        if !remaining.isEmpty {
            for pid in remaining {
                terminateProcessTree(rootPID: pid, force: true)
            }
            waitForExit(remaining, timeout: max(0, timeout - min(timeout, 3)))
        }
        return processIDs
    }

    @discardableResult
    static func terminateProcessTreeAndWait(
        rootPID: Int32,
        timeout: TimeInterval = 3
    ) -> Bool {
        terminateProcessTree(rootPID: rootPID)
        waitForExit([rootPID], timeout: min(timeout, 2))

        guard isProcessAlive(rootPID) else { return true }
        terminateProcessTree(rootPID: rootPID, force: true)
        waitForExit([rootPID], timeout: max(0, timeout - min(timeout, 2)))
        return !isProcessAlive(rootPID)
    }

    static func descendantProcessIDs(of rootPID: Int32) -> [Int32] {
        var pending = [rootPID]
        var descendants: [Int32] = []

        while let parentPID = pending.popLast() {
            for childPID in childProcessIDs(of: parentPID) {
                guard childPID != rootPID, !descendants.contains(childPID) else { continue }
                descendants.append(childPID)
                pending.append(childPID)
            }
        }
        return descendants
    }

    static func terminateProcessTree(rootPID: Int32, force: Bool = false) {
        let descendants = descendantProcessIDs(of: rootPID)
        let signal = force ? SIGKILL : SIGTERM

        for pid in descendants.reversed() {
            _ = kill(pid, signal)
        }
        _ = kill(rootPID, signal)
    }

    private static func waitForExit(_ processIDs: [Int32], timeout: TimeInterval) {
        guard timeout > 0 else { return }
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline && processIDs.contains(where: isProcessAlive) {
            Thread.sleep(forTimeInterval: 0.05)
        }
    }

    private static func commandOutput(
        executableURL: URL,
        arguments: [String]
    ) -> String? {
        let command = Process()
        let output = Pipe()
        command.executableURL = executableURL
        command.arguments = arguments
        command.standardOutput = output
        command.standardError = FileHandle.nullDevice

        do {
            try command.run()
            let data = output.fileHandleForReading.readDataToEndOfFile()
            command.waitUntilExit()
            guard command.terminationStatus == 0 else { return nil }
            return String(data: data, encoding: .utf8)
        } catch {
            return nil
        }
    }

    private static func childProcessIDs(of parentPID: Int32) -> [Int32] {
        guard FileManager.default.isExecutableFile(atPath: "/usr/bin/pgrep") else {
            return []
        }

        let command = Process()
        let output = Pipe()
        command.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
        command.arguments = ["-P", String(parentPID)]
        command.standardOutput = output
        command.standardError = FileHandle.nullDevice

        do {
            try command.run()
            let data = output.fileHandleForReading.readDataToEndOfFile()
            command.waitUntilExit()
            guard command.terminationStatus == 0,
                  let text = String(data: data, encoding: .utf8) else {
                return []
            }
            return text
                .split(whereSeparator: \.isNewline)
                .compactMap { Int32($0.trimmingCharacters(in: .whitespacesAndNewlines)) }
        } catch {
            return []
        }
    }
}
