import Darwin
import Foundation

enum PortScanner {
    static func firstAvailable(
        startingAt start: UInt16 = 3080,
        endingAt end: UInt16 = 3099
    ) -> UInt16? {
        guard start <= end else { return nil }
        for port in start...end where isAvailable(port: port) {
            return port
        }
        return nil
    }

    static func isAvailable(port: UInt16) -> Bool {
        let socketDescriptor = socket(AF_INET, SOCK_STREAM, 0)
        guard socketDescriptor >= 0 else { return false }
        defer { close(socketDescriptor) }

        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian
        address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))

        let result = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { reboundPointer in
                bind(
                    socketDescriptor,
                    reboundPointer,
                    socklen_t(MemoryLayout<sockaddr_in>.size)
                )
            }
        }
        return result == 0
    }
}
