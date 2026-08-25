import assert from "assert";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const html = readFileSync(join(root, "src", "index.html"), "utf8");

const frameContainer = html.match(/<div class="([^"]+)" id="frame-container">/);
const startupView = html.match(/<section class="([^"]+)" id="startup-view"/);

assert.ok(frameContainer, "the Harness frame container must exist");
assert.ok(startupView, "the startup view must exist");
assert.doesNotMatch(
  frameContainer[1],
  /\bhidden\b/,
  "the initial shell must show the webpage loading state",
);
assert.match(
  startupView[1],
  /\bhidden\b/,
  "the initial shell must keep the runtime startup card hidden",
);

console.log("✓ reopening starts with webpage loading instead of the runtime startup card");
