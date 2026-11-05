#!/bin/bash
# Package tailzip (꼬리집) for macOS: universal (arm64 + x86_64) .app and .dmg.
# Ad-hoc signed, not notarized: on first launch use right-click > Open.
set -e
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
PK=$(cd "$(dirname "$0")" && pwd)
ROOT=$(dirname "$PK"); APP=$ROOT/app; DIST=$ROOT/dist
NAME=tailzip; BIN=tailzip; VER=$(grep -m1 '^version' $APP/Cargo.toml | cut -d'"' -f2)
mkdir -p $DIST
[ -f "$PK/AppIcon.icns" ] || bash "$PK/make-icons.sh"

# 1. universal binary
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true
cd $APP
cargo build --release --target aarch64-apple-darwin 2>&1 | tail -1
cargo build --release --target x86_64-apple-darwin 2>&1 | tail -1
OUT=$(mktemp -d)
lipo -create -output $OUT/$BIN target/aarch64-apple-darwin/release/$BIN target/x86_64-apple-darwin/release/$BIN
lipo -info $OUT/$BIN

# 2. bundle (English name "tailzip", Korean name "꼬리집" via ko.lproj)
B="$DIST/$NAME.app"; rm -rf "$B"
mkdir -p "$B/Contents/MacOS" "$B/Contents/Resources/en.lproj" "$B/Contents/Resources/ko.lproj"
cp $OUT/$BIN "$B/Contents/MacOS/$BIN"; rm -rf $OUT
cp $PK/AppIcon.icns "$B/Contents/Resources/AppIcon.icns"
sed "s/@VERSION@/$VER/g" $PK/Info.plist.in > "$B/Contents/Info.plist"
printf 'CFBundleName = "tailzip";\nCFBundleDisplayName = "tailzip";\n' > "$B/Contents/Resources/en.lproj/InfoPlist.strings"
printf 'CFBundleName = "꼬리집";\nCFBundleDisplayName = "꼬리집";\n' > "$B/Contents/Resources/ko.lproj/InfoPlist.strings"
plutil -lint "$B/Contents/Info.plist"
codesign --force --deep --sign - "$B"
codesign --verify --verbose=2 "$B" 2>&1 | tail -1

# 3. dmg with an /Applications shortcut
ST=$(mktemp -d)
cp -R "$B" $ST/; ln -s /Applications $ST/Applications
DMG="$DIST/$NAME-$VER-mac-universal.dmg"; rm -f "$DMG"
hdiutil create -volname "$NAME $VER" -srcfolder $ST -ov -format UDZO "$DMG" >/dev/null 2>&1
rm -rf $ST
du -sh "$B" "$DMG"
