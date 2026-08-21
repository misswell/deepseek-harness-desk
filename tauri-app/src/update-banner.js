export function shouldShowAppUpdateBanner(update, ignoredVersion, dismissedVersion) {
  if (!update || update.available !== true) return false;
  const version = update.latest_version;
  return ignoredVersion !== version && dismissedVersion !== version;
}
