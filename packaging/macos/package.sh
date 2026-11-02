#!/usr/bin/env bash
# Build a Dock-less TokenBar.app (LSUIElement) for the host Mac or a Rust target.
#
# Intel Macs (x86_64-apple-darwin):
#   rustup target add x86_64-apple-darwin
#   TARGET=x86_64-apple-darwin ./packaging/macos/package.sh
# On Apple Silicon the same command cross-compiles with the universal macOS SDK.
# GitHub's Intel-hosted macos-13 runners are retired, so Release does that cross
# build on macos-latest and uploads tokenbar-x86_64-apple-darwin.tar.gz.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "package.sh creates a macOS .app and must run on macOS" >&2
  exit 1
fi

VERSION="$(awk -F'"' '/^version = / { print $2; exit }' Cargo.toml)"
TARGET="${TARGET:-}"

if [[ -n "${TARGET}" ]]; then
  if command -v rustup >/dev/null 2>&1; then
    rustup target add "${TARGET}"
  fi
  cargo build -p tokenbar --release --target "${TARGET}"
  BIN="${ROOT}/target/${TARGET}/release/tokenbar"
  ARCH_NAME="${TARGET}"
else
  cargo build -p tokenbar --release
  BIN="${ROOT}/target/release/tokenbar"
  case "$(uname -m)" in
    arm64) ARCH_NAME="aarch64-apple-darwin" ;;
    x86_64) ARCH_NAME="x86_64-apple-darwin" ;;
    *) ARCH_NAME="$(uname -m)-apple-darwin" ;;
  esac
fi

if [[ ! -x "${BIN}" ]]; then
  echo "missing binary ${BIN}" >&2
  exit 1
fi

ICON_SRC="${ROOT}/assets/AppIcon.icns"
if [[ ! -f "${ICON_SRC}" ]]; then
  echo "missing ${ICON_SRC}; run packaging/icons/make-icons.py" >&2
  exit 1
fi

OUT="${ROOT}/dist/tokenbar-${ARCH_NAME}"
APP="${OUT}/TokenBar.app"
rm -rf "${OUT}"
mkdir -p "${APP}/Contents/MacOS" "${APP}/Contents/Resources"
cp "${BIN}" "${APP}/Contents/MacOS/tokenbar"
chmod 755 "${APP}/Contents/MacOS/tokenbar"
cp "${BIN}" "${OUT}/tokenbar"
# Finder and Launchpad use CFBundleIconFile → Contents/Resources/AppIcon.icns.
cp "${ICON_SRC}" "${APP}/Contents/Resources/AppIcon.icns"

cat > "${APP}/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>TokenBar</string>
  <key>CFBundleDisplayName</key>
  <string>TokenBar</string>
  <key>CFBundleIdentifier</key>
  <string>app.tokenbar.TokenBar</string>
  <key>CFBundleExecutable</key>
  <string>tokenbar</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleIconFile</key>
  <string>AppIcon</string>
  <key>LSMinimumSystemVersion</key>
  <string>12.0</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
EOF

echo "packed ${APP}"
echo "binary ${OUT}/tokenbar"
