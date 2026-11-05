#!/bin/bash
# Regenerates the tool-made fixtures in this folder (needs 7zz and rar; RAR is
# used only to create test files). ALZ/EGG/RAR4 samples are copied from
# upstream, see THIRD_PARTY_NOTICES.md. Password for every encrypted fixture: Test1234
set -euo pipefail
cd "$(dirname "$0")"
RAR=${RAR:-rar}
src=$(mktemp -d)
mkdir -p "$src/sub"
for i in $(seq 20); do echo "Hello from tailzip $i"; done > "$src/hello.txt"
for i in $(seq 10); do echo "암호 테스트 / password test $i"; done > "$src/sub/notes.txt"
# 3000 incompressible bytes so 1 KB volumes really split
head -c 3000 /dev/urandom > "$src/noise.bin"
pw=Test1234
rm -f ./*.zip ./*.zipx ./*.7z ./*.7z.0* ./*.zip.0* rar5*.rar
( cd "$src" && 7zz a -bso0 -tzip -p$pw -mem=AES256 "$OLDPWD/zip_aes256.zip" hello.txt sub
  7zz a -bso0 -tzip -p$pw -mem=AES128 "$OLDPWD/zip_aes128.zip" hello.txt sub
  7zz a -bso0 -tzip -p$pw -mem=AES192 "$OLDPWD/zip_aes192.zip" hello.txt sub
  7zz a -bso0 -tzip -p$pw -mem=ZipCrypto "$OLDPWD/zip_zipcrypto.zip" hello.txt sub
  for m in PPMd LZMA BZip2 Deflate64; do
    7zz a -bso0 -tzip -mm=$m "$OLDPWD/zipx_$(echo $m | tr A-Z a-z).zipx" hello.txt sub
  done
  7zz a -bso0 -t7z -p$pw "$OLDPWD/7z_aes.7z" hello.txt sub
  7zz a -bso0 -t7z -p$pw -mhe=on "$OLDPWD/7z_aes_hdr.7z" hello.txt sub
  7zz a -bso0 -t7z -mx0 -v1k "$OLDPWD/vol.7z" hello.txt noise.bin
  7zz a -bso0 -tzip -mx0 -v1k "$OLDPWD/vol.zip" hello.txt noise.bin
  "$RAR" a -idq -ma5 "$OLDPWD/rar5.rar" hello.txt sub
  "$RAR" a -idq -ma5 -p$pw "$OLDPWD/rar5_enc.rar" hello.txt sub
  "$RAR" a -idq -ma5 -hp$pw "$OLDPWD/rar5_hdr.rar" hello.txt sub
  "$RAR" a -idq -ma5 -m0 -v2k "$OLDPWD/rar5_vol.rar" hello.txt noise.bin )
rm -rf "$src"
ls -l
