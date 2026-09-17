// Verifies the shell keeps link handling in the native layer: links that leave
// the app must go to the system browser instead of being dropped by WebKit
// (target="_blank" with no new-window handler) or replacing the shell page.
import assert from "assert";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const rustSource = readFileSync(join(root, "src-tauri", "src", "lib.rs"), "utf8");
const mainSource = readFileSync(join(root, "src", "main.js"), "utf8");

// Both WebKit entry points have to be wired: navigation (a plain click) and
// new-window requests (window.open and the link context menu).
assert.match(
  rustSource,
  /WebviewWindowBuilder::from_config\(app, config\)\?[\s\S]*?\.on_navigation\(/,
  "the main window must decide navigations natively",
);
assert.match(
  rustSource,
  /\.on_new_window\(move \|url, _features\| \{[\s\S]*?NewWindowResponse::Deny/,
  "new-window requests must be answered instead of silently dropped",
);
assert.match(
  rustSource,
  /fn handle_navigation[\s\S]*?NavigationDisposition::External => \{[\s\S]*?open_link_externally\(app, url\);[\s\S]*?false/,
  "external navigations must open in the default browser and be cancelled",
);

// The loopback Harness port changes on every start, so the rule is per host:
// loopback stays in the app, everything else is a web link.
assert.match(
  rustSource,
  /fn is_loopback_host\(host: &str\) -> bool \{[\s\S]*?eq_ignore_ascii_case\("localhost"\)[\s\S]*?is_loopback\(\)/,
  "loopback hosts (including ::1 and 127.0.0.0/8) must be recognised",
);
assert.match(
  rustSource,
  /"http" \| "https" => match url\.host_str\(\) \{[\s\S]*?NavigationDisposition::External/,
  "non-loopback web links must be classified as external",
);
assert.match(
  rustSource,
  /fn is_openable_web_url\(url: &str\) -> bool \{[\s\S]*?"https:\/\/"[\s\S]*?"http:\/\/"[\s\S]*?"mailto:"/,
  "only http(s) and mailto links may be handed to the operating system",
);

// Failures surface in the shell instead of failing silently.
assert.match(
  mainSource,
  /listen\("link-open-failed",[\s\S]*?toast\.linkFailed/,
  "the shell must tell the user when a link could not be opened",
);

console.log("✓ external links are opened by the system browser");
