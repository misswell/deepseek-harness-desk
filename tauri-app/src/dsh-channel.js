/**
 * Update channel handling for the bundled DeepSeek Harness (`dsh`).
 *
 * By default the bundled updater follows the npm `latest`/`next` dist-tags —
 * the release candidates the harness team publishes as its normal stream. The
 * preview channel additionally considers the `alpha`/`beta` tags, which is
 * where early builds live, so a beta can be detected and installed on purpose
 * instead of by accident.
 *
 * Everything the settings page needs to describe a check result lives here so
 * it can be unit tested without a DOM.
 */

export const DSH_CHANNEL_STORAGE_KEY = "dshUpdateChannel";
export const DSH_CHANNEL_STABLE = "stable";
export const DSH_CHANNEL_PREVIEW = "preview";

/** Coerce a stored or user-selected value into a known channel. */
export function normalizeDshChannel(value) {
  return value === DSH_CHANNEL_PREVIEW ? DSH_CHANNEL_PREVIEW : DSH_CHANNEL_STABLE;
}

export function isPreviewChannel(value) {
  return normalizeDshChannel(value) === DSH_CHANNEL_PREVIEW;
}

/** Channel persisted in the shell, defaulting to the stable channel. */
export function storedDshChannel() {
  try {
    return normalizeDshChannel(localStorage.getItem(DSH_CHANNEL_STORAGE_KEY));
  } catch {
    return DSH_CHANNEL_STABLE;
  }
}

/**
 * True when a version string is a preview build (`0.1.6-alpha.2`,
 * `0.1.6-beta.1`). Mirrors the backend rule: the prerelease label must match
 * exactly, so `0.1.6-alphabet.1` is not treated as a preview. Release
 * candidates (`0.1.5-rc.2`) belong to the stable channel.
 */
export function isPreviewDshVersion(version) {
  if (typeof version !== "string") return false;
  const [, prerelease] = splitVersion(version);
  return prerelease
    .split(".")
    .filter(Boolean)
    .some((part) => part.toLowerCase() === "alpha" || part.toLowerCase() === "beta");
}

/** Split `0.1.6-alpha.2` into `["0.1.6", "alpha.2"]`, ignoring a leading `v`. */
function splitVersion(version) {
  const normalized = String(version).trim().replace(/^[vV]/, "");
  const separator = normalized.indexOf("-");
  if (separator < 0) return [normalized, ""];
  return [normalized.slice(0, separator), normalized.slice(separator + 1)];
}

/**
 * Localized status line for a `check_dsh_update` result.
 *
 * The backend also returns a `status` string, but it is not localized; the
 * shell derives the sentence from the structured fields instead.
 */
export function dshUpdateStatusText(update, t) {
  if (!update || update.managed !== true) return t("updates.dsh.notManaged");
  const channel = normalizeDshChannel(update.channel);
  const current = update.current_version || "—";
  if (update.available === true) {
    const vars = { version: update.latest_version || "" };
    return update.preview === true
      ? t("updates.dsh.newPreview", vars)
      : t("updates.dsh.newVersion", vars);
  }
  if (update.current_is_preview === true) {
    return channel === DSH_CHANNEL_PREVIEW
      ? t("updates.dsh.previewUpToDate", { version: current })
      : t("updates.dsh.previewNewerThanStable", {
          current,
          stable: update.latest_version || "—",
        });
  }
  return t("updates.dsh.upToDate", { version: current });
}

/**
 * Hint shown when npm has a preview build that is newer than the installed
 * version while the stable channel is selected — i.e. "a beta is available".
 * Returns null when there is nothing to announce.
 */
export function dshPreviewHintText(update, t) {
  if (!update || update.managed !== true) return null;
  if (isPreviewChannel(update.channel)) return null;
  if (update.current_is_preview === true) return null;
  if (!update.preview_version) return null;
  return t("updates.dsh.previewAvailable", { version: update.preview_version });
}
