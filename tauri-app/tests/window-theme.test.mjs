import assert from "assert";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const html = readFileSync(join(root, "src", "index.html"), "utf8");
const styles = readFileSync(join(root, "src", "styles.css"), "utf8");
const main = readFileSync(join(root, "src", "main.js"), "utf8");

const preflightThemeScript = html.indexOf('localStorage.getItem("windowTheme")');
const stylesheet = html.indexOf('<link rel="stylesheet" href="/styles.css" />');

assert.ok(preflightThemeScript >= 0, "the initial theme must be read from storage");
assert.ok(
  preflightThemeScript < stylesheet,
  "the stored theme must be applied before the stylesheet loads",
);
assert.match(
  styles,
  /:root\[data-theme="dark"\]/,
  "dark theme CSS must support the restored theme attribute",
);
assert.match(
  main,
  /pagehide[\s\S]*rememberColorScheme|rememberColorScheme[\s\S]*pagehide/,
  "the current theme must be remembered before a WebView is destroyed",
);
assert.match(
  main,
  /window-hidden[\s\S]*rememberColorScheme|rememberColorScheme[\s\S]*window-hidden/,
  "window hiding must persist the current theme",
);

console.log("✓ dark theme is restored before a reopened WebView paints");
