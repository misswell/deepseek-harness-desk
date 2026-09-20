// Compare the two macOS notification transports the shell can use.
//
// Background: tauri-plugin-notification rides mac-notification-sys, which
// posts through NSUserNotificationCenter — the API Apple deprecated and later
// removed. This probe posts the same banner through both transports so the
// shell's fallback order can be checked against the real system instead of
// against documentation.
//
// Build & run:
//   scripts/make_notification_probe.sh
import AppKit
import UserNotifications

/// LaunchServices starts the bundle without inheriting our stdout, so the
/// probe's findings go to a file the caller can read back.
let logURL = URL(fileURLWithPath: NSTemporaryDirectory())
    .appendingPathComponent("dsh-notification-probe.log")
try? FileManager.default.removeItem(at: logURL)
FileManager.default.createFile(atPath: logURL.path, contents: nil)
let logHandle = try! FileHandle(forWritingTo: logURL)
func report(_ line: String) {
    print(line)
    logHandle.write(Data((line + "\n").utf8))
}

let legacyTitle = "Legacy path (NSUserNotification)"
let modernTitle = "Modern path (UNUserNotificationCenter)"
let body = "If you can read this title, that transport still works."

// --- transport 1: NSUserNotificationCenter (what the plugin uses) ----------
// The class may be gone entirely; reflect instead of referencing it directly
// so this file still compiles against current SDKs.
if let legacyClass = NSClassFromString("NSUserNotificationCenter") as? NSObject.Type,
   let legacyNotificationClass = NSClassFromString("NSUserNotification") as? NSObject.Type {
    let center = legacyClass
        .perform(NSSelectorFromString("defaultUserNotificationCenter"))
        .map { $0.takeUnretainedValue() as? NSObject } ?? nil
    let notification = legacyNotificationClass.init()
    notification.setValue(legacyTitle, forKey: "title")
    notification.setValue(body, forKey: "informativeText")
    _ = center?.perform(NSSelectorFromString("deliverNotification:"), with: notification)
    report("legacy: posted via NSUserNotificationCenter")
} else {
    report("legacy: NSUserNotificationCenter is gone from this system")
}

// --- transport 2: UNUserNotificationCenter (what the shell now uses) -------
let center = UNUserNotificationCenter.current()
center.requestAuthorization(options: [.alert, .badge, .sound]) { granted, error in
    report("modern: authorization granted=\(granted) error=\(String(describing: error))")
    guard granted else {
        report("modern: not authorized, stopping")
        return
    }
    let content = UNMutableNotificationContent()
    content.title = modernTitle
    content.body = body
    content.sound = .default
    let request = UNNotificationRequest(
        identifier: "probe.notification.\(UUID().uuidString)",
        content: content,
        trigger: nil
    )
    center.add(request) { error in
        report("modern: add finished error=\(String(describing: error))")
        // Listing what the center holds proves the request was accepted even
        // when the banner itself cannot be seen from a script.
        center.getDeliveredNotifications { delivered in
            let titles = delivered.map { $0.request.content.title }
            report("modern: delivered notifications on record = \(titles)")
        }
    }
}

RunLoop.main.run(until: Date().addingTimeInterval(8))
report("probe finished; check Notification Center for which titles appeared")

