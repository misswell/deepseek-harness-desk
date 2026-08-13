import Foundation

struct CrashRecoveryPolicy: Equatable, Sendable {
    let maximumFailures: Int
    let window: TimeInterval
    let delays: [TimeInterval]

    static let `default` = CrashRecoveryPolicy(
        maximumFailures: 3,
        window: 60,
        delays: [1, 2, 5]
    )

    func delay(for failureCount: Int) -> TimeInterval? {
        guard failureCount > 0, failureCount <= maximumFailures else {
            return nil
        }
        return delays[min(failureCount - 1, delays.count - 1)]
    }
}
