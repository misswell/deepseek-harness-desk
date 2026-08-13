import Foundation

enum PathUtils {
    static var logsDirectory: URL {
        FileManager.default
            .urls(for: .libraryDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Logs/DeepSeek Harness Desk", isDirectory: true)
    }

    static var applicationSupportDirectory: URL {
        FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("DeepSeek Harness Desk", isDirectory: true)
    }

    static func findExecutable(
        named name: String,
        environment: [String: String] = ProcessInfo.processInfo.environment
    ) -> URL? {
        var directories = environment["PATH"]?
            .split(separator: ":", omittingEmptySubsequences: true)
            .map(String.init) ?? []

        let fallbackDirectories = [
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin"
        ]
        directories.append(contentsOf: fallbackDirectories)

        var seen = Set<String>()
        for directory in directories where seen.insert(directory).inserted {
            let candidate = URL(fileURLWithPath: directory)
                .appendingPathComponent(name, isDirectory: false)
            if FileManager.default.isExecutableFile(atPath: candidate.path) {
                return candidate
            }
        }
        return nil
    }
}
