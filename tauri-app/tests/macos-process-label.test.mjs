import assert from "assert";
import { readFileSync } from "fs";
import { dirname, join } from "path";
import { fileURLToPath } from "url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const config = JSON.parse(
  readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const rustSource = readFileSync(join(root, "src-tauri", "src", "lib.rs"), "utf8");

const mainWindow = config.app.windows.find((window) => window.label === "main");
assert.equal(mainWindow?.create, false, "the main window must be created through Rust");
assert.match(
  rustSource,
  /processDisplayName/,
  "the macOS WebKit configuration must set a display name for the web-content process",
);
assert.match(
  rustSource,
  /DeepSeek Harness Desk/,
  "the WebKit process display name must match the app name",
);
assert.match(
  rustSource,
  /with_webview_configuration/,
  "the custom WebKit configuration must be applied to the main window",
);

console.log("✓ macOS WebKit process label is configured as DeepSeek Harness Desk");
