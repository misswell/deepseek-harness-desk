import assert from "assert";

import {
  DEFAULT_ZOOM,
  MAX_ZOOM,
  MIN_ZOOM,
  ZOOM_STEP,
  clampZoom,
  zoomFromShortcut,
} from "../src/zoom.js";

function test(name, callback) {
  callback();
  console.log(`✓ ${name}`);
}

test("command or control plus increases zoom", () => {
  assert.equal(
    zoomFromShortcut({ key: "+", metaKey: true }, DEFAULT_ZOOM),
    DEFAULT_ZOOM + ZOOM_STEP,
  );
  assert.equal(
    zoomFromShortcut({ key: "=", ctrlKey: true }, DEFAULT_ZOOM),
    DEFAULT_ZOOM + ZOOM_STEP,
  );
});

test("numpad minus decreases zoom", () => {
  assert.equal(
    zoomFromShortcut({ code: "NumpadSubtract", ctrlKey: true }, 1.1),
    1.0,
  );
});

test("command or control zero resets zoom", () => {
  assert.equal(zoomFromShortcut({ key: "0", metaKey: true }, 1.6), DEFAULT_ZOOM);
  assert.equal(zoomFromShortcut({ code: "Numpad0", ctrlKey: true }, 0.8), DEFAULT_ZOOM);
});

test("shortcuts require command or control and ignore alternate shortcuts", () => {
  assert.equal(zoomFromShortcut({ key: "+" }, DEFAULT_ZOOM), null);
  assert.equal(zoomFromShortcut({ key: "+", ctrlKey: true, altKey: true }, DEFAULT_ZOOM), null);
  assert.equal(zoomFromShortcut({ key: "x", metaKey: true }, DEFAULT_ZOOM), null);
});

test("zoom values stay within the supported range", () => {
  assert.equal(clampZoom(0.2), MIN_ZOOM);
  assert.equal(clampZoom(3), MAX_ZOOM);
  assert.equal(clampZoom(Number.NaN), DEFAULT_ZOOM);
});
