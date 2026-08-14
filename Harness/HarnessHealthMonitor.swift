import Foundation

enum HarnessHealthMonitor {
    enum HealthError: LocalizedError, Equatable {
        case timeout(URL)
        case invalidResponse

        var errorDescription: String? {
            switch self {
            case let .timeout(url):
                return "Harness did not become available at \(url.absoluteString) within 30 seconds."
            case .invalidResponse:
                return "Harness returned an invalid HTTP response."
            }
        }
    }

    static func waitUntilHealthy(
        at url: URL,
        timeout: TimeInterval = 30,
        interval: TimeInterval = 0.4
    ) async throws {
        let deadline = Date().addingTimeInterval(timeout)
        let session = makeLocalSession()
        defer { session.invalidateAndCancel() }

        while Date() < deadline {
            try Task.checkCancellation()

            var request = URLRequest(
                url: url,
                cachePolicy: .reloadIgnoringLocalCacheData,
                timeoutInterval: min(1.5, max(0.1, deadline.timeIntervalSinceNow))
            )
            request.setValue("no-cache", forHTTPHeaderField: "Cache-Control")
            request.setValue("DeepSeek-Harness-Desk-Health-Check", forHTTPHeaderField: "User-Agent")

            do {
                let (_, response) = try await session.data(for: request)
                guard let httpResponse = response as? HTTPURLResponse else {
                    throw HealthError.invalidResponse
                }
                if (200..<400).contains(httpResponse.statusCode) {
                    return
                }
            } catch is CancellationError {
                throw CancellationError()
            } catch let error as HealthError {
                throw error
            } catch {
                // The process may still be starting. Retry until the deadline.
            }

            let remaining = deadline.timeIntervalSinceNow
            if remaining > 0 {
                let sleepDuration = min(interval, remaining)
                try await Task.sleep(nanoseconds: UInt64(sleepDuration * 1_000_000_000))
            }
        }

        throw HealthError.timeout(url)
    }

    private static func makeLocalSession() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.waitsForConnectivity = false
        configuration.timeoutIntervalForRequest = 1.5
        configuration.timeoutIntervalForResource = 2
        configuration.connectionProxyDictionary = [
            "HTTPEnable": 0,
            "HTTPSEnable": 0,
            "SOCKSEnable": 0
        ]
        return URLSession(configuration: configuration)
    }
}
