import Foundation

enum ProcessRunner {
    struct Result: Sendable {
        let terminationStatus: Int32
        let output: String

        var succeeded: Bool {
            terminationStatus == 0
        }
    }

    enum RunnerError: LocalizedError {
        case launchFailed(String)

        var errorDescription: String? {
            switch self {
            case let .launchFailed(message):
                return message
            }
        }
    }

    private final class OutputBuffer: @unchecked Sendable {
        private let lock = NSLock()
        private var data = Data()
        private let onOutput: (@Sendable (String) -> Void)?

        init(onOutput: (@Sendable (String) -> Void)?) {
            self.onOutput = onOutput
        }

        func append(_ data: Data) {
            guard !data.isEmpty else { return }
            lock.lock()
            self.data.append(data)
            lock.unlock()
            onOutput?(String(decoding: data, as: UTF8.self))
        }

        func text() -> String {
            lock.lock()
            defer { lock.unlock() }
            return String(decoding: data, as: UTF8.self)
        }
    }

    static func run(
        executableURL: URL,
        arguments: [String],
        environment: [String: String]? = nil,
        currentDirectoryURL: URL? = nil,
        onOutput: (@Sendable (String) -> Void)? = nil
    ) async throws -> Result {
        try await withCheckedThrowingContinuation { continuation in
            let process = Process()
            let outputPipe = Pipe()
            let errorPipe = Pipe()
            let outputBuffer = OutputBuffer(onOutput: onOutput)

            outputPipe.fileHandleForReading.readabilityHandler = { handle in
                outputBuffer.append(handle.availableData)
            }
            errorPipe.fileHandleForReading.readabilityHandler = { handle in
                outputBuffer.append(handle.availableData)
            }

            process.executableURL = executableURL
            process.arguments = arguments
            process.environment = environment
            process.currentDirectoryURL = currentDirectoryURL
            process.standardOutput = outputPipe
            process.standardError = errorPipe
            process.terminationHandler = { terminatedProcess in
                outputPipe.fileHandleForReading.readabilityHandler = nil
                errorPipe.fileHandleForReading.readabilityHandler = nil
                outputBuffer.append(outputPipe.fileHandleForReading.readDataToEndOfFile())
                outputBuffer.append(errorPipe.fileHandleForReading.readDataToEndOfFile())

                continuation.resume(
                    returning: Result(
                        terminationStatus: terminatedProcess.terminationStatus,
                        output: outputBuffer.text()
                    )
                )
            }

            do {
                try process.run()
            } catch {
                outputPipe.fileHandleForReading.readabilityHandler = nil
                errorPipe.fileHandleForReading.readabilityHandler = nil
                continuation.resume(
                    throwing: RunnerError.launchFailed(
                        "无法启动 \(executableURL.lastPathComponent)：\(error.localizedDescription)"
                    )
                )
            }
        }
    }
}

enum DownloadRunner {
    struct Progress: Sendable {
        let bytesWritten: Int64
        let totalBytes: Int64?

        var fractionCompleted: Double? {
            guard let totalBytes, totalBytes > 0 else { return nil }
            return min(max(Double(bytesWritten) / Double(totalBytes), 0), 1)
        }
    }

    enum DownloadError: LocalizedError {
        case invalidResponse
        case cannotCreateFile(URL)

        var errorDescription: String? {
            switch self {
            case .invalidResponse:
                return "下载服务器返回了无效响应。"
            case let .cannotCreateFile(url):
                return "无法创建下载文件：\(url.lastPathComponent)"
            }
        }
    }

    static func download(
        request: URLRequest,
        using session: URLSession = .shared,
        to destination: URL,
        onProgress: @escaping @Sendable (Progress) -> Void = { _ in }
    ) async throws -> HTTPURLResponse {
        let fileManager = FileManager.default
        try fileManager.createDirectory(
            at: destination.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        if fileManager.fileExists(atPath: destination.path) {
            try fileManager.removeItem(at: destination)
        }
        guard fileManager.createFile(atPath: destination.path, contents: nil) else {
            throw DownloadError.cannotCreateFile(destination)
        }

        let fileHandle = try FileHandle(forWritingTo: destination)
        var completed = false
        defer {
            try? fileHandle.close()
            if !completed {
                try? fileManager.removeItem(at: destination)
            }
        }

        let (bytes, response) = try await session.bytes(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw DownloadError.invalidResponse
        }

        let totalBytes = httpResponse.expectedContentLength > 0
            ? httpResponse.expectedContentLength
            : nil
        var bytesWritten: Int64 = 0
        var nextProgressReport: Int64 = 0
        var buffer = Data()
        buffer.reserveCapacity(64 * 1024)

        onProgress(Progress(bytesWritten: 0, totalBytes: totalBytes))
        for try await byte in bytes {
            try Task.checkCancellation()
            buffer.append(byte)
            bytesWritten += 1

            if buffer.count >= 64 * 1024 {
                try fileHandle.write(contentsOf: buffer)
                buffer.removeAll(keepingCapacity: true)
            }

            if bytesWritten >= nextProgressReport {
                onProgress(Progress(bytesWritten: bytesWritten, totalBytes: totalBytes))
                nextProgressReport = bytesWritten + 32 * 1024
            }
        }

        if !buffer.isEmpty {
            try fileHandle.write(contentsOf: buffer)
        }
        onProgress(Progress(bytesWritten: bytesWritten, totalBytes: totalBytes))
        completed = true
        return httpResponse
    }
}
