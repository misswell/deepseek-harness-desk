// Guards the frontend <-> backend contract of the notification pipeline. The
// watcher must (1) subscribe to the dsh 0.1.2+ forwarded-event stream instead
// of waiting on a silent mux, (2) keep reading legacy streams, (3) post
// through a notification transport that modern macOS still delivers, and
// (4) expose the platform permission to the settings page.
import assert from "assert";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const mainSource = readFileSync(resolve(root, "src/main.js"), "utf8");
const htmlSource = readFileSync(resolve(root, "src/index.html"), "utf8");
const rustSource = readFileSync(resolve(root, "src-tauri/src/lib.rs"), "utf8");
const cargoSource = readFileSync(resolve(root, "src-tauri/Cargo.toml"), "utf8");

// --- the watcher subscribes before it waits --------------------------------
assert.match(
  rustSource,
  /const EVENTS_STREAM_ID: &str = "desk-events"/,
  "the forwarded-event stream needs a stable stream id",
);
assert.match(
  rustSource,
  /"endpoint": "\$events"/,
  "the mux open frame must target the gateway's forwarded-event endpoint",
);
assert.match(
  rustSource,
  /fn events_stream_open_frame\(\) -> String/,
  "the open frame must be built in one place",
);
assert.match(
  rustSource,
  /mux\.send\(tokio_tungstenite::tungstenite::Message::text\(/,
  "the watcher must actually send the open frame on connect",
);
// The watcher must interpret each frame it receives, not just keep the socket
// open: mux frames go through the classifier and then the semantic layer.
assert.match(
  rustSource,
  /match classify_mux_message\(text\) \{[\s\S]{0,400}?HarnessInbound::Notice\(notice\) =>[\s\S]{0,400}?apply_harness_notice\(/,
  "mux frames must be classified and routed through the semantic layer",
);

// --- the wire vocabulary covers both dsh generations ------------------------
for (const name of [
  "user-questions/request",
  "question/requested",
  "approval/request",
  "approval/requested",
  "api-session/status",
  "host/session-status",
]) {
  assert.ok(
    rustSource.includes(`"${name}"`),
    `the adapter must understand the ${name} event`,
  );
}

// --- semantic layer keeps policy independent of the wire --------------------
assert.match(
  rustSource,
  /enum HarnessNotice \{/,
  "wire formats must normalize into semantic notices",
);
assert.match(
  rustSource,
  /enum HarnessInbound \{/,
  "raw messages must classify without side effects",
);
// Dedup and running state must survive reconnects: both live outside the
// reconnect loop now.
assert.match(
  rustSource,
  /let mut seen_interactions = VecDeque::new\(\);\s*let mut running_sessions = HashMap::new\(\);\s*loop \{/,
  "dedup and session state must outlive a single stream connection",
);

// --- macOS delivery goes through UNUserNotificationCenter -------------------
assert.equal(
  /app\.notification\(\)\.builder\(\)/.test(rustSource.match(/fn raise_attention[\s\S]*?\n}/)?.[0] ?? ""),
  false,
  "raise_attention must not post directly through the plugin",
);
assert.match(
  rustSource,
  /fn post_system_notification\(\s*app: &AppHandle,\s*state: &HarnessState,\s*title: &str,\s*body: &str,?\s*\)/,
  "one place must own the delivery fallback chain",
);
assert.match(
  rustSource,
  /UNUserNotificationCenter::currentNotificationCenter/,
  "macOS delivery must use the supported notification center",
);
assert.match(
  rustSource,
  /requestAuthorizationWithOptions_completionHandler/,
  "macOS permission must be requested before the first banner",
);
assert.match(
  cargoSource,
  /objc2-user-notifications/,
  "the macOS notification framework must be a dependency",
);
// The old API is gone on modern systems; nothing may still depend on it for
// the actual delivery path.
assert.equal(
  /mac_notification_sys/.test(rustSource),
  false,
  "the removed NSUserNotification backend must not be used directly",
);

// --- permission is surfaced to the settings page ----------------------------
assert.match(
  rustSource,
  /fn notification_permission\(\)/,
  "the shell must be able to read the platform permission",
);
assert.match(
  rustSource,
  /fn open_notification_settings\(\) -> Result<\(\), String>/,
  "the shell must be able to open the system notification pane",
);
assert.match(
  rustSource,
  /notification_permission,\s*send_test_notification,\s*open_notification_settings,/,
  "all three notification commands must be registered",
);
assert.match(
  mainSource,
  /import \{[\s\S]*?normalizeNotificationPermission,[\s\S]*?\} from "\.\/notification-permission\.js"/,
  "the shell must normalize the permission payload",
);
assert.match(
  mainSource,
  /await call\("notification_permission"\)/,
  "the shell must ask the backend for the permission",
);
assert.match(
  mainSource,
  /await call\("open_notification_settings"\)/,
  "the settings button must open the system pane",
);
assert.match(
  mainSource,
  /void refreshNotificationPermission\(\)/,
  "opening the settings must refresh the permission",
);
// A prompt the user has not answered inside the timeout is not a denial, and a
// denial lifted in System Settings is not one either: only a real grant may be
// held for the rest of the run, or every later banner goes quietly missing.
assert.equal(
  /AUTHORIZED\.get_or_init/.test(rustSource),
  false,
  "an unanswered authorization request must not be cached as a denial",
);
assert.match(
  rustSource,
  /let \(determined, granted\) = authorization_status\(\);/,
  "the permission must be re-read live until it is granted",
);
assert.match(
  rustSource,
  /macos_notification::ask_for_authorization_again\(\)/,
  "the test button must be able to bring the prompt back",
);
assert.match(
  htmlSource,
  /id="notify-permission-hint"/,
  "the settings page must have a place for the permission hint",
);
assert.match(
  htmlSource,
  /id="notify-permission-open"/,
  "the hint must offer a way into System Settings",
);
assert.match(
  rustSource,
  /fn send_test_notification\(app: AppHandle, state: State<'_, HarnessState>\)/,
  "the shell must be able to post a test banner",
);
assert.match(
  mainSource,
  /await call\("send_test_notification"\)/,
  "the settings button must trigger the test banner",
);
assert.match(
  htmlSource,
  /id="notify-test-send"/,
  "the settings page must expose the test button",
);

console.log("✓ the notification pipeline is wired end to end");
