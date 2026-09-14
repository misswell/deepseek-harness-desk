/**
 * Paste bridge for the desktop shell.
 *
 * Two things are broken about pasting into the Harness composer:
 *
 * - the Harness page keeps file pastes to itself: its paste command claims the
 *   event, prevents the default insertion and then drops the files, so pasting
 *   a screenshot or a file silently does nothing (plain text still works);
 * - the composer lives in a cross-origin iframe, so the shell frame can never
 *   reach it, and WebKit hands the shell frame none of the file payload.
 *
 * Both are fixed on the page side, where the files have to end up anyway:
 *
 * - main frame: `window.__dshDeskPasteFiles(payloadJson)` is called by the
 *   native layer with the `NSPasteboard` payload and forwards it to the Harness
 *   iframe; bridge failures surface through the shell toast.
 * - Harness frame: file pastes are intercepted in the capture phase and the
 *   files are attached through the composer's own `input[type=file]` (the
 *   paperclip button's path, which the composer does honor). Bridge payloads
 *   from the native layer take the same route.
 *
 * Keeping both halves here (instead of splitting them across `main.js`) means
 * the mechanism does not depend on module load order and can be unit tested as
 * one unit. See `tests/paste-bridge.test.mjs`.
 */
(function () {
  "use strict";

  var CHANNEL = "__dsh_desk_paste__";
  var ACK_CHANNEL = "__dsh_desk_paste_ack__";
  var FRAME_ID = "harness-frame";
  var MAX_FILES = 40;

  // The only user-facing strings this low-level bridge owns; the rest of the
  // shell copy lives in i18n.js. Chosen from the language the shell applied to
  // <html lang>, so both supported languages stay covered.
  var MESSAGES = {
    zh: {
      notReady: "Harness 页面尚未就绪，无法粘贴",
      noComposer: "请先进入一个会话，再粘贴图片或文件",
      failed: "粘贴失败，请重试或用回形针按钮添加文件",
      skipped: "已跳过 {count} 个过大的文件",
    },
    en: {
      notReady: "The Harness page is not ready yet",
      noComposer: "Open a session before pasting an image or file",
      failed: "Paste failed; try again or use the paperclip button",
      skipped: "Skipped {count} oversized file(s)",
    },
  };

  function language() {
    try {
      var lang = document.documentElement && document.documentElement.lang;
      return String(lang || "").toLowerCase().indexOf("en") === 0 ? "en" : "zh";
    } catch (error) {
      return "zh";
    }
  }

  function message(key, params) {
    var table = MESSAGES[language()] || MESSAGES.zh;
    var text = table[key] || key;
    if (params) {
      for (var name in params) {
        if (Object.prototype.hasOwnProperty.call(params, name)) {
          text = text.split("{" + name + "}").join(String(params[name]));
        }
      }
    }
    return text;
  }

  /** Shows a shell toast without depending on main.js being initialized. */
  function shellToast(text, isError) {
    try {
      var toast = document.getElementById("toast");
      if (!toast) return;
      toast.textContent = text;
      toast.classList.toggle("error", Boolean(isError));
      toast.classList.remove("hidden");
      if (shellToast.timer) clearTimeout(shellToast.timer);
      shellToast.timer = setTimeout(function () {
        toast.classList.add("hidden");
      }, 3800);
    } catch (error) {
      // A failed toast must never break pasting.
    }
  }

  function installShellSide() {
    /**
     * Called from the native layer with a JSON payload: an `id`, `files`
     * (each `{ name, type, data }` where `data` is base64) and the names of
     * files the native layer refused as `skipped`.
     */
    window.__dshDeskPasteFiles = function (payloadJson) {
      var payload = null;
      try {
        payload = typeof payloadJson === "string" ? JSON.parse(payloadJson) : payloadJson;
      } catch (error) {
        shellToast(message("failed"), true);
        return false;
      }
      if (!payload || !Array.isArray(payload.files) || payload.files.length === 0) return false;

      var frame = document.getElementById(FRAME_ID);
      var target = frame && frame.contentWindow;
      if (!target) {
        shellToast(message("notReady"), true);
        return false;
      }
      target.postMessage(
        { channel: CHANNEL, pasteId: payload.id || null, files: payload.files },
        "*",
      );
      if (Array.isArray(payload.skipped) && payload.skipped.length > 0) {
        shellToast(message("skipped", { count: payload.skipped.length }), true);
      }
      return true;
    };

    window.addEventListener("message", function (event) {
      var data = event.data;
      if (!data || data.channel !== ACK_CHANNEL) return;
      if (data.ok) return;
      shellToast(message(data.reason === "no-composer" ? "noComposer" : "failed"), true);
    });
  }

  function installFrameSide() {
    // Pasting an image or a file inside the Harness page does nothing on its
    // own: the composer's paste command takes the event, prevents the default
    // insertion and then drops the files. The paperclip button, however, goes
    // through the composer's own `input[type=file]`, which works. So intercept
    // every file paste and hand the files to that input instead.
    document.addEventListener("paste", onNativePaste, true);
    window.addEventListener("message", onBridgeMessage);
  }

  function onNativePaste(event) {
    try {
      if (isFormField(event.target)) return;
      var files = filesFromTransfer(event.clipboardData);
      if (files.length === 0) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      attach(files);
    } catch (error) {
      // Never let the interception break the page's own paste handling.
    }
  }

  function onBridgeMessage(event) {
    if (event.source !== window.parent) return;
    var data = event.data;
    if (!data || data.channel !== CHANNEL || !Array.isArray(data.files)) return;
    deliver(data).catch(function (error) {
      ack(data, false, "failed:" + String((error && error.message) || error));
    });
  }

  async function deliver(data) {
    var files = [];
    for (var index = 0; index < data.files.length && index < MAX_FILES; index += 1) {
      files.push(await buildFile(data.files[index]));
    }
    if (files.length === 0) {
      ack(data, false, "failed:no-files");
      return;
    }
    var result = attach(files);
    ack(data, result.ok, result.reason || null);
  }

  /** Hands files to the composer, preferring the paperclip's own file input. */
  function attach(files) {
    var input = findFileInput();
    if (input) {
      try {
        var transfer = new DataTransfer();
        for (var index = 0; index < files.length; index += 1) transfer.items.add(files[index]);
        input.files = transfer.files;
        if (input.files && input.files.length === files.length) {
          input.dispatchEvent(new Event("change", { bubbles: true }));
          return { ok: true };
        }
      } catch (error) {
        // Fall through to the paste-event route below.
      }
    }

    var editor = findComposer();
    if (!editor) return { ok: false, reason: "no-composer" };
    editor.focus();
    var paste = new ClipboardEvent("paste", {
      bubbles: true,
      cancelable: true,
      clipboardData: transferOf(files),
    });
    editor.dispatchEvent(paste);
    return { ok: true };
  }

  function transferOf(files) {
    var transfer = new DataTransfer();
    for (var index = 0; index < files.length; index += 1) transfer.items.add(files[index]);
    return transfer;
  }

  function filesFromTransfer(data) {
    if (!data) return [];
    var files = [];
    try {
      var listed = data.files;
      for (var index = 0; listed && index < listed.length; index += 1) files.push(listed[index]);
    } catch (error) {
      // `files` may be unavailable; the items below still carry the payload.
    }
    if (files.length > 0) return files;

    try {
      var items = data.items || [];
      for (var itemIndex = 0; itemIndex < items.length; itemIndex += 1) {
        var item = items[itemIndex];
        if (item.kind !== "file") continue;
        var file = item.getAsFile();
        if (file) files.push(file);
      }
    } catch (error) {
      return [];
    }
    return files;
  }

  function isFormField(node) {
    if (!node) return false;
    var name = node.tagName;
    return name === "INPUT" || name === "TEXTAREA";
  }

  /** The composer's hidden input, i.e. what the paperclip button opens. */
  function findFileInput() {
    var inputs = document.querySelectorAll('input[type="file"]');
    var fallback = null;
    for (var index = 0; index < inputs.length; index += 1) {
      var input = inputs[index];
      if (input.disabled) continue;
      if (input.multiple) return input;
      if (!fallback) fallback = input;
    }
    return fallback;
  }

  /** The Harness composer is a Lexical contenteditable. */
  function findComposer() {
    var nodes = document.querySelectorAll('[data-lexical-editor="true"]');
    for (var index = 0; index < nodes.length; index += 1) {
      var node = nodes[index];
      if (node.getAttribute("aria-disabled") === "true") continue;
      var rect = node.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) return node;
    }
    return null;
  }

  async function buildFile(entry) {
    if (!entry || typeof entry.data !== "string") throw new Error("bad-entry");
    var bytes = base64Bytes(entry.data);
    var type = entry.type || "application/octet-stream";
    var name = entry.name || "pasted";
    var file = new File([bytes], name, { type: type });
    // WebKit hands some copy sources (Preview, Finder previews) a TIFF
    // pasteboard flavor, and the Harness attachment store only accepts
    // png/jpeg/webp/gif. Re-encode anything else the browser can decode.
    if (!isAcceptedImageType(type) && type.indexOf("image/") === 0) {
      var converted = await reencodeAsPng(file);
      if (converted) return converted;
    }
    return file;
  }

  function isAcceptedImageType(type) {
    return (
      type === "image/png" ||
      type === "image/jpeg" ||
      type === "image/webp" ||
      type === "image/gif"
    );
  }

  async function reencodeAsPng(file) {
    var url = null;
    try {
      url = URL.createObjectURL(file);
      var image = await loadImage(url);
      var width = image.naturalWidth || image.width;
      var height = image.naturalHeight || image.height;
      if (!width || !height) return null;
      var canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      canvas.getContext("2d").drawImage(image, 0, 0);
      var blob = await new Promise(function (resolve) {
        canvas.toBlob(resolve, "image/png");
      });
      if (!blob) return null;
      var name = String(file.name || "pasted").replace(/\.[^./\\]+$/, "") + ".png";
      return new File([blob], name, { type: "image/png" });
    } catch (error) {
      return null;
    } finally {
      if (url) URL.revokeObjectURL(url);
    }
  }

  function loadImage(url) {
    return new Promise(function (resolve, reject) {
      var image = new Image();
      image.onload = function () {
        resolve(image);
      };
      image.onerror = function () {
        reject(new Error("image-decode-failed"));
      };
      image.src = url;
    });
  }

  function base64Bytes(base64) {
    var binary = atob(base64);
    var bytes = new Uint8Array(binary.length);
    for (var index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return bytes;
  }

  function ack(data, ok, reason) {
    try {
      window.parent.postMessage(
        { channel: ACK_CHANNEL, pasteId: (data && data.pasteId) || null, ok: ok, reason: reason || null },
        "*",
      );
    } catch (error) {
      // The shell may already be gone; nothing to report to.
    }
  }

  try {
    if (window.top === window) {
      installShellSide();
    } else {
      installFrameSide();
    }
  } catch (error) {
    // Never break the page that hosts this bridge.
  }
})();
