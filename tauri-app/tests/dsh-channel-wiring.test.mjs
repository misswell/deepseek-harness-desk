// Guards the frontend <-> backend contract of the bundled dsh update channel
// and version picker: the shell must send a channel with every check, offer a
// rollback and a version switch, and the Rust side must accept/register every
// command. These assertions cross a language boundary that neither unit suite
// can cover on its own.
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
for (const id of [
  "dsh-version-select",
  "apply-dsh-version-button",
  "follow-latest-dsh-button",
  "dsh-version-pin",
]) {
  assert.ok(
    htmlSource.includes(`id="${id}"`),
    `the updates page must expose the dsh version picker (#${id})`,
  );
}
console.log("✓ updates page exposes channel, rollback, hint and version controls");

// --- shell sends the channel and can roll back -----------------------------
assert.match(
  mainSource,
  /call\("check_dsh_update",\s*\{\s*channel:\s*currentDshChannel\(\)\s*\}\)/,
  "every dsh check must carry the selected channel",
);
// The rollback button is a version switch to the newest stable build; the
// backend no longer deletes preview directories.
assert.match(
  mainSource,
  /const version = state\.dshUpdate\?\.stable_version;/,
  "the rollback must target the stable version reported by the backend",
);
assert.ok(
  !mainSource.includes('call("rollback_dsh_preview")'),
  "the removed rollback_dsh_preview command must not be called anymore",
);
assert.match(
  mainSource,
  /call\("set_dsh_version", \{ version \}\)/,
  "the picker must call set_dsh_version",
);
assert.match(
  mainSource,
  /call\("follow_latest_dsh_version"\)/,
  "following the newest build must call follow_latest_dsh_version",
);
assert.match(
  mainSource,
  /call\("list_dsh_versions"\)/,
  "the picker must load its options from list_dsh_versions",
);
assert.match(
  mainSource,
  /shouldAutoInstallDshUpdate\(state\.dshUpdate\)/,
  "automatic installs must be skipped while a version is pinned",
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
console.log("✓ shell sends the channel, persists it and drives the version picker");

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
for (const expected of [
  /async fn list_dsh_versions\(app: AppHandle\)/,
  /async fn set_dsh_version\(/,
  /async fn follow_latest_dsh_version\(/,
]) {
  assert.match(rustSource, expected, `the backend must define ${expected}`);
}
assert.ok(
  !rustSource.includes("async fn rollback_dsh_preview("),
  "rollback_dsh_preview must be gone (the pin replaces it)",
);
// The launcher must resolve the pinned version instead of always taking the
// newest managed directory, otherwise a downgrade could not work.
assert.match(
  rustSource,
  /fn select_active_dsh_version\(/,
  "the launcher must resolve the active version through the pin",
);
assert.match(
  rustSource,
  /fn active_dsh_version_path\(app: Option<&AppHandle>\)/,
  "candidates and the PATH wrapper must use the active version",
);
for (const command of [
  "check_dsh_update",
  "install_dsh_update",
  "list_dsh_versions",
  "set_dsh_version",
  "follow_latest_dsh_version",
]) {
  assert.ok(
    rustSource.includes(`            ${command},\n`),
    `${command} must be registered in the invoke handler`,
  );
}
console.log("✓ backend accepts the channel and registers the version commands");

console.log("dsh-channel wiring: all checks passed");
