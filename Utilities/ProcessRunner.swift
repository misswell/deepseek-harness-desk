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

    static func run(
        executableURL: URL,
        arguments: [String],
        environment: [String: String]? = nil,
        currentDirectoryURL: URL? = nil
    ) async throws -> Result {
        try await withCheckedThrowingContinuation { continuation in
            let process = Process()
            let outputPipe = Pipe()
            let errorPipe = Pipe()
            let outputLock = NSLock()
            var output = Data()

            func append(_ data: Data) {
                guard !data.isEmpty else { return }
                outputLock.lock()
                output.append(data)
                outputLock.unlock()
            }

            outputPipe.fileHandleForReading.readabilityHandler = { handle in
                append(handle.availableData)
            }
            errorPipe.fileHandleForReading.readabilityHandler = { handle in
                append(handle.availableData)
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
                append(outputPipe.fileHandleForReading.readDataToEndOfFile())
                append(errorPipe.fileHandleForReading.readDataToEndOfFile())

                outputLock.lock()
                let outputText = String(decoding: output, as: UTF8.self)
                outputLock.unlock()

                continuation.resume(
                    returning: Result(
                        terminationStatus: terminatedProcess.terminationStatus,
                        output: outputText
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
