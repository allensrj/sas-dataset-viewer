#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "Building macOS Apple Metal release binary (backend-wgpu only)..."
cargo build --release --bin sas-viewer --no-default-features --features backend-wgpu

APP_NAME="SAS Dataset Viewer"
BUNDLE_DIR="$ROOT_DIR/dist/${APP_NAME}.app"
CONTENTS_DIR="$BUNDLE_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

rm -rf "$BUNDLE_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "$ROOT_DIR/target/release/sas-viewer" "$MACOS_DIR/sas-viewer"
chmod +x "$MACOS_DIR/sas-viewer"

cat > "$CONTENTS_DIR/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>zh_CN</string>
    <key>CFBundleExecutable</key>
    <string>sas-viewer</string>
    <key>CFBundleIdentifier</key>
    <string>com.sasviewer.app</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>SAS Dataset Viewer</string>
    <key>CFBundleDisplayName</key>
    <string>SAS 数据集查看器</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.3.1</string>
    <key>CFBundleVersion</key>
    <string>7</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key>
            <string>SAS Dataset</string>
            <key>CFBundleTypeRole</key>
            <string>Viewer</string>
            <key>CFBundleTypeExtensions</key>
            <array>
                <string>sas7bdat</string>
                <string>xpt</string>
            </array>
        </dict>
    </array>
</dict>
</plist>
EOF

# Ad-hoc sign the full .app bundle so Info.plist and resources are properly sealed on Apple Silicon
codesign --force --deep --sign - --identifier "com.sasviewer.app" "$BUNDLE_DIR"
codesign --verify --deep --strict "$BUNDLE_DIR"

echo "Created and ad-hoc signed macOS application bundle at: $BUNDLE_DIR"
