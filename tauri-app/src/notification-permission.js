// Notification permission reporting is a platform concern, not a preference:
// the backend answers "did macOS grant us the right to post banners", and the
// shell only decides whether the settings page should surface a hint. Keeping
// the decision here keeps it testable and independent of the backend shape.

export function normalizeNotificationPermission(payload) {
  const value = payload && typeof payload === "object" ? payload : {};
  return {
    granted: value.granted === true,
    determined: value.determined === true,
  };
}

// Only a platform that both understands permission and answered "denied"
// deserves a hint; anything else (undetermined, granted, other platforms)
// stays quiet so the settings page does not nag.
export function shouldShowNotificationPermissionHint(permission) {
  return permission.determined && !permission.granted;
}
