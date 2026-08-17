export const DEFAULT_ZOOM = 1;
export const MIN_ZOOM = 0.75;
export const MAX_ZOOM = 1.75;
export const ZOOM_STEP = 0.1;

function roundZoom(value) {
  return Math.round(value * 100) / 100;
}

export function clampZoom(value) {
  const numericValue = Number(value);
  if (!Number.isFinite(numericValue)) return DEFAULT_ZOOM;
  return roundZoom(Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, numericValue)));
}

export function zoomFromShortcut(event, currentZoom) {
  if (!event || (!event.metaKey && !event.ctrlKey) || event.altKey) return null;

  const key = event.key || "";
  const code = event.code || "";
  const zoom = clampZoom(currentZoom);

  if (key === "+" || key === "=" || code === "NumpadAdd") {
    return clampZoom(zoom + ZOOM_STEP);
  }
  if (key === "-" || key === "_" || code === "NumpadSubtract") {
    return clampZoom(zoom - ZOOM_STEP);
  }
  if (key === "0" || code === "Numpad0") {
    return DEFAULT_ZOOM;
  }
  return null;
}
