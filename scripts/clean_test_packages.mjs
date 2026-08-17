import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const outputDirectories = [
  path.join(repositoryRoot, "dist"),
  path.join(repositoryRoot, "tauri-app", "src-tauri", "target", "debug", "bundle"),
  path.join(repositoryRoot, "tauri-app", "src-tauri", "target", "release", "bundle"),
];

for (const directory of outputDirectories) {
  if (!fs.existsSync(directory)) continue;

  for (const entry of fs.readdirSync(directory)) {
    fs.rmSync(path.join(directory, entry), { force: true, recursive: true });
  }
  console.log(`已清理测试包目录：${path.relative(repositoryRoot, directory) || "."}`);
}
