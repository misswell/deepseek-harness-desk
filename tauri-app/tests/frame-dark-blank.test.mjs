import assert from "assert";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const styles = readFileSync(join(root, "src", "styles.css"), "utf8");
const main = readFileSync(join(root, "src", "main.js"), "utf8");

// A blank or mid-navigation cross-origin iframe paints the embedded document's
// default surface, which is white. The shell must never let that surface show
// through in dark mode: the frame stays invisible until a real Harness document
// has loaded, and the themed container background shows instead.
const frameRule = styles.match(/\.frame-container iframe \{([^}]*)\}/);
assert.ok(frameRule, "the Harness iframe must have a style rule");
assert.match(
  frameRule[1],
  /opacity:\s*0/,
  "the iframe must start hidden so a blank frame cannot paint its white base surface",
);
assert.match(
  frameRule[1],
  /background:\s*transparent/,
  "the iframe surface must be transparent so the themed container shows through",
);
assert.match(
  styles,
  /\.frame-container\.frame-ready iframe \{[^}]*opacity:\s*1/,
  "the iframe may only be revealed once its document is ready",
);
assert.match(
  styles,
  /:root\[data-theme="dark"\] \.frame-container iframe \{[^}]*color-scheme:\s*dark/,
  "the dark theme must give the iframe surface a dark base color",
);
assert.match(
  styles,
  /:root\[data-theme="dark"\] \.frame-container \{[^}]*background:\s*#1c1c1e/,
  "the dark theme must keep the frame container dark",
);

// Blanking the frame (unload / recycle / restart / stop) must drop the reveal
// state, otherwise the about:blank navigation would be shown as white.
const blankHelper = main.match(/function blankHarnessFrame\(\) \{([^}]*)\}/);
assert.ok(blankHelper, "the shell must centralize frame blanking");
assert.match(
  blankHelper[1],
  /setHarnessFrameReady\(false\)/,
  "blanking the frame must clear the reveal state",
);
assert.match(
  blankHelper[1],
  /removeAttribute\("src"\)/,
  "blanking the frame must drop its src",
);

// Only a real Harness document may be revealed; about:blank must be ignored.
assert.match(
  main,
  /elements\.frame\.addEventListener\("load", \(\) => \{[\s\S]*?markHarnessFrameReady\(\)/,
  "the frame load handler must route through the reveal gate",
);
const reveal = main.match(/function markHarnessFrameReady\(\) \{([\s\S]*?)\n\}/);
assert.ok(reveal, "the shell must define the reveal gate");
assert.match(
  reveal[1],
  /if \(!frameDocumentIsCurrent\(\)\) return/,
  "the reveal gate must ignore about:blank and unloaded frames",
);
assert.match(
  main,
  /FRAME_REVEAL_FALLBACK_MS/,
  "a fallback must reveal the frame even if WebKit cancels the frame load",
);

console.log("✓ dark mode shows the themed background instead of a white frame flash");
