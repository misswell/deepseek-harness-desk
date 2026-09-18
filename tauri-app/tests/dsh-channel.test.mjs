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
  dshPreviewHintText,
  dshUpdateStatusText,
  isPreviewChannel,
  isPreviewDshVersion,
  normalizeDshChannel,
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
for (const sample of samples) {
  keys.add(dshUpdateStatusText(sample, (key) => key));
  const hint = dshPreviewHintText({ ...sample, preview_version: "1.0.0-beta.1" }, (key) => key);
  if (hint) keys.add(hint);
}
for (const key of keys) {
  const zhCount = i18nSource.split(`"${key}":`).length - 1;
  assert.equal(zhCount, 2, `${key} must exist in both zh and en dictionaries (found ${zhCount})`);
}
console.log(`✓ all ${keys.size} dsh channel message keys exist in zh and en`);

console.log("dsh-channel: all checks passed");
