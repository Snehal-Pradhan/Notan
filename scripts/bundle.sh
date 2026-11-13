#!/bin/zsh
set -euo pipefail

cd "$(dirname "$0")/.."

cargo build --release

app="dist/Notan.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cp target/release/notan "$app/Contents/MacOS/notan"

# App icon from assets/logo-icon-1024.png
icon_src="assets/logo-icon-1024.png"
if [[ -f "$icon_src" ]]; then
  tmp="$(mktemp -d)"
  iconset="$tmp/AppIcon.iconset"
  mkdir -p "$iconset"
  sips -z 16 16     "$icon_src" --out "$iconset/icon_16x16.png"     >/dev/null
  sips -z 32 32     "$icon_src" --out "$iconset/icon_16x16@2x.png"  >/dev/null
  sips -z 32 32     "$icon_src" --out "$iconset/icon_32x32.png"     >/dev/null
  sips -z 64 64     "$icon_src" --out "$iconset/icon_32x32@2x.png"  >/dev/null
  sips -z 128 128   "$icon_src" --out "$iconset/icon_128x128.png"   >/dev/null
  sips -z 256 256   "$icon_src" --out "$iconset/icon_128x128@2x.png">/dev/null
  sips -z 256 256   "$icon_src" --out "$iconset/icon_256x256.png"   >/dev/null
  sips -z 512 512   "$icon_src" --out "$iconset/icon_256x256@2x.png">/dev/null
  sips -z 512 512   "$icon_src" --out "$iconset/icon_512x512.png"   >/dev/null
  cp "$icon_src" "$iconset/icon_512x512@2x.png"
  iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
  rm -rf "$tmp"
fi

cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Notan</string>
    <key>CFBundleDisplayName</key>
    <string>Notan</string>
    <key>CFBundleIdentifier</key>
    <string>dev.notan.app</string>
    <key>CFBundleExecutable</key>
    <string>notan</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.productivity</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
PLIST

if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$app" >/dev/null 2>&1 || true
fi

echo "built $app"
