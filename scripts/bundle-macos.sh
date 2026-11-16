#!/bin/sh
# Build Oxicord.app. Usage: scripts/bundle-macos.sh [--install]
set -eu
cd "$(dirname "$0")/.."

cargo build --release

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=target/release/Oxicord.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/oxicord "$APP/Contents/MacOS/oxicord"

if [ ! -f assets/icon.icns ]; then
    [ -f assets/icon.png ] || python3 scripts/make-icon.py assets/icon.png
    SET=target/icon.iconset
    rm -rf "$SET" && mkdir -p "$SET"
    for s in 16 32 128 256 512; do
        sips -z $s $s assets/icon.png --out "$SET/icon_${s}x${s}.png" >/dev/null
        d=$((s * 2))
        sips -z $d $d assets/icon.png --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
    done
    iconutil -c icns "$SET" -o assets/icon.icns
fi
cp assets/icon.icns "$APP/Contents/Resources/icon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Oxicord</string>
    <key>CFBundleDisplayName</key><string>Oxicord</string>
    <key>CFBundleIdentifier</key><string>dev.oxicord.app</string>
    <key>CFBundleExecutable</key><string>oxicord</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.social-networking</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>Oxicord uses the microphone for voice chat.</string>
    <key>NSCameraUsageDescription</key><string>Oxicord uses the camera for video chat.</string>
</dict>
</plist>
PLIST

codesign --force --sign - "$APP" >/dev/null 2>&1
echo "Built $APP"

if [ "${1:-}" = "--install" ]; then
    rm -rf /Applications/Oxicord.app
    cp -R "$APP" /Applications/Oxicord.app
    echo "Installed /Applications/Oxicord.app"
fi
