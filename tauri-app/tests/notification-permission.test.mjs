// The settings page must not stay silent when macOS has notifications turned
// off for the app: the backend reports the platform verdict, and the shell
// decides whether to surface it.
import assert from "assert";
import {
  normalizeNotificationPermission,
  shouldShowNotificationPermissionHint,
} from "../src/notification-permission.js";

assert.deepEqual(normalizeNotificationPermission({ granted: true, determined: true }), {
  granted: true,
  determined: true,
});

// Anything that is not a positive boolean reads as "no permission": the hint
// must never be hidden by a malformed payload.
assert.deepEqual(normalizeNotificationPermission(undefined), {
  granted: false,
  determined: false,
});
assert.deepEqual(normalizeNotificationPermission(null), {
  granted: false,
  determined: false,
});
assert.deepEqual(normalizeNotificationPermission({ granted: "true", determined: 1 }), {
  granted: false,
  determined: false,
});

// Only a decided-and-denied permission deserves the hint. A not-yet-answered
// prompt (determined false) and a granted permission both stay quiet.
assert.equal(shouldShowNotificationPermissionHint({ granted: false, determined: true }), true);
assert.equal(shouldShowNotificationPermissionHint({ granted: false, determined: false }), false);
assert.equal(shouldShowNotificationPermissionHint({ granted: true, determined: true }), false);
assert.equal(shouldShowNotificationPermissionHint({ granted: true, determined: false }), false);

console.log("✓ notification permission hints only appear for a decided denial");
