import assert from "assert";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const html = readFileSync(join(root, "src", "index.html"), "utf8");
const styles = readFileSync(join(root, "src", "styles.css"), "utf8");
const librs = readFileSync(join(root, "src-tauri", "src", "lib.rs"), "utf8");

// The Desk startup screen is the prelude to the Harness web client's own boot
// page (dsh-client-web AppRoot: wordmark / spinner / hint on a flat surface).
// Both screens show in sequence, so they must share the same flat layout and
// palette instead of the old carded, blue-accented startup card.

for (const id of ["startup-view", "error-view"]) {
  const section = html.match(
    new RegExp(`<section class="[^"]*" id="${id}"[\\s\\S]*?</section>`),
  );
  assert.ok(section, `${id} must exist`);
  assert.match(
    section[0],
    /class="boot-wordmark"[^>]*>HARNESS</,
    `${id} must carry the Harness wordmark like the embedded boot page`,
  );
  assert.doesNotMatch(
    section[0],
    /brand-mark|error-mark/,
    `${id} must not bring back the colored app-icon tile`,
  );
}

// Flat stack: no card chrome (border / shadow / surface box) on the boot screen.
assert.doesNotMatch(html, /startup-card|error-card/, "the startup card markup must stay removed");
assert.doesNotMatch(styles, /\.startup-card|\.error-card/, "the startup card styles must stay removed");
assert.match(
  styles,
  /\.startup-stack \{[^}]*flex-direction:\s*column/,
  "the boot stack must be a centered flex column",
);

// Spinner replicates the boot page's ring: border ring + conic-gradient arc
// masked to a 2px track, instead of the old blue accent ring.
const spinner = styles.match(/\.spinner \{([^}]*)\}/);
assert.ok(spinner, "the spinner must have a style rule");
assert.match(spinner[1], /width:\s*20px/, "the spinner must keep the boot page's 20px ring");
assert.match(spinner[1], /border:\s*2px solid var\(--boot-border\)/, "the ring must use the boot border color");
const arc = styles.match(/\.spinner::after \{([^}]*)\}/);
assert.ok(arc, "the spinner arc must be styled");
assert.match(
  arc[1],
  /conic-gradient\(var\(--boot-brand\) var\(--boot-arc, 72deg\), transparent 0\)/,
  "the arc must be the boot brand color like the embedded spinner",
);
assert.match(
  arc[1],
  /-webkit-mask:\s*radial-gradient\(farthest-side, transparent calc\(100% - 2px\), #000 0\)/,
  "the arc must be masked to a thin ring",
);

// Palette must match body[data-ds-dark-theme] / light boot variables in the
// Harness web client, and the startup view must not fall back to app accents.
assert.match(
  styles,
  /\.startup-view,\n\.error-view \{[\s\S]*?--boot-bg:\s*#ffffff;[\s\S]*?--boot-label-primary:\s*#0f1115;[\s\S]*?--boot-label-tertiary:\s*#81858c;/,
  "the light boot palette must match the embedded boot page",
);
assert.match(
  styles,
  /:root\[data-theme="dark"\] \.startup-view,\n:root\[data-theme="dark"\] \.error-view \{[\s\S]*?--boot-bg:\s*#151517;[\s\S]*?--boot-label-primary:\s*#f9fafb;[\s\S]*?--boot-label-secondary:\s*#cfd3d6;[\s\S]*?--boot-label-tertiary:\s*#adb2b8;/,
  "the dark boot palette must match the embedded boot page",
);
const startupButtons = styles.match(
  /\.startup-view \.primary-button,\n\.error-view \.primary-button \{([^}]*)\}/,
);
assert.ok(startupButtons, "the boot screens must restyle the primary button");
assert.doesNotMatch(
  startupButtons[1],
  /--accent/,
  "the boot screens' buttons must stay monochrome instead of the app accent",
);

// The native pre-first-paint surface must use the same colors, or the window
// flashes a mismatched shade before the prelude screen paints.
const bgFn = librs.match(/fn window_background_color\(theme: Theme\) -> Color \{([\s\S]*?)\n\}/);
assert.ok(bgFn, "the native window background function must exist");
assert.match(bgFn[1], /Color\(21, 21, 23, 255\)/, "dark native background must be #151517");
assert.match(bgFn[1], /Color\(255, 255, 255, 255\)/, "light native background must be #ffffff");

console.log("✓ startup screen mirrors the Harness boot page's flat style and palette");
