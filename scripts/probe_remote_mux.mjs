// Probe the live dsh remote.mux the way the shell's notification watcher now
// does: connect with the minted auth cookie, open the `$events` logical
// stream, and dump the raw frames that come back.
// Usage: node scripts/probe_remote_mux.mjs <port> <token> [seconds]
import { createRequire } from "node:module";
import { resolve } from "node:path";

const runtimeModules = resolve(
  process.env.HOME,
  "Library/Application Support/DeepSeek Harness Desk/runtime/dsh/0.1.5-rc.2/node_modules",
);
const require = createRequire(resolve(runtimeModules, "noop.js"));
const WebSocket = require("ws");

const [, , portArg, token, secondsArg] = process.argv;
const port = portArg || "3080";
const seconds = Number(secondsArg || 20);

const loginRes = await fetch(`http://127.0.0.1:${port}/?token=${token}`, { redirect: "manual" });
const setCookie = loginRes.headers.get("set-cookie");
console.log("login:", loginRes.status, setCookie ? setCookie.split(";")[0] : "(no cookie)");
if (!setCookie) process.exit(1);
const cookie = setCookie.split(";")[0];

const ws = new WebSocket(`ws://127.0.0.1:${port}/api/remote.mux`, { headers: { cookie } });
const started = Date.now();
const stamp = () => ((Date.now() - started) / 1000).toFixed(1);

ws.on("open", () => {
  console.log("=== connected; opening $events ===");
  ws.send(
    JSON.stringify({
      type: "open",
      streamId: "desk-events",
      endpoint: "$events",
      payload: { args: {} },
    }),
  );
});
ws.on("message", (data) => {
  console.log(`[${stamp()}s] ${data.toString().slice(0, 700)}`);
});
ws.on("close", (code, reason) => console.log(`=== closed ${code} ${reason}`));
ws.on("error", (error) => console.log("=== error:", error.message));

setTimeout(() => {
  console.log("=== capture window ended ===");
  ws.close();
  process.exit(0);
}, seconds * 1000);
