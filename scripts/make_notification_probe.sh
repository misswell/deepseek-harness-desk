#!/bin/bash
# Build and launch the notification transport probe as a real .app bundle.
#
# UNUserNotificationCenter refuses to work outside an app bundle (the framework
# raises unless the process has a bundle identifier), so the probe is packaged
# like the shell instead of being run as a bare executable.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="${TMPDIR:-/tmp}/dsh-notification-probe"
# A real location, not a temp dir: LaunchServices refuses to open bundles it
# cannot register, and a bundle in $TMPDIR keeps falling out of its database.
APP="$HOME/Applications/NotificationProbe.app"
IDENTITY="Developer ID Application: Guofeng Liu (U8U443D7ZL)"

rm -rf "$BUILD_DIR"
mkdir -p "$APP/Contents/MacOS"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>NotificationProbe</string>
  <key>CFBundleIdentifier</key>
  <string>com.deepseek.harnessdesk.notificationprobe</string>
  <key>CFBundleName</key>
  <string>Notification Probe</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>1.0</string>
  <key>LSUIElement</key>
  <true/>
</dict>
</plist>
PLIST

swiftc -O -o "$APP/Contents/MacOS/NotificationProbe" "$HERE/notification_probe.swift" \
  -framework AppKit -framework UserNotifications

# Sign with the real Developer ID. An unsigned or ad-hoc bundle gets its
# notification authorization denied outright, which would make the probe
# measure the signature instead of the transport.
xattr -cr "$APP" 2>/dev/null || true
codesign --force --deep --sign "$IDENTITY" --options runtime --timestamp "$APP" >/dev/null

LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
"$LSREGISTER" -f "$APP"

echo "built and signed $APP"
codesign -dv --verbose=2 "$APP" 2>&1 | grep -E "Identifier|TeamIdentifier" | head -2
echo "running probe (grant the permission prompt if macOS shows one)…"
open -W "$APP" || true
echo "--- probe output ---"
cat "${TMPDIR:-/tmp}/dsh-notification-probe.log" 2>/dev/null || echo "(no log written)"
