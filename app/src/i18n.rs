// Interface language: English text is the key, STRINGS maps it to Korean.
use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Ko,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Ko];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Ko => "ko",
        }
    }

    /// Shown in its own language in the Language menu.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Ko => "한국어",
        }
    }

    pub fn parse(s: &str) -> Option<Lang> {
        let s = s.trim().to_lowercase();
        if s.starts_with("ko") {
            Some(Lang::Ko)
        } else if s.starts_with("en") {
            Some(Lang::En)
        } else {
            None
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn lang() -> Lang {
    if LANG.load(Ordering::Relaxed) == 1 {
        Lang::Ko
    } else {
        Lang::En
    }
}

pub fn set(l: Lang) {
    LANG.store(l as u8, Ordering::Relaxed);
}

/// TAILZIP_LANG, then the saved choice, then the system language, then English.
pub fn init() {
    let l = std::env::var("TAILZIP_LANG")
        .ok()
        .and_then(|v| Lang::parse(&v))
        .or_else(|| config_file().and_then(|p| read_lang(&p)))
        .or_else(|| {
            sys_locale::get_locale().map(|l| {
                if l.to_lowercase().starts_with("ko") {
                    Lang::Ko
                } else {
                    Lang::En
                }
            })
        })
        .unwrap_or(Lang::En);
    set(l);
}

/// Switch now and remember the choice; write errors are ignored.
pub fn choose(l: Lang) {
    set(l);
    if let Some(p) = config_file() {
        write_lang(&p, l);
    }
}

fn config_file() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(target_os = "macos") {
        env("HOME")?.join("Library/Application Support")
    } else if cfg!(windows) {
        env("APPDATA")?
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))?
    };
    Some(base.join("tailzip").join("lang"))
}

pub fn read_lang(p: &Path) -> Option<Lang> {
    Lang::parse(&fs::read_to_string(p).ok()?)
}

pub fn write_lang(p: &Path, l: Lang) {
    if let Some(d) = p.parent() {
        let _ = fs::create_dir_all(d);
    }
    let _ = fs::write(p, l.code());
}

fn lookup(l: Lang, key: &str) -> Option<&'static str> {
    let (en, ko) = STRINGS.iter().find(|(en, _)| *en == key)?;
    Some(if l == Lang::Ko { ko } else { en })
}

/// Translate `key` (the English text); unknown keys come back unchanged.
pub fn tr(key: &str) -> &str {
    lookup(lang(), key).unwrap_or(key)
}

/// Translate a template, then fill `{name}` placeholders.
pub fn trf(key: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut s = tr(key).to_string();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), &v.to_string());
    }
    s
}

const STRINGS: &[(&str, &str)] = &[
    ("tailzip", "꼬리집"),
    // Menus
    ("File", "파일"),
    ("Open Archive...", "압축 파일 열기..."),
    ("New Archive...", "새로 압축..."),
    ("Recent Files", "최근 파일"),
    ("(none)", "(없음)"),
    ("Close Archive", "압축 파일 닫기"),
    ("Exit", "끝내기"),
    ("Edit", "편집"),
    ("Add Files...", "파일 추가..."),
    ("Delete Files", "파일 삭제"),
    ("Select All", "모두 선택"),
    ("Find", "찾기"),
    ("Find Files...", "파일 찾기..."),
    ("Options", "옵션"),
    ("Settings...", "설정..."),
    ("View", "보기"),
    ("Toolbar", "도구 모음"),
    ("Status Bar", "상태 표시줄"),
    ("Language", "언어"),
    ("Tools", "도구"),
    ("Test Archive", "압축 파일 테스트"),
    ("Extract All...", "모두 압축 풀기..."),
    ("Help", "도움말"),
    ("About...", "꼬리집 정보..."),
    // Toolbar
    ("Open", "열기"),
    ("Extract", "압축 풀기"),
    ("New", "새로 압축"),
    ("Add", "추가"),
    ("Delete", "삭제"),
    ("Test", "테스트"),
    ("Find files", "파일 찾기"),
    // Start page
    ("Open Archive", "압축 파일 열기"),
    ("New Archive", "새로 압축"),
    ("No recent archives", "최근 압축 파일 없음"),
    ("Help Page", "도움말"),
    // File list
    ("Name", "이름"),
    ("Compressed", "압축된 크기"),
    ("Original", "원본 크기"),
    ("Type", "형식"),
    ("Modified", "수정한 날짜"),
    ("Method", "압축 방식"),
    ("CRC", "CRC"),
    ("File folder", "파일 폴더"),
    ("{ext} File", "{ext} 파일"),
    ("Store", "저장"),
    ("Stored", "저장"),
    ("Extract selected files...", "선택한 파일 압축 풀기..."),
    // Status bar
    (
        "{kind} - {files} file(s), {size}  ·  Compression ratio {ratio}%",
        "{kind} - 파일 {files}개, {size}  ·  압축률 {ratio}%",
    ),
    (
        "{files} file(s), {dirs} folder(s) selected  ·  Selected file size: {size}",
        "파일 {files}개, 폴더 {dirs}개 선택됨  ·  선택한 파일 크기: {size}",
    ),
    // Messages and dialogs
    (
        "Cannot open {path}:\n{e}",
        "{path} 파일을 열 수 없습니다.\n{e}",
    ),
    (
        "No errors found in this archive.\n{n} file(s) tested.",
        "압축 파일에 오류가 없습니다.\n파일 {n}개를 테스트했습니다.",
    ),
    (
        "Errors were found in this archive:\n{e}",
        "압축 파일에서 오류를 발견했습니다.\n{e}",
    ),
    ("OK", "확인"),
    (
        "Create ZIP, 7Z, TAR and TGZ archives; open and extract RAR, ZIPX, ALZ and EGG too.",
        "ZIP, 7Z, TAR, TGZ 압축 파일을 만들고, RAR, ZIPX, ALZ, EGG 압축 파일도 열어서 풉니다.",
    ),
    (
        "Runs on Windows and macOS.",
        "Windows와 macOS에서 실행됩니다.",
    ),
    (
        "Tip: drag files onto the window to compress them, or drop an archive to open it.",
        "팁: 파일을 창에 끌어다 놓으면 압축하고, 압축 파일을 놓으면 엽니다.",
    ),
    ("Settings", "설정"),
    (
        "Do you want to delete the selected file(s)?",
        "선택한 파일을 삭제하시겠습니까?",
    ),
    ("{n} item(s)", "항목 {n}개"),
    ("Yes", "예"),
    ("No", "아니요"),
    (
        "Extract {n} selected item(s) to:",
        "선택한 항목 {n}개를 풀 위치:",
    ),
    ("Extract all files to:", "모든 파일을 풀 위치:"),
    ("Browse...", "찾아보기..."),
    ("Cancel", "취소"),
    (
        "Extraction completed successfully.\n{n} file(s) extracted.",
        "압축을 풀었습니다.\n파일 {n}개를 풀었습니다.",
    ),
    (
        "Extraction has failed.\n({e})",
        "압축을 풀지 못했습니다.\n({e})",
    ),
    ("Add Files", "파일 추가"),
    ("Archive name:", "압축 파일 이름:"),
    ("Archive format:", "압축 형식:"),
    ("Compression level:", "압축 수준:"),
    ("Files to add ({n}):", "추가할 파일 ({n}개):"),
    (
        "Add files or folders, or drop them on the window.",
        "파일이나 폴더를 추가하거나 창에 끌어다 놓으세요.",
    ),
    ("Add files...", "파일 추가..."),
    ("Add folder...", "폴더 추가..."),
    ("Start", "시작"),
    ("Archiving completed successfully.", "압축을 완료했습니다."),
    ("0 - No compression", "0 - 압축 안 함"),
    ("1 - Fast compression", "1 - 빠른 압축"),
    ("2 - Normal compression", "2 - 보통 압축"),
    ("3 - Maximum compression", "3 - 최대 압축"),
    ("Archives", "압축 파일"),
    ("All files", "모든 파일"),
    (
        "Unsupported archive format.",
        "지원하지 않는 압축 파일 형식입니다.",
    ),
    // Passwords
    ("Enter Password", "암호 입력"),
    ("Password:", "암호:"),
    ("Confirm password:", "암호 확인:"),
    ("Show password", "암호 표시"),
    ("Set password", "암호 설정"),
    ("Encryption:", "암호화 방식:"),
    ("AES-256 (recommended)", "AES-256 (권장)"),
    ("ZipCrypto (compatible)", "ZipCrypto (호환용)"),
    ("Encrypt file names", "파일 이름 암호화"),
    ("Encrypted", "암호화됨"),
    ("The passwords do not match.", "암호가 일치하지 않습니다."),
    (
        "This archive is encrypted. Enter the password.",
        "암호가 걸린 압축 파일입니다. 암호를 입력하세요.",
    ),
    (
        "This archive is encrypted. A password is required.",
        "암호가 걸린 압축 파일입니다. 암호가 필요합니다.",
    ),
    ("The password is incorrect.", "암호가 틀렸습니다."),
    (
        "The password is incorrect. Try again.",
        "암호가 틀렸습니다. 다시 입력하세요.",
    ),
    (
        "Passwords and volumes are available for ZIP and 7Z.",
        "암호와 분할 압축은 ZIP과 7Z에서만 쓸 수 있습니다.",
    ),
    // Split volumes
    ("Split into volumes", "분할 압축"),
    ("Volume size:", "볼륨 크기:"),
    ("10 MB", "10 MB"),
    ("100 MB", "100 MB"),
    ("700 MB (CD)", "700 MB (CD)"),
    ("4 GB (FAT32)", "4 GB (FAT32)"),
    ("Custom", "직접 입력"),
    ("Split archive", "분할 압축 파일"),
    ("Volume {name} is missing.", "{name} 볼륨이 없습니다."),
    (
        "Volume {name} may be missing.",
        "{name} 볼륨이 없는 것 같습니다.",
    ),
    (
        "Split archives cannot be changed. Extract the files and create a new archive instead.",
        "분할 압축 파일은 고칠 수 없습니다. 압축을 푼 뒤 새로 압축하세요.",
    ),
    (
        "The split ZIP archive is damaged.",
        "분할 ZIP 압축 파일이 손상되었습니다.",
    ),
    (
        "Split ZIP64 archives are not supported.",
        "분할된 ZIP64 압축 파일은 지원하지 않습니다.",
    ),
    // Read-only formats and errors
    ("Read-only format", "읽기 전용 형식"),
    (
        "RAR, ALZ and EGG archives are read-only. Extract the files and create a new archive to change them.",
        "RAR, ALZ, EGG는 읽기 전용 형식입니다. 파일을 바꾸려면 압축을 푼 뒤 새로 압축하세요.",
    ),
    (
        "{name}: compression method {m} is not supported.",
        "{name}: {m} 압축 방식은 지원하지 않습니다.",
    ),
    ("The archive is incomplete.", "압축 파일이 중간에 잘렸습니다."),
    ("Cancelled.", "취소했습니다."),
    (
        "The operation failed unexpectedly.",
        "작업 중에 예기치 않은 오류가 발생했습니다.",
    ),
    ("{done} / {total} file(s)", "파일 {done} / {total}개"),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// String literals passed to tr()/trf() in the given source.
    fn used_keys(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        // Also catches a literal on the next line: trf(\n    "...".
        for pat in ["tr(", "trf("] {
            for (i, _) in src.match_indices(pat) {
                let Some(rest) = src[i + pat.len()..].trim_start().strip_prefix('"') else {
                    continue;
                };
                let end = rest.find('"').unwrap();
                out.push(rest[..end].replace("\\n", "\n"));
            }
        }
        out
    }

    #[test]
    fn every_key_translated() {
        for (i, (en, _)) in STRINGS.iter().enumerate() {
            assert!(
                STRINGS[..i].iter().all(|(k, _)| k != en),
                "duplicate key {en:?}"
            );
            for l in Lang::ALL {
                assert!(!lookup(l, en).unwrap().trim().is_empty(), "{l:?} {en:?}");
            }
            // Placeholders must survive translation.
            for ph in en
                .split('{')
                .skip(1)
                .filter_map(|s| s.split_once('}'))
                .map(|(p, _)| p)
            {
                assert!(
                    lookup(Lang::Ko, en).unwrap().contains(&format!("{{{ph}}}")),
                    "{en:?} lost {{{ph}}}"
                );
            }
        }
        let srcs = [include_str!("main.rs"), include_str!("archive.rs")];
        let keys: Vec<String> = srcs.iter().flat_map(|s| used_keys(s)).collect();
        assert!(keys.len() > 50, "scanner found only {} keys", keys.len());
        for k in keys {
            assert!(
                lookup(Lang::Ko, &k).is_some(),
                "missing translation for {k:?}"
            );
        }
    }

    #[test]
    fn lang_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tailzip-lang-{}", std::process::id()));
        let p = dir.join("sub/lang");
        assert_eq!(read_lang(&p), None);
        for l in Lang::ALL {
            write_lang(&p, l);
            assert_eq!(read_lang(&p), Some(l));
        }
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(Lang::parse(" KO_kr\n"), Some(Lang::Ko));
        assert_eq!(Lang::parse("fr"), None);
    }
}
