#!/bin/bash
# Build SmartRename.app for macOS (Apple Silicon / Intel universal host arch).
# Usage: bash packaging/macos/build-app.sh
set -euo pipefail

cd "$(dirname "$0")/../.."

APP_NAME="SmartReName"
BIN_NAME="SmartRename"

echo "== cargo build --release =="
cargo build --release -p srename-app

APP="${APP_NAME}.app"
echo "== assembling ${APP} =="
rm -rf "${APP}"
mkdir -p "${APP}/Contents/MacOS" "${APP}/Contents/Resources"
cp "target/release/${BIN_NAME}" "${APP}/Contents/MacOS/${BIN_NAME}"
# App icon: prefer the Icon Composer asset (same approach as PureMap).
# actool compiles packaging/macos/Icon.icon into Assets.car + Icon.icns,
# which current macOS renders natively (proper squircle, no fallback
# plate). Without actool/Xcode, fall back to the classic .icns under
# the same name so the build always ships an icon.
ICON_SRC="packaging/macos/Icon.icon"
ACTOOL=""
if command -v xcrun >/dev/null 2>&1; then
    ACTOOL="$(xcrun --find actool 2>/dev/null || true)"
fi
if [ -n "${ACTOOL}" ] && [ -d "${ICON_SRC}" ]; then
    if "${ACTOOL}" "${ICON_SRC}" \
        --compile "${APP}/Contents/Resources" \
        --app-icon Icon \
        --enable-on-demand-resources NO \
        --development-region en \
        --target-device mac \
        --minimum-deployment-target 26.0 \
        --platform macosx \
        --output-partial-info-plist "${TMPDIR:-/tmp}/smartrename-icon-partial.plist" \
        > "icon-build.log" 2>&1; then
        echo "== icon: Icon Composer (actool) =="
    else
        echo "== icon: actool failed, falling back to classic icns (see icon-build.log) =="
        rm -f "${APP}/Contents/Resources/Assets.car"
        cp "packaging/macos/AppIcon.icns" "${APP}/Contents/Resources/Icon.icns"
    fi
else
    cp "packaging/macos/AppIcon.icns" "${APP}/Contents/Resources/Icon.icns"
fi

# Single source of truth for the version: the app crate's Cargo.toml.
VERSION="$(grep -m1 '^version = ' crates/srename-app/Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"
echo "== version ${VERSION} =="

cat > "${APP}/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>SmartRename</string>
    <key>CFBundleIdentifier</key>
    <string>net.mrben.smartrename</string>
    <key>CFBundleName</key>
    <string>SmartReName</string>
    <key>CFBundleDisplayName</key>
    <string>SmartReName</string>
    <key>CFBundleIconFile</key>
    <string>Icon</string>
    <key>CFBundleIconName</key>
    <string>Icon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>__VERSION__</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

# Fill in the version read from Cargo.toml above.
sed -i '' "s/__VERSION__/${VERSION}/" "${APP}/Contents/Info.plist" 2>/dev/null \
    || sed -i "s/__VERSION__/${VERSION}/" "${APP}/Contents/Info.plist"

# Ad-hoc signature so Gatekeeper lets it run locally (same approach as PureMap).
codesign --force --deep --sign - "${APP}" || echo "(codesign skipped)"

LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [ -x "${LSREGISTER}" ]; then
    "${LSREGISTER}" -f "${APP}" || true
fi

echo "== done: ${APP} =="
