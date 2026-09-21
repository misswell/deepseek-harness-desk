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
  /match classify_mux_message\(text\) \{[\s\S]{0,400}?HarnessInbound::Notice\(notice\) =>[\s\S]{0,400}?ledger\.route\(/,
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
  "api-session/error",
  "api-session/removed",
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
  /let mut ledger = AttentionLedger::default\(\);\s*loop \{/,
  "dedup and session state must outlive a single stream connection",
);
// A backend that went away says nothing about the next one.
assert.match(
  rustSource,
  /ledger\.reset\(\);/,
  "session state must be dropped when the Harness stops",
);

// --- one turn gets one banner ----------------------------------------------
// The completion is only reported after it has waited for the interaction or
// failure that may describe the same turn of work.
assert.match(
  rustSource,
  /const COMPLETION_DEBOUNCE: Duration = Duration::from_millis\((\d{2,})\)/,
  "a completion must wait out a debounce window before it fires",
);
assert.match(
  rustSource,
  /struct AttentionLedger \{[\s\S]*?pending_completions: HashMap<String, Instant>,[\s\S]*?attended_sessions: HashSet<String>,/,
  "the ledger must track both waiting completions and turns already covered",
);
// The first idle a page reports is a baseline, not a finished task.
assert.match(
  rustSource,
  /if previous == Some\(true\) && !self\.attended_sessions\.contains\(session_id\) \{/,
  "only a running session that goes idle may complete",
);
assert.match(
  rustSource,
  /fn wait_until_deadline\(deadline: Option<Instant>\)/,
  "the read loop must wait for the socket and the debounce together",
);
assert.match(
  rustSource,
  /_ = wait_until_deadline\(ledger\.deadline\(\)\) => ledger\.due\(Instant::now\(\)\),/,
  "both stream paths must actually collect due completions",
);

// --- every reminder kind has wording --------------------------------------
for (const kind of ["Question", "PlanReview", "Approval"]) {
  assert.match(
    rustSource,
    new RegExp(`NoticeKind::${kind}`),
    `${kind} must be a distinct notice kind`,
  );
}
assert.match(
  rustSource,
  /fn on_task_failed\(/,
  "a failed task must have its own banner path",
);
assert.match(
  rustSource,
  /"计划等待你的确认"/,
  "plan review needs Chinese wording",
);
assert.match(
  rustSource,
  /"任务执行失败"/,
  "a failure needs Chinese wording",
);
assert.ok(
  /fn on_needs_interaction[\s\S]*?\n}\n/.test(rustSource),
  "interaction wording must live in one place",
);
assert.match(
  rustSource,
  /NoticeKind::PlanReview \| NoticeKind::Approval => None,\n\s*\};\n\s*raise_attention\(app, state, title, preview\.unwrap_or\(fallback\)\);/,
  "only a plain question may quote Harness text back",
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

// --- the two new categories are settable and defaulted safely ---------------
assert.match(
  rustSource,
  /fn set_notification_prefs\(\s*state: State<'_, HarnessState>,\s*enabled: bool,\s*task_completed: bool,\s*interaction: bool,\s*error: bool,\s*detail: bool,\s*\)/,
  "the backend must accept every notification category",
);
assert.match(
  rustSource,
  /notify_error: Arc::new\(AtomicBool::new\(true\)\),\s*notify_detail: Arc::new\(AtomicBool::new\(false\)\),/,
  "errors default to on, and quoting Harness text defaults to off",
);
assert.match(
  rustSource,
  /struct NotificationPrefsView \{[^}]*?error: bool,\s*detail: bool,\s*\}/,
  "the prefs view must report the new categories",
);
assert.match(
  mainSource,
  /notifyError: localStorage\.getItem\("notifyError"\) !== "false",/,
  "the shell must keep errors on unless the user turns them off",
);
// A stored "false" must not read as true: the default-off categories have to
// compare against "true", not against "false".
assert.match(
  mainSource,
  /notifyDetail: localStorage\.getItem\("notifyDetail"\) === "true",/,
  "the detail preview must stay off unless the user explicitly turned it on",
);
assert.match(
  mainSource,
  /error: state\.notifyError,\s*detail: state\.notifyDetail,/,
  "both new categories must reach the backend",
);
for (const id of ["notify-error-toggle", "notify-detail-toggle"]) {
  assert.ok(htmlSource.includes(`id="${id}"`), `the settings page needs #${id}`);
  assert.ok(
    mainSource.includes(`#${id}`),
    `the shell must bind #${id}`,
  );
}
// The privacy rule is enforced where the banner is built, not in the UI.
assert.match(
  rustSource,
  /fn notice_detail<'a>\(state: &HarnessState, detail: &'a str\) -> Option<&'a str>/,
  "payload text must pass one gate before it can reach a banner",
);
assert.match(
  rustSource,
  /let body = detail\s*\.and_then\(\|detail\| notice_detail\(state, detail\)\)\s*\.unwrap_or\(fallback\);/,
  "a failure message must fall back to generic wording when details are off",
);
// Both languages need labels for the new rows.
const i18nSource = readFileSync(resolve(root, "src/i18n.js"), "utf8");
for (const key of ["advanced.notifyError", "advanced.notifyErrorDesc", "advanced.notifyDetail", "advanced.notifyDetailDesc"]) {
  assert.equal(
    [...i18nSource.matchAll(new RegExp(`"${key}":`, "g"))].length,
    2,
    `${key} must exist in both languages`,
  );
}

console.log("✓ the notification pipeline is wired end to end");
