import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "..");
const outputDirectories = [
  path.join(repositoryRoot, "dist"),
  path.join(repositoryRoot, "tauri-app", "src-tauri", "target", "debug", "bundle"),
  path.join(repositoryRoot, "tauri-app", "src-tauri", "target", "release", "bundle"),
];

function uniquePath(directory, name) {
  const extension = path.extname(name);
  const stem = extension ? name.slice(0, -extension.length) : name;
  let candidate = path.join(directory, name);
  let suffix = 1;
  while (fs.existsSync(candidate)) {
    candidate = path.join(directory, `${stem} (${suffix})${extension}`);
    suffix += 1;
  }
  return candidate;
}

function moveToMacTrash(source) {
  const trashDirectory = path.join(os.homedir(), ".Trash");
  fs.mkdirSync(trashDirectory, { recursive: true });
  const target = uniquePath(trashDirectory, path.basename(source));
  try {
    fs.renameSync(source, target);
  } catch (error) {
    if (error?.code !== "EXDEV") throw error;
    fs.cpSync(source, target, { recursive: true });
    fs.rmSync(source, { force: true, recursive: true });
  }
  return "系统废纸篓";
}

function moveToWindowsRecycleBin(source) {
  const escapedSource = source.replaceAll("'", "''");
  const command = [
    "Add-Type -AssemblyName Microsoft.VisualBasic;",
    `$source = '${escapedSource}';`,
    "if ([IO.Directory]::Exists($source)) {",
    "[Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($source,",
    "[Microsoft.VisualBasic.FileIO.UIOption]::OnlyErrorDialogs,",
    "[Microsoft.VisualBasic.FileIO.RecycleOption]::SendToRecycleBin)",
    "} else {",
    "[Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($source,",
    "[Microsoft.VisualBasic.FileIO.UIOption]::OnlyErrorDialogs,",
    "[Microsoft.VisualBasic.FileIO.RecycleOption]::SendToRecycleBin)",
    "}",
  ].join(" ");
  execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", command], {
    stdio: "ignore",
  });
  return "系统回收站";
}

function moveToLinuxTrash(source) {
  for (const [command, args] of [
    ["gio", ["trash", source]],
    ["trash-put", [source]],
  ]) {
    try {
      execFileSync(command, args, { stdio: "ignore" });
      return "系统废纸篓";
    } catch {
      // Try the next desktop trash implementation.
    }
  }
  return null;
}

function moveToRecoverableFallback(source) {
  const fallbackDirectory = path.join(os.tmpdir(), "DeepSeekHarnessDesk-Trash");
  fs.mkdirSync(fallbackDirectory, { recursive: true });
  const target = uniquePath(fallbackDirectory, path.basename(source));
  try {
    fs.renameSync(source, target);
  } catch (error) {
    if (error?.code !== "EXDEV") throw error;
    fs.cpSync(source, target, { recursive: true });
    fs.rmSync(source, { force: true, recursive: true });
  }
  return "可恢复临时回收目录";
}

function moveToTrash(source) {
  if (process.platform === "darwin") {
    return moveToMacTrash(source);
  }
  if (process.platform === "win32") {
    try {
      return moveToWindowsRecycleBin(source);
    } catch {
      return moveToRecoverableFallback(source);
    }
  }
  if (process.platform === "linux") {
    const trash = moveToLinuxTrash(source);
    if (trash) return trash;
  }
  return moveToRecoverableFallback(source);
}

for (const directory of outputDirectories) {
  if (!fs.existsSync(directory)) continue;

  for (const entry of fs.readdirSync(directory)) {
    const source = path.join(directory, entry);
    const destination = moveToTrash(source);
    console.log(`已将历史测试包移入${destination}：${path.relative(repositoryRoot, source)}`);
  }
  console.log(`测试包目录已清理，下一次构建后仅保留最新输出：${path.relative(repositoryRoot, directory) || "."}`);
}
