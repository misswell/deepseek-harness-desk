import Foundation

enum HarnessState: Equatable, Sendable {
    case stopped
    case starting
    case running
    case stopping
    case crashed
    case failed(String)

    var title: String {
        switch self {
        case .stopped:
            return "Stopped"
        case .starting:
            return "Starting"
        case .running:
            return "Running"
        case .stopping:
            return "Stopping"
        case .crashed:
            return "Crashed"
        case .failed:
            return "Failed"
        }
    }

    var isBusy: Bool {
        switch self {
        case .starting, .stopping:
            return true
        case .stopped, .running, .crashed, .failed:
            return false
        }
    }

    var isFailed: Bool {
        if case .failed = self { return true }
        return false
    }

    var errorMessage: String? {
        if case let .failed(message) = self {
            return message
        }
        return nil
    }
}
