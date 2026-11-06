#!/bin/sh
# Installs ~/Applications/HerdrBridge.app and, when Herdr is set up, its
# launcher mod. Run from a release folder or a source checkout. Re-running
# updates both; replaced copies are moved to the backups folder.
set -eu
HERE=$(CDPATH= cd "$(dirname "$0")" && pwd)
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

BIN="$HERE/herdr-zh-bridge"
if [ ! -x "$BIN" ]; then
  BIN="$HERE/target/release/herdr-zh-bridge"
  if [ -f "$HERE/Cargo.toml" ] && command -v cargo >/dev/null 2>&1; then
    (cd "$HERE" && cargo build --release)
  fi
fi
if [ ! -x "$BIN" ]; then
  echo "找不到 herdr-zh-bridge。请使用发行版压缩包，或安装 Rust 后在源码目录运行。" >&2
  exit 1
fi
# Validate the payload before touching any previous installation.
VERSION=$("$BIN" --version | awk '{print $2}')

DATA="${HERDR_BRIDGE_HOME:-$HOME/Library/Application Support/HerdrBridge}"
BACKUP="$DATA/backups/$(date +%Y%m%d-%H%M%S)"
APP="$HOME/Applications/HerdrBridge.app"
MARKER="$APP/Contents/Resources/herdr-bridge.json"
if [ -e "$APP" ] && [ ! -f "$MARKER" ]; then
  echo "已存在无关的 HerdrBridge.app，未做任何修改。" >&2
  exit 1
fi
if [ -e "$APP" ]; then
  mkdir -p "$BACKUP"
  mv "$APP" "$BACKUP/HerdrBridge.app"
fi
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/HerdrBridge"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Herdr Bridge</string>
  <key>CFBundleDisplayName</key><string>中文桥</string>
  <key>CFBundleIdentifier</key><string>local.herdrbridge.desktop</string>
  <key>CFBundleExecutable</key><string>HerdrBridge</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
printf '{"app":"HerdrBridge","version":"%s"}\n' "$VERSION" > "$MARKER"
codesign --force --sign - "$APP" >/dev/null 2>&1 || true
echo "已安装：$APP"

# The Herdr launcher mod, only after Herdr accepts it.
HERDR_HOME="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"
MOD="$HERDR_HOME/mods/zh-bridge"
if [ -f "$HERDR_HOME/AGENTS.md" ] && command -v herdr-app >/dev/null 2>&1; then
  if [ -e "$MOD" ] && ! grep -q 'zh-bridge' "$MOD/main.js" 2>/dev/null; then
    echo "mods/zh-bridge 不是本项目的入口，未修改。"
  elif herdr-app check "$HERE/mod"; then
    if [ -e "$MOD" ]; then
      mkdir -p "$BACKUP"
      mv "$MOD" "$BACKUP/zh-bridge"
    fi
    mkdir -p "$MOD"
    cp "$BIN" "$MOD/herdr-zh-bridge"
    cp "$HERE/mod/main.js" "$MOD/main.js"
    # Manifest last, so discovery only sees a complete mod.
    cp "$HERE/mod/mod.json" "$MOD/mod.json"
    echo "已安装 Herdr 启动入口：$MOD"
  else
    echo "Herdr 未通过入口检查，未安装 mod。"
  fi
fi
[ -d "$BACKUP" ] && echo "旧版本已备份到：$BACKUP"
open "$APP"
