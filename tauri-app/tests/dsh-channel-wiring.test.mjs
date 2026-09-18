// Guards the frontend <-> backend contract of the bundled dsh update channel:
// the shell must send a channel with every check, offer a rollback, and the
// Rust side must accept/register both commands. These assertions cross a
// language boundary that neither unit suite can cover on its own.
import assert from "assert";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const mainSource = readFileSync(resolve(root, "src/main.js"), "utf8");
const htmlSource = readFileSync(resolve(root, "src/index.html"), "utf8");
const rustSource = readFileSync(resolve(root, "src-tauri/src/lib.rs"), "utf8");

// --- settings UI -----------------------------------------------------------
const select = htmlSource.match(/<select id="dsh-update-channel">([\s\S]*?)<\/select>/);
assert.ok(select, "the updates page must expose a bundled dsh channel selector");
assert.match(select[1], /value="stable"/, "the stable channel must be selectable");
assert.match(select[1], /value="preview"/, "the preview channel must be selectable");
assert.ok(
  htmlSource.includes('id="rollback-dsh-preview-button"'),
  "the updates page must offer a rollback back to the stable channel",
);
assert.ok(
  htmlSource.includes('id="dsh-preview-hint"'),
  "the updates page must have a slot for the preview-available hint",
);
console.log("✓ updates page exposes channel, rollback and hint controls");

// --- shell sends the channel and can roll back -----------------------------
assert.match(
  mainSource,
  /call\("check_dsh_update",\s*\{\s*channel:\s*currentDshChannel\(\)\s*\}\)/,
  "every dsh check must carry the selected channel",
);
assert.match(
  mainSource,
  /call\("rollback_dsh_preview"\)/,
  "the rollback button must call rollback_dsh_preview",
);
assert.match(
  mainSource,
  /localStorage\.setItem\(DSH_CHANNEL_STORAGE_KEY, currentDshChannel\(\)\)/,
  "the selected channel must be persisted",
);
assert.match(
  mainSource,
  /elements\.dshUpdateChannel\.value = storedDshChannel\(\)/,
  "the stored channel must be restored into the selector",
);
console.log("✓ shell sends the channel and persists the preference");

// --- backend accepts the channel and registers both commands ---------------
assert.match(
  rustSource,
  /async fn check_dsh_update\(\s*app: AppHandle,\s*channel: Option<String>,?\s*\)/,
  "check_dsh_update must accept the channel argument",
);
assert.match(
  rustSource,
  /fn from_request\(value: Option<&str>\) -> Self/,
  "the channel must be parsed from the request",
);
assert.match(
  rustSource,
  /DshUpdateChannel::from_request\(channel\.as_deref\(\)\)/,
  "check_dsh_update must parse the requested channel",
);
assert.match(
  rustSource,
  /async fn rollback_dsh_preview\(/,
  "rollback_dsh_preview must exist",
);
for (const command of ["check_dsh_update", "install_dsh_update", "rollback_dsh_preview"]) {
  assert.ok(
    rustSource.includes(`            ${command},\n`),
    `${command} must be registered in the invoke handler`,
  );
}
console.log("✓ backend accepts the channel and registers the rollback command");

console.log("dsh-channel wiring: all checks passed");
