// Verifies the cross-process paste bridge:
// - the injected script runs in every frame (shell + cross-origin Harness frame),
// - file pastes inside the Harness page are rerouted to the composer's own
//   `input[type=file]` (the paperclip path, the only route the composer honors),
// - the shell frame forwards the native payload and surfaces bridge failures,
// - the Tauri layer injects the script into all frames, wires the native paste
//   handler into setup, and stops swallowing native file drops.
import assert from "assert";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const bridgeSource = readFileSync(join(root, "src", "paste-bridge.js"), "utf8");
const libSource = readFileSync(join(root, "src-tauri", "src", "lib.rs"), "utf8");
const conf = JSON.parse(
  readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);

const CHANNEL = "__dsh_desk_paste__";
const ACK_CHANNEL = "__dsh_desk_paste_ack__";

class FakeFile {
  constructor(parts, name, options = {}) {
    this.parts = parts;
    this.name = name;
    this.type = options.type || "";
    this.size = parts[0] ? parts[0].length : 0;
  }
}

class FakeClipboardEvent {
  constructor(type, options = {}) {
    this.type = type;
    this.bubbles = options.bubbles;
    this.cancelable = options.cancelable;
    this.clipboardData = options.clipboardData;
  }
}

function makeDataTransfer(files = []) {
  const transfer = { files: [], types: [], items: null };
  transfer.items = {
    add(file) {
      transfer.files.push(file);
      if (!transfer.types.includes("Files")) transfer.types.push("Files");
    },
  };
  for (const file of files) transfer.items.add(file);
  return transfer;
}

/** Runs the injected bridge in a fake frame that has the given role. */
function runBridge({ shell, editor = true, frame = true, fileInput = true }) {
  const events = [];
  const posted = [];
  const windowListeners = [];
  const pasteListeners = [];
  const toastNode = {
    textContent: "",
    classList: { add() {}, remove() {}, toggle() {} },
  };
  const editorNode = {
    focused: false,
    getAttribute: () => null,
    getBoundingClientRect: () => ({ width: 240, height: 40 }),
    focus() {
      this.focused = true;
    },
    dispatchEvent(event) {
      events.push(event);
      return true;
    },
  };
  const changes = [];
  const inputNode = {
    disabled: false,
    multiple: true,
    files: [],
    dispatchEvent(event) {
      changes.push(event);
      return true;
    },
  };
  const fileInputs = fileInput ? [inputNode] : [];
  const frameNode = frame
    ? { contentWindow: { postMessage: (message, origin) => posted.push({ message, origin }) } }
    : null;

  const window = {
    addEventListener(type, listener) {
      if (type === "message") windowListeners.push(listener);
    },
    parent: { postMessage: (message, origin) => posted.push({ message, origin }) },
  };
  window.top = shell ? window : {};

  const context = {
    window,
    document: {
      documentElement: { lang: "zh-CN" },
      addEventListener(type, listener, capture) {
        if (type === "paste") pasteListeners.push({ listener, capture });
      },
      getElementById(id) {
        if (id === "toast") return toastNode;
        if (id === "harness-frame") return frameNode;
        return null;
      },
      querySelectorAll(selector) {
        if (selector.indexOf("input") === 0) return fileInputs;
        return editor ? [editorNode] : [];
      },
      createElement() {
        throw new Error("canvas unavailable in test");
      },
    },
    DataTransfer: function DataTransfer() {
      return makeDataTransfer();
    },
    Event: class FakeEvent {
      constructor(type, options = {}) {
        this.type = type;
        this.bubbles = options.bubbles;
      }
    },
    File: FakeFile,
    ClipboardEvent: FakeClipboardEvent,
    URL: {
      createObjectURL() {
        throw new Error("no blob urls in test");
      },
      revokeObjectURL() {},
    },
    Image: class Image {},
    atob: (value) => Buffer.from(value, "base64").toString("binary"),
    setTimeout,
    clearTimeout,
    console,
  };
  vm.createContext(context);
  vm.runInContext(bridgeSource, context);

  return {
    window,
    events,
    posted,
    changes,
    inputNode,
    toastNode,
    windowListeners,
    message: (data) => {
      for (const listener of windowListeners) {
        listener({ source: window.parent, data });
      }
    },
    paste: (target, files) => {
      const event = new FakeClipboardEvent("paste", {
        bubbles: true,
        cancelable: true,
        clipboardData: makeDataTransfer(files),
      });
      event.target = target;
      event.defaultPrevented = false;
      event.propagationStopped = false;
      event.preventDefault = () => {
        event.defaultPrevented = true;
      };
      event.stopImmediatePropagation = () => {
        event.propagationStopped = true;
      };
      for (const entry of pasteListeners) entry.listener(event);
      return event;
    },
    pasteListenerCount: pasteListeners.length,
  };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const png = (name) => new FakeFile([Buffer.from("hello-png")], name, { type: "image/png" });

// --- Harness frame: the native payload goes through the composer's file input -

{
  const frame = runBridge({ shell: false });
  const base64 = Buffer.from("hello-png").toString("base64");
  frame.message({
    channel: CHANNEL,
    pasteId: 7,
    files: [{ name: "shot.png", type: "image/png", data: base64 }],
  });
  await settle();

  assert.equal(frame.changes.length, 1, "the composer file input must be told about the files");
  assert.equal(frame.changes[0].type, "change", "the input must fire the change event it listens for");
  assert.equal(frame.changes[0].bubbles, true, "the change event must bubble");
  assert.equal(frame.inputNode.files.length, 1, "one file must be handed to the input");
  assert.equal(frame.inputNode.files[0].name, "shot.png");
  assert.equal(frame.inputNode.files[0].type, "image/png");
  assert.equal(frame.inputNode.files[0].size, "hello-png".length);
  assert.equal(frame.events.length, 0, "the file input route must not also dispatch a paste event");
  assert.equal(frame.window.__dshDeskPasteFiles, undefined, "the frame must not define the shell entry point");
  assert.deepEqual(
    frame.posted.map((entry) => entry.message),
    [{ channel: ACK_CHANNEL, pasteId: 7, ok: true, reason: null }],
    "a successful bridge must acknowledge the shell",
  );
}

// --- Harness frame: file paste events are intercepted and rerouted ------------

{
  const frame = runBridge({ shell: false });
  assert.equal(frame.pasteListenerCount, 1, "the frame must watch paste events in the capture phase");
  const event = frame.paste({ tagName: "DIV" }, [png("clipboard.png")]);

  assert.equal(event.defaultPrevented, true, "the composer must not swallow the file paste");
  assert.equal(event.propagationStopped, true, "the broken paste handling must not run");
  assert.equal(frame.changes.length, 1, "the intercepted files must reach the composer file input");
  assert.equal(frame.inputNode.files[0].name, "clipboard.png");
}

{
  // Several files at once (Finder multi-select) must all be attached.
  const frame = runBridge({ shell: false });
  const event = frame.paste({ tagName: "DIV" }, [
    png("one.png"),
    new FakeFile([Buffer.from("doc")], "two.pdf", { type: "application/pdf" }),
  ]);
  assert.equal(event.defaultPrevented, true);
  assert.deepEqual(
    Array.from(frame.inputNode.files, (file) => file.name),
    ["one.png", "two.pdf"],
  );
}

{
  const frame = runBridge({ shell: false });
  const event = frame.paste({ tagName: "DIV" }, []);
  assert.equal(event.defaultPrevented, false, "a text paste must stay untouched");
  assert.equal(frame.changes.length, 0);
}

{
  // Pasting while a plain form field owns the event must not be hijacked.
  const frame = runBridge({ shell: false });
  const event = frame.paste({ tagName: "INPUT" }, [png("clipboard.png")]);
  assert.equal(event.defaultPrevented, false, "form fields keep their own paste handling");
  assert.equal(frame.changes.length, 0);
}

// --- Harness frame: fallbacks -------------------------------------------------

{
  // Without the composer file input the paste event is still worth dispatching.
  const frame = runBridge({ shell: false, fileInput: false });
  frame.message({
    channel: CHANNEL,
    pasteId: 8,
    files: [{ name: "shot.png", type: "image/png", data: "AAAA" }],
  });
  await settle();
  assert.equal(frame.changes.length, 0);
  assert.equal(frame.events.length, 1, "the paste event is the last resort");
  assert.equal(frame.events[0].type, "paste");
  assert.equal(frame.events[0].clipboardData.files[0].name, "shot.png");
  assert.equal(frame.posted[0].message.ok, true);
}

{
  const frame = runBridge({ shell: false, editor: false, fileInput: false });
  frame.message({ channel: CHANNEL, pasteId: 1, files: [{ name: "a.png", type: "image/png", data: "" }] });
  await settle();
  assert.equal(frame.events.length, 0, "without a composer nothing is delivered");
  assert.equal(frame.posted[0].message.reason, "no-composer", "the shell must learn why nothing happened");
  assert.equal(frame.posted[0].message.ok, false);
}

{
  // TIFF is not an accepted attachment type, but the Harness frame cannot
  // re-encode it in this environment: the original file must still be handed
  // over instead of dropping the paste silently.
  const frame = runBridge({ shell: false });
  frame.message({
    channel: CHANNEL,
    pasteId: 2,
    files: [{ name: "shot.tiff", type: "image/tiff", data: Buffer.from("tiff").toString("base64") }],
  });
  await settle();
  assert.equal(frame.changes.length, 1, "an unencodable image must still be handed over");
  assert.equal(frame.inputNode.files[0].type, "image/tiff");
}

{
  const frame = runBridge({ shell: false });
  frame.message({ channel: ACK_CHANNEL, files: [{ name: "x", type: "image/png", data: "" }] });
  await settle();
  assert.equal(frame.changes.length, 0, "the frame must ignore unrelated messages");
}

// --- Shell frame: forward the native payload ---------------------------------

{
  const shell = runBridge({ shell: true });
  assert.equal(typeof shell.window.__dshDeskPasteFiles, "function", "the native layer needs a shell entry point");
  const files = [{ name: "shot.png", type: "image/png", data: "AAAA" }];
  const handled = shell.window.__dshDeskPasteFiles(JSON.stringify({ id: 4, files, skipped: [] }));

  assert.equal(handled, true);
  assert.equal(shell.posted.length, 1, "the payload must be forwarded to the Harness frame");
  assert.deepEqual(shell.posted[0].message, { channel: CHANNEL, pasteId: 4, files });
  assert.equal(shell.posted[0].origin, "*", "the Harness frame is cross-origin");

  // Oversized files that the native layer refused must be reported.
  shell.window.__dshDeskPasteFiles(JSON.stringify({ id: 5, files, skipped: ["huge.mov"] }));
  assert.equal(shell.posted.length, 2);
  assert.equal(shell.toastNode.textContent, "已跳过 1 个过大的文件");
}

{
  const shell = runBridge({ shell: true, frame: false });
  assert.equal(shell.window.__dshDeskPasteFiles(JSON.stringify({ id: 1, files: [{ name: "a", type: "image/png", data: "" }] })), false);
  assert.equal(shell.posted.length, 0, "a missing Harness frame cannot receive anything");
  assert.match(shell.toastNode.textContent, /尚未就绪/, "the shell must explain the failure");
}

{
  const shell = runBridge({ shell: true });
  assert.equal(shell.window.__dshDeskPasteFiles("{not json"), false, "a malformed payload must not throw");
  assert.equal(shell.window.__dshDeskPasteFiles(JSON.stringify({ id: 6, files: [] })), false, "an empty payload is a no-op");

  // A failed bridge in the Harness frame must surface in the shell.
  for (const listener of shell.windowListeners) {
    listener({ data: { channel: ACK_CHANNEL, pasteId: 6, ok: false, reason: "no-composer" } });
  }
  assert.match(shell.toastNode.textContent, /先进入一个会话/, "the shell must translate the frame failure");
}

// --- Native wiring -----------------------------------------------------------

assert.match(
  libSource,
  /WKUserScript::initWithSource_injectionTime_forMainFrameOnly\([\s\S]*?WKUserScriptInjectionTime::AtDocumentStart,\s*false,?\s*\)/,
  "the bridge must be injected into every frame at document start",
);
assert.match(
  libSource,
  /const PASTE_BRIDGE_SCRIPT: &str = include_str!\("\.\.\/\.\.\/src\/paste-bridge\.js"\)/,
  "the injected script must be the tested paste-bridge.js",
);
assert.match(
  libSource,
  /paste_bridge::install\(app\.handle\(\)\)/,
  "setup must install the native paste handler",
);
assert.match(
  libSource,
  /addLocalMonitorForEventsMatchingMask_handler\(NSEventMask::KeyDown/,
  "the native handler must observe Cmd+V key events",
);
assert.match(
  libSource,
  /NSPasteboardTypeFileURL/,
  "the native handler must read files copied in Finder",
);
assert.match(
  libSource,
  /window\.__dshDeskPasteFiles/,
  "the native handler must call into the shell bridge",
);

const mainWindow = conf.app.windows.find((window) => window.label === "main");
assert.equal(
  mainWindow.dragDropEnabled,
  false,
  "native drag handling must stay off so WebKit delivers HTML5 file drops to the Harness frame",
);

console.log("✓ paste bridge: native paste and in-page paste reach the Harness composer");
