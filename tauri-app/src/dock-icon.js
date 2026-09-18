/**
 * Dock icon styles for the shell.
 *
 * macOS repaints the Dock tile from `setApplicationIconImage`, but it only
 * reads that while the app is running: quitting falls back to the icon inside
 * the app bundle. The backend therefore also writes the chosen style into the
 * bundle as a Finder custom icon, and reports whether that write worked. This
 * module keeps the value coercion, the persisted preference and the wording
 * keys together so they can be unit tested without a DOM.
 */

export const DOCK_ICON_VARIANT_STORAGE_KEY = "dockIconVariant";
export const DEFAULT_DOCK_ICON_VARIANT = "blue";
export const DOCK_ICON_VARIANTS = ["blue", "black", "avatar"];

const TOAST_KEYS = {
  blue: "toast.dockIconBlue",
  black: "toast.dockIconBlack",
  avatar: "toast.dockIconAvatar",
};

/** Coerce a stored or user-selected value into a known Dock icon style. */
export function normalizeDockIconVariant(value) {
  return DOCK_ICON_VARIANTS.includes(value) ? value : DEFAULT_DOCK_ICON_VARIANT;
}

/** Style persisted in the shell, defaulting to the bundled blue icon. */
export function storedDockIconVariant() {
  try {
    return normalizeDockIconVariant(localStorage.getItem(DOCK_ICON_VARIANT_STORAGE_KEY));
  } catch {
    return DEFAULT_DOCK_ICON_VARIANT;
  }
}

/** Localized toast key confirming a switch. */
export function dockIconToastKey(variant) {
  return TOAST_KEYS[normalizeDockIconVariant(variant)];
}

/**
 * True when the running icon changed but the app bundle could not be written
 * to — the Dock tile changes now, yet the default icon comes back after the
 * app quits. Old backends that return nothing are treated as fine.
 */
export function dockIconPersistWarning(outcome) {
  return outcome?.applied === true && outcome?.persisted === false;
}
