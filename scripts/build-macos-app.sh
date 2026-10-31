#!/usr/bin/env bash
set -euo pipefail

# scripts/build-macos-app.sh
# Builds a standalone, double-clickable LocSamp.app bundle on macOS.

APP_NAME="LocSamp"
BIN_NAME="locsamp"
BUNDLE_DIR="dist/${APP_NAME}.app"
CONTENTS_DIR="${BUNDLE_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"

echo "==> Building ${BIN_NAME} in release mode..."
cargo build --release --bin "${BIN_NAME}"

echo "==> Creating macOS app bundle directory structure..."
rm -rf "${BUNDLE_DIR}"
mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}"

echo "==> Copying binary into bundle..."
if [[ -f "target/release/${BIN_NAME}" ]]; then
  cp "target/release/${BIN_NAME}" "${MACOS_DIR}/${BIN_NAME}"
elif [[ -f "target/aarch64-apple-darwin/release/${BIN_NAME}" ]]; then
  cp "target/aarch64-apple-darwin/release/${BIN_NAME}" "${MACOS_DIR}/${BIN_NAME}"
elif [[ -f "target/x86_64-apple-darwin/release/${BIN_NAME}" ]]; then
  cp "target/x86_64-apple-darwin/release/${BIN_NAME}" "${MACOS_DIR}/${BIN_NAME}"
else
  echo "Error: target/release/${BIN_NAME} was not found!" >&2
  exit 1
fi
chmod +x "${MACOS_DIR}/${BIN_NAME}"

echo "==> Generating Info.plist..."
cat << 'EOF' > "${CONTENTS_DIR}/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleName</key>
    <string>LocSamp</string>
    <key>CFBundleDisplayName</key>
    <string>LocSamp</string>
    <key>CFBundleIdentifier</key>
    <string>com.locsamp.app</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleExecutable</key>
    <string>locsamp</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
EOF

if [[ -f "assets/LocSamp.icns" ]]; then
  cp "assets/LocSamp.icns" "${RESOURCES_DIR}/LocSamp.icns"
fi

if command -v codesign &>/dev/null; then
  echo "==> Ad-hoc code signing bundle..."
  codesign --force --deep --sign - "${BUNDLE_DIR}" || true
fi

echo "=========================================================="
echo "✓ Success! Created ${BUNDLE_DIR}"
echo ""
echo "To install to your Applications folder, run:"
echo "  cp -r \"${BUNDLE_DIR}\" /Applications/"
echo "=========================================================="
