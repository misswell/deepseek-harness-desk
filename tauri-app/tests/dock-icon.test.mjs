// Covers the Dock icon helpers: style coercion, the persisted preference, the
// confirmation/warning wording keys, and when a switch is only temporary.
import assert from "assert";
import {
  DEFAULT_DOCK_ICON_VARIANT,
  DOCK_ICON_VARIANT_STORAGE_KEY,
  DOCK_ICON_VARIANTS,
  dockIconPersistWarning,
  dockIconToastKey,
  normalizeDockIconVariant,
  storedDockIconVariant,
} from "../src/dock-icon.js";

// --- style coercion --------------------------------------------------------
assert.deepEqual(DOCK_ICON_VARIANTS, ["blue", "black", "avatar"]);
assert.equal(DEFAULT_DOCK_ICON_VARIANT, "blue");
for (const variant of DOCK_ICON_VARIANTS) {
  assert.equal(normalizeDockIconVariant(variant), variant, `${variant} must round-trip`);
}
assert.equal(normalizeDockIconVariant(undefined), "blue", "missing values mean the bundled icon");
assert.equal(normalizeDockIconVariant("BLACK"), "blue", "values must match exactly");
assert.equal(normalizeDockIconVariant("miku"), "blue", "unknown styles must fall back");
assert.equal(normalizeDockIconVariant(null), "blue");
console.log("✓ dock icon styles normalize to known values");

// --- persisted preference --------------------------------------------------
const store = new Map();
globalThis.localStorage = {
  getItem: (key) => (store.has(key) ? store.get(key) : null),
};
assert.equal(storedDockIconVariant(), "blue", "nothing stored means the bundled icon");
store.set(DOCK_ICON_VARIANT_STORAGE_KEY, "avatar");
assert.equal(storedDockIconVariant(), "avatar");
store.set(DOCK_ICON_VARIANT_STORAGE_KEY, "garbage");
assert.equal(storedDockIconVariant(), "blue");
delete globalThis.localStorage;
assert.equal(storedDockIconVariant(), "blue", "a missing localStorage must not throw");
console.log("✓ the stored style survives a broken store");

// --- wording keys ----------------------------------------------------------
assert.equal(dockIconToastKey("blue"), "toast.dockIconBlue");
assert.equal(dockIconToastKey("black"), "toast.dockIconBlack");
assert.equal(dockIconToastKey("avatar"), "toast.dockIconAvatar");
assert.equal(
  dockIconToastKey("unknown"),
  "toast.dockIconBlue",
  "an unknown style must still produce a usable key",
);
console.log("✓ every style maps to its confirmation wording");

// --- temporary switches ----------------------------------------------------
// The backend reports `applied` for the running Dock tile and `persisted` for
// the bundle icon macOS falls back to once the app quits.
assert.equal(dockIconPersistWarning({ applied: true, persisted: false }), true);
assert.equal(dockIconPersistWarning({ applied: true, persisted: true }), false);
assert.equal(
  dockIconPersistWarning({ applied: false, persisted: false }),
  false,
  "platforms without a Dock tile must not warn",
);
assert.equal(dockIconPersistWarning(undefined), false, "older backends must not warn");
assert.equal(dockIconPersistWarning({}), false);
console.log("✓ only a failed bundle write is reported as a temporary switch");

console.log("dock-icon: all checks passed");
