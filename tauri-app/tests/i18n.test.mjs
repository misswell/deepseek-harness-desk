// Verifies i18n consistency: every key referenced from main.js and index.html
// exists in both the zh and en dictionaries, and both dictionaries match.
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const i18nSource = readFileSync(resolve(root, "src/i18n.js"), "utf8");
const mainSource = readFileSync(resolve(root, "src/main.js"), "utf8");
const htmlSource = readFileSync(resolve(root, "src/index.html"), "utf8");

// Extract the zh/en message objects without executing DOM-dependent code.
function extractMessages() {
  const zh = {};
  const en = {};
  const re = /const messages = \{([\s\S]*?)\n\};/;
  const match = i18nSource.match(re);
  if (!match) throw new Error("cannot find messages object");
  let current = null;
  for (const line of match[1].split("\n")) {
    const entry = line.match(/^\s*"([^"]+)":\s*"((?:[^"\\]|\\.)*)",?\s*$/);
    if (entry) {
      (current === "en" ? en : zh)[entry[1]] = entry[2];
      continue;
    }
    if (/^\s*zh: \{/.test(line)) current = "zh";
    else if (/^\s*en: \{/.test(line)) current = "en";
  }
  return { zh, en };
}

const { zh, en } = extractMessages();

const usedInJs = new Set();
for (const m of mainSource.matchAll(/\bt\(\s*"([^"]+)"/g)) usedInJs.add(m[1]);

const usedInHtml = new Set();
for (const attr of ["data-i18n", "data-i18n-title", "data-i18n-aria-label"]) {
  for (const m of htmlSource.matchAll(new RegExp(`${attr}="([^"]+)"`, "g"))) {
    usedInHtml.add(m[1]);
  }
}

const missing = [];
for (const key of [...usedInJs, ...usedInHtml]) {
  if (!(key in zh)) missing.push(`zh: ${key}`);
  if (!(key in en)) missing.push(`en: ${key}`);
}
const unusedInZh = Object.keys(zh).filter((k) => !(k in en));
const unusedInEn = Object.keys(en).filter((k) => !(k in zh));

const problems = [];
if (missing.length) problems.push(`missing keys:\n  ${missing.join("\n  ")}`);
if (unusedInZh.length) problems.push(`only in zh (not in en): ${unusedInZh.join(", ")}`);
if (unusedInEn.length) problems.push(`only in en (not in zh): ${unusedInEn.join(", ")}`);

console.log(`keys: zh=${Object.keys(zh).length} en=${Object.keys(en).length}`);
console.log(`used: js=${usedInJs.size} html=${usedInHtml.size}`);
console.log(problems.length ? `PROBLEMS:\n${problems.join("\n")}` : "ALL CONSISTENT ✓");
process.exit(problems.length ? 1 : 0);
