// Covers the bundled dsh update-channel helpers: channel coercion, preview
// version recognition, and the status/hint sentences the settings page shows.
import assert from "assert";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
  DSH_CHANNEL_PREVIEW,
  DSH_CHANNEL_STABLE,
  DSH_CHANNEL_STORAGE_KEY,
  dshPinText,
  dshPreviewHintText,
  dshUpdateStatusText,
  dshVersionLabel,
  isPreviewChannel,
  isPreviewDshVersion,
  normalizeDshChannel,
  shouldAutoInstallDshUpdate,
  storedDshChannel,
} from "../src/dsh-channel.js";

// A translate stand-in that echoes the key and its placeholders, so the test
// asserts which sentence was chosen without depending on the wording.
const t = (key, vars) =>
  vars ? `${key}(${Object.entries(vars).map(([k, v]) => `${k}=${v}`).join(",")})` : key;

// --- channel normalization -------------------------------------------------
assert.equal(normalizeDshChannel("preview"), DSH_CHANNEL_PREVIEW);
assert.equal(normalizeDshChannel("stable"), DSH_CHANNEL_STABLE);
assert.equal(normalizeDshChannel(undefined), DSH_CHANNEL_STABLE, "missing preference must stay stable");
assert.equal(normalizeDshChannel("nightly"), DSH_CHANNEL_STABLE, "unknown values must stay stable");
assert.equal(isPreviewChannel("preview"), true);
assert.equal(isPreviewChannel(""), false);

// The persisted preference round-trips; a broken store falls back to stable.
const store = new Map();
globalThis.localStorage = {
  getItem: (key) => (store.has(key) ? store.get(key) : null),
};
assert.equal(storedDshChannel(), DSH_CHANNEL_STABLE, "nothing stored means the stable channel");
store.set(DSH_CHANNEL_STORAGE_KEY, "preview");
assert.equal(storedDshChannel(), DSH_CHANNEL_PREVIEW);
store.set(DSH_CHANNEL_STORAGE_KEY, "garbage");
assert.equal(storedDshChannel(), DSH_CHANNEL_STABLE);
delete globalThis.localStorage;
assert.equal(storedDshChannel(), DSH_CHANNEL_STABLE, "a missing localStorage must not throw");
console.log("✓ channel values fall back to stable");

// --- preview version recognition ------------------------------------------
assert.equal(isPreviewDshVersion("0.1.6-alpha.2"), true);
assert.equal(isPreviewDshVersion("0.1.6-beta.1"), true);
assert.equal(isPreviewDshVersion("v0.1.6-Beta.1"), true);
assert.equal(isPreviewDshVersion("0.1.5-rc.2"), false, "release candidates are stable");
assert.equal(isPreviewDshVersion("0.1.5"), false);
assert.equal(isPreviewDshVersion("0.1.6-alphabet.1"), false, "labels must match exactly");
assert.equal(isPreviewDshVersion(undefined), false);
console.log("✓ preview versions are recognized by their prerelease label");

// --- status sentences ------------------------------------------------------
assert.equal(
  dshUpdateStatusText({ managed: false }, t),
  "updates.dsh.notManaged",
  "an unmanaged dsh has its own message",
);
assert.equal(
  dshUpdateStatusText(
    { managed: true, channel: "stable", available: true, latest_version: "0.1.5-rc.3" },
    t,
  ),
  "updates.dsh.newVersion(version=0.1.5-rc.3)",
);
assert.equal(
  dshUpdateStatusText(
    { managed: true, channel: "preview", available: true, preview: true, latest_version: "0.1.6-alpha.2" },
    t,
  ),
  "updates.dsh.newPreview(version=0.1.6-alpha.2)",
);
assert.equal(
  dshUpdateStatusText(
    { managed: true, channel: "stable", available: false, current_version: "0.1.5-rc.2" },
    t,
  ),
  "updates.dsh.upToDate(version=0.1.5-rc.2)",
);
assert.equal(
  dshUpdateStatusText(
    {
      managed: true,
      channel: "preview",
      available: false,
      current_version: "0.1.6-alpha.2",
      current_is_preview: true,
    },
    t,
  ),
  "updates.dsh.previewUpToDate(version=0.1.6-alpha.2)",
);
assert.equal(
  dshUpdateStatusText(
    {
      managed: true,
      channel: "stable",
      available: false,
      current_version: "0.1.6-alpha.2",
      latest_version: "0.1.5-rc.2",
      current_is_preview: true,
    },
    t,
  ),
  "updates.dsh.previewNewerThanStable(current=0.1.6-alpha.2,stable=0.1.5-rc.2)",
);
console.log("✓ status sentences follow channel and preview state");

// --- preview hint ----------------------------------------------------------
const stableCheck = {
  managed: true,
  channel: "stable",
  available: false,
  current_version: "0.1.5-rc.2",
  preview_version: "0.1.6-alpha.2",
};
assert.equal(
  dshPreviewHintText(stableCheck, t),
  "updates.dsh.previewAvailable(version=0.1.6-alpha.2)",
  "a beta must be announced while the stable channel is selected",
);
assert.equal(
  dshPreviewHintText({ ...stableCheck, channel: "preview" }, t),
  null,
  "no hint once the preview channel is selected",
);
assert.equal(
  dshPreviewHintText({ ...stableCheck, preview_version: null }, t),
  null,
  "no hint when npm has no newer preview",
);
assert.equal(
  dshPreviewHintText({ ...stableCheck, current_is_preview: true }, t),
  null,
  "no hint while a preview is already running",
);
console.log("✓ preview hint only appears when a beta is actually installable");

// --- pinned versions (downgrades) ------------------------------------------
// A pin explains the running version and names the newest build on the channel.
assert.equal(
  dshUpdateStatusText(
    {
      managed: true,
      channel: "stable",
      available: true,
      current_version: "0.1.2-rc.1",
      latest_version: "0.1.5-rc.2",
      pinned_version: "0.1.2-rc.1",
    },
    t,
  ),
  "updates.dsh.pinnedWithLatest(version=0.1.2-rc.1,latest=0.1.5-rc.2)",
  "a pinned older build must say so instead of only offering an update",
);
assert.equal(
  dshUpdateStatusText(
    {
      managed: true,
      channel: "preview",
      available: false,
      current_version: "0.1.6-alpha.2",
      latest_version: "0.1.6-alpha.2",
      current_is_preview: true,
      pinned_version: "0.1.6-alpha.2",
    },
    t,
  ),
  "updates.dsh.pinned(version=0.1.6-alpha.2)",
  "pinning the newest build (and the preview branch) picks the short sentence",
);
console.log("✓ pinned versions get their own status sentence");

// An explicit pin must stop the automatic installer from undoing the choice.
assert.equal(shouldAutoInstallDshUpdate({ available: true }), true);
assert.equal(
  shouldAutoInstallDshUpdate({ available: true, pinned_version: "0.1.2-rc.1" }),
  false,
  "automatic installs must not override a pinned version",
);
assert.equal(shouldAutoInstallDshUpdate({ available: false }), false);
assert.equal(shouldAutoInstallDshUpdate(null), false);

// --- version picker labels -------------------------------------------------
const label = (option) => dshVersionLabel(option, t);
assert.equal(
  label({ version: "0.1.5-rc.2", preview: false, installed: true, active: true }),
  "0.1.5-rc.2 · updates.dsh.versionActive",
);
assert.equal(
  label({ version: "0.1.2-rc.1", preview: false, installed: true, active: false }),
  "0.1.2-rc.1 · updates.dsh.versionInstalled",
);
assert.equal(
  label({ version: "0.1.6-alpha.2", preview: true, installed: false, active: false }),
  "0.1.6-alpha.2 · updates.dsh.versionPreview · updates.dsh.versionDownload",
  "an older preview that still has to be downloaded is marked as such",
);
assert.equal(
  label({ version: "0.1.5-rc.1", preview: false, installed: false, active: false }),
  "0.1.5-rc.1 · updates.dsh.versionDownload",
);
console.log("✓ version entries say whether they are installed or need a download");

// --- pin summary -----------------------------------------------------------
assert.equal(
  dshPinText({ active_version: "0.1.2-rc.1", pinned_version: "0.1.2-rc.1" }, t),
  "updates.dsh.pinnedHint(version=0.1.2-rc.1)",
);
assert.equal(
  dshPinText({ active_version: "0.1.5-rc.2", pinned_version: null }, t),
  "updates.dsh.followingHint(version=0.1.5-rc.2)",
);
assert.equal(
  dshPinText(
    { active_version: "0.1.5-rc.2", pinned_version: null, npm_reachable: false },
    t,
  ),
  "updates.dsh.followingHint(version=0.1.5-rc.2) updates.dsh.npmOffline",
  "an offline npm lookup must be visible, not silently shorten the list",
);
assert.equal(dshPinText(null, t), "");

// --- every message key used here exists in both dictionaries ---------------
const i18nSource = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), "../src/i18n.js"),
  "utf8",
);
const keys = new Set();
const samples = [
  { managed: false },
  { managed: true, channel: "stable", available: true, latest_version: "1.0.0" },
  { managed: true, channel: "preview", available: true, preview: true, latest_version: "1.0.0" },
  { managed: true, channel: "stable", available: false, current_version: "1.0.0" },
  { managed: true, channel: "preview", available: false, current_version: "1.0.0-alpha.1", current_is_preview: true },
  {
    managed: true,
    channel: "stable",
    available: false,
    current_version: "1.0.0-alpha.1",
    latest_version: "1.0.0-rc.1",
    current_is_preview: true,
  },
];
// Sentences can contain a version or two message keys, so collect every
// `updates.*` token instead of treating the whole sentence as one key.
const addKeys = (text) => {
  for (const token of String(text).split(/[\s·]+/)) {
    if (token.startsWith("updates.")) keys.add(token);
  }
};
for (const sample of samples) {
  addKeys(dshUpdateStatusText(sample, (key, vars) => (vars ? key : key)));
  addKeys(dshPreviewHintText({ ...sample, preview_version: "1.0.0-beta.1" }, (key) => key) || "");
}
addKeys(dshUpdateStatusText({ ...samples[1], pinned_version: "0.9.0" }, (key) => key));
addKeys(
  dshUpdateStatusText({ ...samples[1], available: false, pinned_version: "1.0.0" }, (key) => key),
);
addKeys(dshPinText({ active_version: "1.0.0", pinned_version: "0.9.0" }, (key) => key));
addKeys(dshPinText({ active_version: "1.0.0", npm_reachable: false }, (key) => key));
for (const option of [
  { version: "1.0.0", preview: true, installed: true, active: true },
  { version: "1.0.0", preview: false, installed: true, active: false },
  { version: "0.9.0", preview: false, installed: false, active: false },
]) {
  addKeys(dshVersionLabel(option, (key) => key));
}
console.log(`✓ all ${keys.size} dsh channel message keys exist in zh and en`);

console.log("dsh-channel: all checks passed");
