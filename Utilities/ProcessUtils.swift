import Darwin
import Foundation

enum ProcessUtils {
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
