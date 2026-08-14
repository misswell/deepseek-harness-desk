#!/bin/zsh
set -euo pipefail

repo_dir="$(cd "$(dirname "$0")/.." && pwd)"
assets_dir="$repo_dir/Assets.xcassets/AppIcon.appiconset"
source_png="$repo_dir/Assets/DeepSeekHarnessIcon-Source.png"
transparent_source="$repo_dir/Assets/DeepSeekHarnessIcon-Transparent.png"
master_png="$repo_dir/Assets/DeepSeekHarnessIcon-Prepared-1024.png"
dock_master_png="$repo_dir/Assets/DeepSeekHarnessIcon-Dock-1024.png"
status_dir="$repo_dir/Assets.xcassets/StatusBarIcon.imageset"

mkdir -p "$assets_dir"

# The supplied source is the official DeepSeek Harness logo.  Remove the white
# avatar background, enlarge it to a 1024px master, then normalize it into a
# macOS-safe transparent canvas before the Xcode asset compiler resizes it.
# Keep the 1024px master outside the appiconset so Contents.json lists every packaged layer.
xcrun swift "$repo_dir/scripts/make_logo_transparent.swift" \
  --input "$source_png" \
  --output "$transparent_source"

xcrun sips -z 1024 1024 "$transparent_source" --out "$repo_dir/Assets/DeepSeekHarnessIcon-1024.png" >/dev/null
icon_preparation_script="/Users/guofeng/.codex/skills/generate-app-icons/scripts/prepare_macos_icon.swift"
if [[ -f "$icon_preparation_script" ]]; then
  xcrun swift "$icon_preparation_script" \
    --input "$repo_dir/Assets/DeepSeekHarnessIcon-1024.png" \
    --output "$master_png" \
    --scale 0.825
else
  # The external helper is optional; the transparent 1024px whale remains a
  # valid source for the Dock and status-bar variants.
  cp "$repo_dir/Assets/DeepSeekHarnessIcon-1024.png" "$master_png"
fi

xcrun swift "$repo_dir/scripts/make_dock_icon.swift" \
  --input "$master_png" \
  --output "$dock_master_png"

mkdir -p "$status_dir"
status_source="$repo_dir/Assets/DeepSeekHarnessIcon-Status-White.png"
xcrun swift "$repo_dir/scripts/make_logo_transparent.swift" \
  --input "$source_png" \
  --output "$status_source" \
  --white
sips -z 18 18 "$status_source" --out "$status_dir/statusbar_whale.png" >/dev/null
sips -z 36 36 "$status_source" --out "$status_dir/statusbar_whale@2x.png" >/dev/null

for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$dock_master_png" --out "$assets_dir/icon_${size}x${size}.png" >/dev/null
done
sips -z 32 32 "$dock_master_png" --out "$assets_dir/icon_16x16@2x.png" >/dev/null
sips -z 64 64 "$dock_master_png" --out "$assets_dir/icon_32x32@2x.png" >/dev/null
sips -z 256 256 "$dock_master_png" --out "$assets_dir/icon_128x128@2x.png" >/dev/null
sips -z 512 512 "$dock_master_png" --out "$assets_dir/icon_256x256@2x.png" >/dev/null
sips -z 1024 1024 "$dock_master_png" --out "$assets_dir/icon_512x512@2x.png" >/dev/null

echo "Prepared $assets_dir"
