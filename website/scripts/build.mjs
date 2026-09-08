import { cp, mkdir, rm, stat } from "node:fs/promises";
import { resolve } from "node:path";

const root = resolve(new URL("..", import.meta.url).pathname);
const output = resolve(root, "dist");

await rm(output, { recursive: true, force: true });
await mkdir(output, { recursive: true });
await cp(resolve(root, "index.html"), resolve(output, "index.html"));
await cp(resolve(root, "styles.css"), resolve(output, "styles.css"));
await cp(resolve(root, "script.js"), resolve(output, "script.js"));
await cp(resolve(root, "robots.txt"), resolve(output, "robots.txt"));
await cp(resolve(root, "sitemap.xml"), resolve(output, "sitemap.xml"));
await cp(resolve(root, "assets"), resolve(output, "assets"), { recursive: true });

const index = await stat(resolve(output, "index.html"));
console.log(`Built DeepSeek Harness Desk website: ${index.size} bytes`);
