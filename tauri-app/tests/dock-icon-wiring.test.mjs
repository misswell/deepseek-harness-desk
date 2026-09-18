// Guards the frontend <-> backend contract of the selectable Dock icon: the
// shell must offer every style the backend knows, send the selected one, and
// the Rust side must keep the icon effective after the app quits (macOS reads
// the bundle icon, not the running Dock tile, once an app is closed).
import assert from "assert";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const mainSource = readFileSync(resolve(root, "src/main.js"), "utf8");
const helperSource = readFileSync(resolve(root, "src/dock-icon.js"), "utf8");
const htmlSource = readFileSync(resolve(root, "src/index.html"), "utf8");
const rustSource = readFileSync(resolve(root, "src-tauri/src/lib.rs"), "utf8");
const cargoSource = readFileSync(resolve(root, "src-tauri/Cargo.toml"), "utf8");

// --- settings UI -----------------------------------------------------------
const select = htmlSource.match(/<select id="dock-icon-variant-select">([\s\S]*?)<\/select>/);
assert.ok(select, "the general page must expose a Dock icon selector");
for (const variant of ["blue", "black", "avatar"]) {
  assert.match(
    select[1],
    new RegExp(`value="${variant}"`),
    `the ${variant} Dock icon must be selectable`,
  );
}
console.log("✓ the general page offers blue, black and avatar Dock icons");

// --- shell sends the style and reacts to a temporary switch ----------------
assert.match(
  mainSource,
  /import \{[\s\S]*?normalizeDockIconVariant,[\s\S]*?\} from "\.\/dock-icon\.js"/,
  "the shell must use the shared Dock icon helpers",
);
assert.match(
  mainSource,
  /const variant = normalizeDockIconVariant\(elements\.dockIconVariantSelect\.value\)/,
  "the selected style must be normalized before it is sent",
);
assert.match(
  mainSource,
  /call\("set_dock_icon_variant", \{ variant \}\)/,
  "switching the style must call set_dock_icon_variant",
);
assert.match(
  mainSource,
  /const outcome = await call\("set_dock_icon_variant", \{ variant \}\)/,
  "the shell must inspect the outcome of a switch",
);
assert.match(
  mainSource,
  /dockIconPersistWarning\(outcome\)/,
  "a failed bundle write must be surfaced to the user",
);
assert.match(
  mainSource,
  /const dockIconVariant = storedDockIconVariant\(\)/,
  "the stored style must be restored on startup",
);
assert.ok(
  !/set_dock_icon_variant[\s\S]{0,80}=== "black" \? "black" : "blue"/.test(mainSource),
  "the old two-value coercion must be gone",
);
assert.ok(
  helperSource.includes("dockIconPersistWarning"),
  "the persistence warning must live in the shared helper",
);
console.log("✓ the shell sends the style, restores it and reports temporary switches");

// --- backend keeps the icon effective after quitting ----------------------
assert.match(
  rustSource,
  /enum DockIconVariant \{[\s\S]*?Blue,[\s\S]*?Black,[\s\S]*?Avatar,[\s\S]*?\}/,
  "the backend must know all three styles",
);
assert.match(
  rustSource,
  /"avatar" => Some\(Self::Avatar\)/,
  "the backend must accept the avatar style",
);
for (const asset of [
  "DeepSeekHarnessIcon-Prepared-1024.png",
  "DeepSeekHarnessIcon-Black-Prepared-1024.png",
  "DeepSeekHarnessIcon-Avatar-Prepared-1024.png",
]) {
  assert.ok(
    rustSource.includes(`include_bytes!("../../../Assets/${asset}")`),
    `${asset} must be bundled into the app`,
  );
}
// setApplicationIconImage only repaints the running Dock tile; macOS falls back
// to the bundle icon as soon as the app quits, so the chosen style has to be
// written into the bundle as well.
assert.match(
  rustSource,
  /fn persist_macos_dock_icon\(/,
  "the chosen style must also be written into the app bundle",
);
assert.match(
  rustSource,
  /setIcon_forFile_options\(/,
  "the bundle icon must be set through NSWorkspace",
);
assert.match(
  rustSource,
  /bundle\.join\("Icon\\r"\)\.exists\(\)/,
  "clearing the bundled icon must be skippable when nothing is set",
);
assert.match(
  rustSource,
  /fn set_dock_icon_variant\(app: AppHandle, variant: String\) -> Result<DockIconOutcome, String>/,
  "set_dock_icon_variant must report what it changed",
);
assert.match(
  rustSource,
  /struct DockIconOutcome \{[\s\S]*?applied: bool,[\s\S]*?persisted: bool,[\s\S]*?\}/,
  "the outcome must separate the running tile from the persisted icon",
);
assert.ok(
  rustSource.includes("            set_dock_icon_variant,\n"),
  "set_dock_icon_variant must stay registered in the invoke handler",
);
assert.ok(
  cargoSource.includes('"NSWorkspace"'),
  "the macOS dependency list must enable NSWorkspace",
);
console.log("✓ the backend persists the style so a closed app keeps showing it");

console.log("dock-icon wiring: all checks passed");
