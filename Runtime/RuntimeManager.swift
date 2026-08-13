import Foundation
import SwiftUI

@MainActor
final class RuntimeManager: ObservableObject {
    @Published private(set) var dshExecutableURL: URL?
    @Published private(set) var status: String

    init() {
        self.status = "Looking for dsh in PATH"
        refresh()
    }

    func refresh() {
        dshExecutableURL = PathUtils.findExecutable(named: "dsh")
        if let dshExecutableURL {
            status = dshExecutableURL.path
        } else {
            status = "dsh was not found"
        }
    }
}
