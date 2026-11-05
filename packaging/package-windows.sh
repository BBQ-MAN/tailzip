#!/bin/bash
# Cross-build tailzip for Windows x64 (mingw, from macOS) and make the NSIS installer.
# Needs: brew install mingw-w64 nsis
set -e
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
PK=$(cd "$(dirname "$0")" && pwd)
ROOT=$(dirname "$PK"); APP=$ROOT/app; DIST=$ROOT/dist
VER=$(grep -m1 '^version' $APP/Cargo.toml | cut -d'"' -f2)
T=x86_64-pc-windows-gnu
mkdir -p $DIST
[ -f "$PK/tailzip.ico" ] || bash "$PK/make-icons.sh"
rustup target add $T >/dev/null 2>&1 || true
cd $APP
cargo build --release --target $T 2>&1 | tail -1
EXE=$APP/target/$T/release/tailzip.exe
ls -la $EXE
cd $PK
SETUP="$DIST/tailzip-$VER-windows-x64-setup.exe"
makensis -V2 -DVERSION=$VER -DSRC=$(dirname $EXE) -DOUT="$SETUP" tailzip.nsi
# portable zip as well
Z=$(mktemp -d); mkdir -p $Z/tailzip; cp $EXE $ROOT/README.md $ROOT/LICENSE $ROOT/THIRD_PARTY_NOTICES.md $Z/tailzip/
(cd $Z && /usr/bin/zip -qr "$DIST/tailzip-$VER-windows-x64-portable.zip" tailzip); rm -rf $Z
ls -la "$SETUP" "$DIST/tailzip-$VER-windows-x64-portable.zip"
