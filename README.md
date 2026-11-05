<p align="center">
  <img src="packaging/icon_win_1024.png" width="128" alt="tailzip icon">
</p>

<h1 align="center">tailzip · 꼬리집</h1>

<p align="center">
  Windows와 macOS에서 쓰는 가벼운 압축 프로그램 (Rust + egui)<br>
  A small archiver for Windows and macOS, written in Rust with egui
</p>

<p align="center">
  <img src="docs/screenshots/ko_start.png" width="45%" alt="시작 화면">
  <img src="docs/screenshots/ko_zip.png" width="45%" alt="압축 파일 목록">
</p>

[English](#english) · [한국어](#한국어)

---

## 한국어

### 지원 형식

| 형식 | 확장자 | 열기·풀기·테스트 | 새로 압축 | 추가·삭제 | 암호 | 분할 압축 |
|---|---|---|---|---|---|---|
| ZIP | `.zip` | ✅ | ✅ (Deflate) | ✅ | 열기: ZipCrypto, AES-128/192/256 · 만들기: AES-256(기본), ZipCrypto | 열기: `.zip.001`…, `.z01`+`.zip` · 만들기: `.zip.001`… |
| ZIPX | `.zipx` | ✅ (Deflate64, BZip2, LZMA, PPMd, XZ, Zstandard) | — | ✅ (기존 항목은 그대로 복사) | ZIP과 같음 | ZIP과 같음 |
| 7-Zip | `.7z` | ✅ | ✅ (LZMA2) | ✅ | AES-256, 파일 이름 암호화 포함 (열기·만들기) | 열기·만들기: `.7z.001`… |
| TAR | `.tar` | ✅ | ✅ | ✅ | — | 열기: `.tar.001`… |
| TAR + gzip | `.tgz`, `.tar.gz` | ✅ | ✅ | ✅ | — | — |
| RAR | `.rar` | ✅ RAR4, RAR5 | ❌ (읽기 전용) | ❌ | 열기 (파일 이름 암호화 포함) | 열기: `.part1.rar`…, `.rar`+`.r00`… |
| ALZ | `.alz` | ✅ 저장, Deflate, BZip2 | ❌ (읽기 전용) | ❌ | 열기: ZipCrypto | 열기: `.alz`+`.a00`, `.a01`… |
| EGG | `.egg` | ✅ 저장, Deflate, BZip2, LZMA, AZO, 솔리드 | ❌ (읽기 전용) | ❌ | 열기: ZipCrypto, AES-128/256, LEA-128/256 | 열기: `.vol1.egg`, `.vol2.egg`… |

- 형식은 파일 앞부분의 서명(ZIP `PK`, 7z, gzip, ustar, `Rar!`, `ALZ`, `EGGA`)으로 먼저 판별하고, 그다음 확장자로 판별합니다.
- 압축 수준은 4단계입니다: 0 무압축, 1 빠른 압축, 2 보통(기본), 3 최대 압축.
- **암호**: 암호가 필요하면 암호 입력 창이 뜹니다. 틀리면 다시 묻습니다. 입력한 암호는 그 압축 파일을 열어 둔 동안 메모리에만 두고, 저장하거나 기록하지 않습니다. 암호를 입력해 둔 ZIP에 파일을 추가하면 새 파일도 그 암호의 AES-256으로 암호화되고, 암호가 걸린 7z는 파일 이름 암호화 여부까지 그대로 유지합니다.
- **분할 압축**: 아무 볼륨이나 열어도 첫 볼륨부터 읽습니다. 볼륨이 빠져 있으면 어떤 파일이 없는지 알려 줍니다. 새로 압축할 때 **분할 압축**을 켜면 10 MB, 100 MB, 700 MB(CD), 4 GB(FAT32) 또는 직접 입력한 크기로 나눕니다. 이때는 7-Zip과 같은 방식으로 완성된 압축 파일을 잘라 `이름.7z.001`, `이름.zip.001`… 을 만듭니다(`.z01` 방식이 아님). 분할 압축 파일에는 파일을 추가하거나 삭제할 수 없습니다.
- **RAR는 읽기 전용입니다.** 공식 UnRAR 소스 코드로 목록 보기, 풀기, 테스트만 합니다. UnRAR 라이선스는 RAR 압축 파일을 만드는 데 쓰는 것을 허락하지 않으므로 RAR 만들기는 지원하지 않습니다. 라이선스 전문은 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)에 있습니다.
- **ALZ·EGG도 읽기 전용입니다.** [unalz](https://github.com/alkegi/unalz), [unegg](https://github.com/alkegi/unegg), [libazo](https://github.com/alkegi/libazo)(MIT)로 읽습니다. 이 라이브러리들은 CC0 형식 명세 [alkegi/docs](https://github.com/alkegi/docs)를 구현합니다. 한계: ALZ는 저장·Deflate·BZip2와 ZipCrypto만 지원합니다. EGG 솔리드 압축 파일은 파일 일부만 고르더라도 내부 스트림 전체를 한 번에 풉니다(진행률은 끝에 한꺼번에 오릅니다). EGG 분할 볼륨은 `이름.volN.egg` 이름일 때 중간 볼륨에서도 열 수 있습니다.
- **ZIPX**: WavPack, JPEG 등 그 밖의 압축 방식으로 저장된 파일은 해당 파일 이름과 함께 "지원하지 않는 압축 방식" 오류를 보여 줍니다.
- 압축 파일 안의 경로가 `..`나 절대 경로로 대상 폴더 밖을 가리키면 그 항목은 풀지 않습니다. TAR 안의 심볼릭 링크가 대상 폴더 밖을 가리켜도 풀지 않습니다.

### 주요 기능

- 시작 화면에서 **압축 파일 열기** / **새로 압축**
- 폴더 트리와 파일 목록(이름, 압축된 크기, 원본 크기, 형식, 수정한 날짜, 압축 방식, CRC)
- 도구 모음: 열기, 압축 풀기, 새로 압축, 추가, 삭제, 테스트. 읽기 전용 형식과 분할 압축 파일에서는 추가·삭제가 꺼지고, 마우스를 올리면 이유가 보입니다.
- 압축 풀기, 새로 압축, 테스트, 추가·삭제는 따로 돌아가며 진행률과 **취소** 버튼을 보여 줍니다.
- 상태 표시줄에 형식, 파일 수, 압축률, 그리고 암호화·분할·읽기 전용 여부를 보여 줍니다.
- 창에 파일을 끌어다 놓아 열기·추가
- 한국어 / 영어 화면. 처음에는 시스템 언어를 따르고, **보기 ▸ 언어** 메뉴에서 바꿀 수 있습니다.
- 압축 파일을 고칠 때는 새 파일을 다 만든 뒤에 원본과 바꿔서, 중간에 실패해도 원본이 망가지지 않습니다.

### 설치

[Releases](https://github.com/BBQ-MAN/tailzip/releases)에서 받습니다.

- **macOS** (Apple 실리콘·Intel 공용, macOS 11 이상): `tailzip-<버전>-mac-universal.dmg`를 열고 `tailzip`을 응용 프로그램 폴더로 끌어 넣습니다.
  - Apple 공증을 받지 않은 앱이라 처음 실행할 때 경고가 뜹니다. **우클릭 ▸ 열기**로 한 번 실행하면 그다음부터는 그냥 열립니다.
  - Finder에는 언어 설정에 따라 "꼬리집" 또는 "tailzip"으로 보입니다.
- **Windows** (64비트, Windows 10 이상)
  - 설치 파일: `tailzip-<버전>-windows-x64-setup.exe`. 시작 메뉴와 바탕 화면 바로 가기를 만들고, 지원 형식의 "연결 프로그램" 목록에 등록합니다. 기본 프로그램은 바꾸지 않습니다.
  - 설치 없이 쓰기: `tailzip-<버전>-windows-x64-portable.zip`을 풀고 `tailzip.exe`를 실행합니다.
  - 서명하지 않은 실행 파일이라 SmartScreen 경고가 뜰 수 있습니다. **추가 정보 ▸ 실행**을 누르면 됩니다.

### 직접 빌드

Rust stable이 필요합니다.

```bash
cd app
cargo run --release                        # 실행
cargo test --release                       # 테스트
TAILZIP_LANG=ko cargo run --release        # 한국어로 실행 (en: 영어)
```

- RAR 지원(UnRAR)은 C++ 코드라 C++ 컴파일러가 필요합니다(macOS: Xcode 명령줄 도구).
- 테스트용 압축 파일은 `app/tests/fixtures/`에 있습니다. `make.sh`가 7-Zip과 RAR로 만든 것이고, ALZ·EGG 샘플은 alkegi 저장소에서 가져온 것입니다(출처는 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)). 더 큰 샘플까지 테스트하려면 `bash app/tests/fetch-cache.sh`로 `app/tests/cache/`에 받아 둡니다(없으면 그 테스트는 건너뜁니다).
- macOS에서 Windows용으로 크로스 빌드할 때(`brew install mingw-w64`) `app/build.rs`가 UnRAR의 Windows 전용 소스(`isnt.cpp`)를 직접 컴파일하고, 정적 winpthread와 충돌하는 `-lpthread` 요청을 빈 라이브러리로 막습니다. UnRAR 소스를 cargo 레지스트리에서 찾지 못하면 `UNRAR_SRC=<unrar_sys>/vendor/unrar`로 알려 줍니다.

배포 파일 만들기(macOS에서):

```bash
bash packaging/make-icons.sh          # 아이콘(icns, ico) 다시 만들기
bash packaging/package-mac.sh         # dist/tailzip.app, dist/tailzip-<버전>-mac-universal.dmg
brew install mingw-w64 nsis
bash packaging/package-windows.sh     # dist/tailzip-<버전>-windows-x64-setup.exe, -portable.zip
```

### 폴더 구성

| 경로 | 내용 |
|---|---|
| `app/src/main.rs` | 화면(egui): 시작 화면, 도구 모음, 목록, 대화상자 |
| `app/src/archive.rs` | 형식 판별, 목록, 압축 풀기, 만들기, 추가, 삭제, 테스트, 암호, 분할 볼륨 |
| `app/src/archive_tests.rs` | 실제 압축 파일(`app/tests/fixtures/`)로 하는 테스트 |
| `app/src/i18n.rs` | 한국어·영어 문구와 언어 선택 |
| `packaging/` | 아이콘, macOS 번들, Windows 설치 파일(NSIS) 스크립트 |

### 라이선스

[MIT](LICENSE). 앱 아이콘은 Codex CLI(이미지 생성)로 만들었습니다. 함께 쓰는 UnRAR, unalz, unegg, libazo의 라이선스와 테스트 파일 출처는 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)에 있습니다.

---

## English

### Supported formats

| Format | Extensions | Open / extract / test | Create | Add / delete | Password | Split volumes |
|---|---|---|---|---|---|---|
| ZIP | `.zip` | ✅ | ✅ (Deflate) | ✅ | open: ZipCrypto, AES-128/192/256 · create: AES-256 (default), ZipCrypto | open: `.zip.001`…, `.z01`+`.zip` · create: `.zip.001`… |
| ZIPX | `.zipx` | ✅ (Deflate64, BZip2, LZMA, PPMd, XZ, Zstandard) | — | ✅ (existing entries copied as they are) | as ZIP | as ZIP |
| 7-Zip | `.7z` | ✅ | ✅ (LZMA2) | ✅ | AES-256, including encrypted file names (open and create) | open and create: `.7z.001`… |
| TAR | `.tar` | ✅ | ✅ | ✅ | — | open: `.tar.001`… |
| TAR + gzip | `.tgz`, `.tar.gz` | ✅ | ✅ | ✅ | — | — |
| RAR | `.rar` | ✅ RAR4, RAR5 | ❌ read-only | ❌ | open (including encrypted file names) | open: `.part1.rar`…, `.rar`+`.r00`… |
| ALZ | `.alz` | ✅ store, Deflate, BZip2 | ❌ read-only | ❌ | open: ZipCrypto | open: `.alz`+`.a00`, `.a01`… |
| EGG | `.egg` | ✅ store, Deflate, BZip2, LZMA, AZO, solid | ❌ read-only | ❌ | open: ZipCrypto, AES-128/256, LEA-128/256 | open: `.vol1.egg`, `.vol2.egg`… |

- The format is detected from the file signature first (ZIP `PK`, 7z, gzip, ustar, `Rar!`, `ALZ`, `EGGA`), then from the extension.
- Four compression levels: 0 store, 1 fast, 2 normal (default), 3 maximum.
- **Passwords**: a password prompt appears when one is needed and asks again after a wrong one. The password stays in memory only while that archive is open; it is never saved or logged. Files added to a ZIP whose password has been entered are encrypted with it (AES-256); an encrypted 7z keeps its encryption, file-name encryption included.
- **Split volumes**: open any volume and tailzip starts from the first; a missing volume is reported by name. **Split into volumes** in New Archive cuts the finished archive into raw volumes like 7-Zip does — `name.7z.001`, `name.zip.001`, … (not the `.z01` scheme) — with presets 10 MB, 100 MB, 700 MB (CD), 4 GB (FAT32) or a custom size. Split archives cannot be added to or deleted from.
- **RAR is read-only.** Listing, extraction and testing use the official UnRAR source code, whose licence does not allow using it to create RAR archives, so tailzip does not create them. The licence text is in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- **ALZ and EGG are read-only too**, read with [unalz](https://github.com/alkegi/unalz), [unegg](https://github.com/alkegi/unegg) and [libazo](https://github.com/alkegi/libazo) (MIT), which implement the CC0 format specification [alkegi/docs](https://github.com/alkegi/docs). Limits: ALZ supports store, Deflate, BZip2 and ZipCrypto only. Solid EGG archives are decoded as one stream even when only some files are selected (progress jumps at the end). EGG volumes can be opened from a middle volume when they are named `name.volN.egg`.
- **ZIPX**: entries using other methods (WavPack, JPEG, …) fail with an "unsupported compression method" error naming the file.
- Entries whose path leaves the destination folder (`..`, absolute paths) are not extracted, nor are TAR symlinks pointing outside it.

### Features

- Start page with **Open Archive** / **New Archive**
- Folder tree and file list (name, compressed and original size, type, modified, method, CRC)
- Toolbar: Open, Extract, New, Add, Delete, Test. Add and Delete are disabled for read-only formats and split archives, with a tooltip saying why.
- Extract, create, test, add and delete run in the background with a progress bar and **Cancel**.
- The status bar shows the format, file count, compression ratio and whether the archive is encrypted, split or read-only.
- Drag and drop files onto the window to open or add them
- Korean and English interface: follows the system language; switch with **View ▸ Language**
- Archives are rewritten to a temporary file and swapped in only when complete, so a failure never damages the original

### Install

Download from [Releases](https://github.com/BBQ-MAN/tailzip/releases).

- **macOS** (universal, macOS 11+): open `tailzip-<version>-mac-universal.dmg` and drag `tailzip` to Applications. The app is not notarized: the first time, right-click ▸ Open.
- **Windows** (x64, Windows 10+): run `tailzip-<version>-windows-x64-setup.exe`, or unzip `tailzip-<version>-windows-x64-portable.zip`. The installer adds Start menu and desktop shortcuts and an "Open with" entry for the supported types without changing your default app. The executable is unsigned, so SmartScreen may ask: More info ▸ Run anyway.

### Build

```bash
cd app
cargo run --release
cargo test --release
TAILZIP_LANG=en cargo run --release   # force English (ko: Korean)
```

- RAR support (UnRAR) is C++, so a C++ compiler is needed (macOS: Xcode command line tools).
- Test archives live in `app/tests/fixtures/`: `make.sh` made them with 7-Zip and RAR, and the ALZ/EGG samples come from the alkegi repositories (sources in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)). For the larger upstream samples run `bash app/tests/fetch-cache.sh` (fills `app/tests/cache/`; the test skips when it is absent).
- Cross-building for Windows from macOS (`brew install mingw-w64`): `app/build.rs` compiles UnRAR's Windows-only source (`isnt.cpp`), which its own build script skips on a non-Windows host, and satisfies UnRAR's dynamic `-lpthread` request with an empty library so it does not clash with the static winpthread Rust links. If the UnRAR sources are not found in the cargo registry, set `UNRAR_SRC=<unrar_sys>/vendor/unrar`.

Packaging (on macOS): `packaging/make-icons.sh`, `packaging/package-mac.sh`, and with `brew install mingw-w64 nsis`, `packaging/package-windows.sh`.

### License

[MIT](LICENSE). The app icon was generated with Codex CLI. Licences of UnRAR, unalz, unegg and libazo, and the sources of the test archives, are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
