import assert from "assert";
import { notificationPrefsPayload } from "../src/notification-prefs.js";

const payload = notificationPrefsPayload({
  enabled: true,
  taskCompleted: false,
  interaction: true,
});

assert.deepEqual(payload, {
  enabled: true,
  taskCompleted: false,
  interaction: true,
});
assert.equal("task_completed" in payload, false);

console.log("✓ notification prefs use Tauri's camelCase taskCompleted key");
