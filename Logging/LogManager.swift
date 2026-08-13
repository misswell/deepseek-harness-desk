import Foundation

final class LogManager: @unchecked Sendable {
    enum LogFile: String {
        case desk
        case harness
        case update

        var filename: String {
            "\(rawValue).log"
        }
    }

    let logsDirectory: URL

    private let lock = NSLock()
    private let maxFileSize: UInt64 = 10 * 1024 * 1024
    private let maximumRotatedFiles = 5

    init(logsDirectory: URL = PathUtils.logsDirectory) {
        self.logsDirectory = logsDirectory
        try? FileManager.default.createDirectory(
            at: logsDirectory,
            withIntermediateDirectories: true
        )
    }

    func append(_ message: String, to file: LogFile = .desk) {
        let redactedMessage = Self.redact(message)
        let timestamp = ISO8601DateFormatter().string(from: Date())
        let line = "[\(timestamp)] \(redactedMessage)\n"
        appendRaw(Data(line.utf8), to: file)
    }

    func append(data: Data, to file: LogFile = .harness) {
        guard !data.isEmpty else { return }
        let message = String(decoding: data, as: UTF8.self)
        append(message, to: file)
    }

    private func appendRaw(_ data: Data, to file: LogFile) {
        lock.lock()
        defer { lock.unlock() }

        let fileURL = logsDirectory.appendingPathComponent(file.filename)
        rotateIfNeeded(fileURL: fileURL)

        if !FileManager.default.fileExists(atPath: fileURL.path) {
            FileManager.default.createFile(atPath: fileURL.path, contents: nil)
        }

        do {
            let handle = try FileHandle(forWritingTo: fileURL)
            try handle.seekToEnd()
            try handle.write(contentsOf: data)
            try handle.close()
        } catch {
            // Logging must never bring down the process that is being supervised.
        }
    }

    private func rotateIfNeeded(fileURL: URL) {
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: fileURL.path),
              let fileSize = attributes[.size] as? UInt64,
              fileSize >= maxFileSize else {
            return
        }

        let fileManager = FileManager.default
        for index in stride(from: maximumRotatedFiles - 1, through: 1, by: -1) {
            let source = rotatedURL(for: fileURL, index: index)
            let destination = rotatedURL(for: fileURL, index: index + 1)
            if fileManager.fileExists(atPath: destination.path) {
                try? fileManager.removeItem(at: destination)
            }
            if fileManager.fileExists(atPath: source.path) {
                try? fileManager.moveItem(at: source, to: destination)
            }
        }

        let firstRotated = rotatedURL(for: fileURL, index: 1)
        if fileManager.fileExists(atPath: firstRotated.path) {
            try? fileManager.removeItem(at: firstRotated)
        }
        try? fileManager.moveItem(at: fileURL, to: firstRotated)
    }

    private func rotatedURL(for fileURL: URL, index: Int) -> URL {
        fileURL.deletingPathExtension()
            .appendingPathExtension("\(index).log")
    }

    private static func redact(_ message: String) -> String {
        var result = message
        let patterns = [
            #"(?i)(authorization\s*[:=]\s*bearer\s+)[^\s,;]+"#,
            #"(?i)((?:api[_-]?key|token|password|secret)\s*[:=]\s*)[^\s,;]+"#
        ]

        for pattern in patterns {
            guard let expression = try? NSRegularExpression(pattern: pattern) else { continue }
            let range = NSRange(result.startIndex..<result.endIndex, in: result)
            result = expression.stringByReplacingMatches(
                in: result,
                range: range,
                withTemplate: "$1[REDACTED]"
            )
        }
        return result
    }
}
