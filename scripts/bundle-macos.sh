#!/bin/sh
# Build Oxicord.app.
#   scripts/bundle-macos.sh            build for this Mac
#   scripts/bundle-macos.sh --install  ...and copy it to /Applications
#   scripts/bundle-macos.sh --dist     build the download: both kinds of Mac in one app, the project's own icon,
#                                      zipped to target/dist/Oxicord-Mac.zip
set -eu
cd "$(dirname "$0")/.."

MODE="${1:-}"
if [ "$MODE" = "--dist" ]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cargo build --release --target aarch64-apple-darwin --target x86_64-apple-darwin
    OUT=target/dist
    rm -rf "$OUT" && mkdir -p "$OUT"
    lipo -create -output "$OUT/oxicord" \
        target/aarch64-apple-darwin/release/oxicord target/x86_64-apple-darwin/release/oxicord
else
    cargo build --release
    OUT=target/release
fi

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP="$OUT/Oxicord.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$OUT/oxicord" "$APP/Contents/MacOS/oxicord"

# assets/icon.local.png (untracked) overrides the project icon on this machine.
[ -f assets/icon.png ] || python3 scripts/make-icon.py assets/icon.png
ICON=assets/icon.png
[ -f assets/icon.local.png ] && [ "$MODE" != "--dist" ] && ICON=assets/icon.local.png
SET=target/icon.iconset
rm -rf "$SET" && mkdir -p "$SET"
for s in 16 32 128 256 512; do
    sips -z $s $s "$ICON" --out "$SET/icon_${s}x${s}.png" >/dev/null
    d=$((s * 2))
    sips -z $d $d "$ICON" --out "$SET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o "$APP/Contents/Resources/icon.icns"

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

if [ "$MODE" = "--dist" ]; then
    ditto -c -k --norsrc --noextattr --keepParent "$APP" "$OUT/Oxicord-Mac.zip"
    echo "Packed $OUT/Oxicord-Mac.zip"
fi

if [ "$MODE" = "--install" ]; then
    rm -rf /Applications/Oxicord.app
    cp -R "$APP" /Applications/Oxicord.app
    echo "Installed /Applications/Oxicord.app"
fi
