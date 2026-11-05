// Archive engine: list, extract, test for ZIP/ZIPX, 7Z, TAR, TAR.GZ, RAR, ALZ and EGG;
// create, add to and delete from ZIP, 7Z, TAR and TAR.GZ. Passwords and split volumes.
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::i18n::{tr, trf};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use sevenz_rust2::{
    encoder_options::{AesEncoderOptions, Lzma2Options},
    ArchiveEntry, ArchiveReader, ArchiveWriter, EncoderConfiguration, EncoderMethod, Password,
};
use zip::{
    result::ZipError, unstable::write::FileOptionsExt, write::FileOptions, AesMode,
    CompressionMethod, ZipArchive, ZipWriter,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The archive or entry is encrypted and no password was given.
    NeedPassword,
    WrongPassword,
    Cancelled,
    Msg(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::NeedPassword => tr("This archive is encrypted. A password is required."),
            Error::WrongPassword => tr("The password is incorrect."),
            Error::Cancelled => tr("Cancelled."),
            Error::Msg(m) => m,
        })
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Msg(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Msg(s.into())
    }
}

impl From<Error> for String {
    fn from(e: Error) -> Self {
        e.to_string()
    }
}

pub type Res<T> = Result<T, Error>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Prefix a plain error with the entry name; password errors stay as they are.
fn named(name: &str, e: Error) -> Error {
    match e {
        Error::Msg(m) => Error::Msg(format!("{name}: {m}")),
        e => e,
    }
}

// ---------- progress / cancel (one operation at a time) ----------

pub struct Progress {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
}

pub static PROGRESS: Progress = Progress {
    done: AtomicUsize::new(0),
    total: AtomicUsize::new(0),
    cancel: AtomicBool::new(false),
};

impl Progress {
    pub fn reset(&self, total: usize) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        self.cancel.store(false, Ordering::Relaxed);
    }
}

/// One entry done; fails once the user pressed Cancel.
fn tick() -> Res<()> {
    if PROGRESS.cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    PROGRESS.done.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

// ---------- formats ----------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Zip,
    SevenZ,
    Tar,
    Tgz,
    Rar,
    Alz,
    Egg,
}

impl Kind {
    /// Formats tailzip can create and change.
    pub const ALL: [Kind; 4] = [Kind::Zip, Kind::SevenZ, Kind::Tar, Kind::Tgz];

    pub fn ext(self) -> &'static str {
        match self {
            Kind::Zip => "zip",
            Kind::SevenZ => "7z",
            Kind::Tar => "tar",
            Kind::Tgz => "tgz",
            Kind::Rar => "rar",
            Kind::Alz => "alz",
            Kind::Egg => "egg",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Zip => "ZIP",
            Kind::SevenZ => "7Z",
            Kind::Tar => "TAR",
            Kind::Tgz => "TGZ (tar.gz)",
            Kind::Rar => "RAR",
            Kind::Alz => "ALZ",
            Kind::Egg => "EGG",
        }
    }

    pub fn read_only(self) -> bool {
        matches!(self, Kind::Rar | Kind::Alz | Kind::Egg)
    }

    /// Detect by file signature first, then by extension. Any volume of a split set may be given.
    pub fn detect(path: &Path) -> Option<Kind> {
        let first = first_volume(path);
        let mut head = [0u8; 512];
        let n = File::open(&first)
            .and_then(|mut f| f.read(&mut head))
            .unwrap_or(0);
        let h = &head[..n];
        if h.starts_with(b"PK") && n >= 4 {
            return Some(Kind::Zip);
        }
        if h.starts_with(&[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C]) {
            return Some(Kind::SevenZ);
        }
        if h.starts_with(&[0x1f, 0x8b]) {
            return Some(Kind::Tgz);
        }
        if n >= 262 && &h[257..262] == b"ustar" {
            return Some(Kind::Tar);
        }
        if h.starts_with(b"Rar!\x1a\x07\x00") || h.starts_with(b"Rar!\x1a\x07\x01\x00") {
            return Some(Kind::Rar);
        }
        if h.starts_with(b"ALZ\x01") {
            return Some(Kind::Alz);
        }
        if h.starts_with(b"EGGA") {
            return Some(Kind::Egg);
        }
        let name = first.file_name()?.to_string_lossy().to_lowercase();
        let ext = name.rsplit('.').next().unwrap_or("");
        match ext {
            "zip" | "zipx" => Some(Kind::Zip),
            "7z" => Some(Kind::SevenZ),
            "tar" => Some(Kind::Tar),
            "tgz" => Some(Kind::Tgz),
            "gz" if name.ends_with(".tar.gz") => Some(Kind::Tgz),
            "rar" => Some(Kind::Rar),
            "alz" => Some(Kind::Alz),
            "egg" => Some(Kind::Egg),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Path inside the archive, '/' separated, no trailing slash.
    pub path: String,
    pub is_dir: bool,
    /// 0 when the format does not report it (RAR).
    pub packed: u64,
    pub size: u64,
    pub modified: String,
    pub method: String,
    pub crc: Option<u32>,
    pub encrypted: bool,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
    pub fn parent(&self) -> &str {
        self.path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
    }
}

/// Options for a new archive.
#[derive(Clone, Debug, Default)]
pub struct NewOpts {
    pub password: Option<String>,
    /// ZIP only: legacy ZipCrypto instead of AES-256.
    pub zip_crypto: bool,
    /// 7z only: also encrypt the file list.
    pub encrypt_names: bool,
    /// Split into raw volumes (name.7z.001, ...) of this many bytes.
    pub volume_size: Option<u64>,
}

/// Compression level as shown in the New Archive window: 0 store, 1 fast, 2 normal, 3 maximum.
pub fn deflate_level(level: u8) -> u32 {
    match level {
        0 => 0,
        1 => 1,
        2 => 6,
        _ => 9,
    }
}

// ---------- dates ----------

/// Unix seconds to (y, m, d, h, mi, s) in UTC.
pub fn civil(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        (rem % 3600 / 60) as u32,
        (rem % 60) as u32,
    )
}

fn fmt_unix(secs: i64) -> String {
    let (y, m, d, h, mi, _) = civil(secs);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}")
}

/// MS-DOS packed date/time (RAR, ALZ).
fn fmt_dos(t: u32) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        (t >> 25) + 1980,
        (t >> 21) & 15,
        (t >> 16) & 31,
        (t >> 11) & 31,
        (t >> 5) & 63
    )
}

fn mtime_secs(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn zip_time(path: &Path) -> zip::DateTime {
    // ponytail: UTC, not local time; add a timezone crate if local stamps matter.
    let (y, m, d, h, mi, s) = civil(mtime_secs(path));
    zip::DateTime::from_date_and_time(
        y.clamp(1980, 2107) as u16,
        m as u8,
        d as u8,
        h as u8,
        mi as u8,
        s as u8,
    )
    .unwrap_or_default()
}

// ---------- split volumes ----------

/// Most volumes we follow; stops runaway sets.
const MAX_VOLUMES: usize = 10_000;

type Namer = Box<dyn Fn(usize) -> PathBuf>;

/// Naming scheme of a split set: volume i -> path, plus a trailing part (the .zip of a split ZIP).
/// None for a plain archive.
fn scheme(path: &Path) -> Option<(Namer, Option<PathBuf>)> {
    let name = path.file_name()?.to_str()?.to_string();
    let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
    let (stem, ext) = name.rsplit_once('.')?;
    let (stem, ext, ext_l) = (stem.to_string(), ext.to_string(), ext.to_lowercase());
    let stem_l = stem.to_lowercase();
    let up = ext.chars().any(|c| c.is_ascii_uppercase());
    let case = move |s: &str| if up { s.to_uppercase() } else { s.to_string() };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let num_ext = |s: &str, c: char| s.len() == 3 && s.starts_with(c) && digits(&s[1..]);
    let sib = move |n: String| dir.join(n);
    // name.7z.001, name.zip.001, ...: raw volumes.
    if ext.len() >= 3 && digits(&ext) {
        let w = ext.len();
        return Some((
            Box::new(move |i| sib(format!("{stem}.{:0w$}", i + 1))),
            None,
        ));
    }
    // Split ZIP: name.z01, name.z02, ..., name.zip
    if num_ext(&ext_l, 'z') || (ext_l == "zip" && sib(format!("{stem}.{}", case("z01"))).is_file())
    {
        let tail = sib(format!("{stem}.{}", case("zip")));
        return Some((
            Box::new(move |i| sib(format!("{stem}.{}", case(&format!("z{:02}", i + 1))))),
            Some(tail),
        ));
    }
    // RAR: name.part1.rar / name.part01.rar ...
    if ext_l == "rar" {
        if let Some(p) = stem_l.rfind(".part") {
            if digits(&stem[p + 5..]) {
                let (base, w, part) = (
                    stem[..p].to_string(),
                    stem.len() - p - 5,
                    stem[p + 1..p + 5].to_string(),
                );
                return Some((
                    Box::new(move |i| sib(format!("{base}.{part}{:0w$}.{ext}", i + 1))),
                    None,
                ));
            }
        }
    }
    // Old RAR naming name.rar, name.r00, ...; ALZ name.alz, name.a00, ..., name.b00, ...
    for (first, c) in [("rar", 'r'), ("alz", 'a')] {
        let mid = num_ext(&ext_l, c) || (c == 'a' && num_ext(&ext_l, 'b'));
        if mid || (ext_l == first && sib(format!("{stem}.{}", case(&format!("{c}00")))).is_file()) {
            return Some((
                Box::new(move |i| {
                    sib(if i == 0 {
                        format!("{stem}.{}", case(first))
                    } else {
                        let letter = (c as u8 + ((i - 1) / 100).min(25) as u8) as char;
                        format!("{stem}.{}", case(&format!("{letter}{:02}", (i - 1) % 100)))
                    })
                }),
                None,
            ));
        }
    }
    // EGG: name.vol1.egg, name.vol2.egg, ...
    if ext_l == "egg" {
        if let Some(p) = stem_l.rfind(".vol") {
            if digits(&stem[p + 4..]) {
                let base = stem[..p + 4].to_string();
                return Some((
                    Box::new(move |i| sib(format!("{base}{}.{ext}", i + 1))),
                    None,
                ));
            }
        }
    }
    None
}

fn missing(p: &Path) -> Error {
    Error::Msg(trf(
        "Volume {name} is missing.",
        &[("name", &p.file_name().unwrap_or_default().to_string_lossy())],
    ))
}

/// Volumes of a split archive in data order (any volume may be given); a plain archive gives itself.
pub fn volumes(path: &Path) -> Res<Vec<PathBuf>> {
    let Some((nth, tail)) = scheme(path) else {
        return Ok(vec![path.to_path_buf()]);
    };
    let mut v = Vec::new();
    while v.len() < MAX_VOLUMES {
        let p = nth(v.len());
        if !p.is_file() {
            break;
        }
        v.push(p);
    }
    if v.is_empty() {
        return Err(missing(&nth(0)));
    }
    if let Some(t) = tail {
        if !t.is_file() {
            return Err(missing(&t));
        }
        v.push(t);
    }
    Ok(v)
}

pub fn is_split(path: &Path) -> bool {
    scheme(path).is_some()
}

/// The volume to open / detect: the first one of a split set.
pub fn first_volume(path: &Path) -> PathBuf {
    scheme(path).map_or_else(|| path.to_path_buf(), |(nth, _)| nth(0))
}

/// First volume after the existing ones, for "missing volume" messages.
fn next_volume(path: &Path) -> Option<PathBuf> {
    let (nth, tail) = scheme(path)?;
    if tail.is_some() {
        return None;
    }
    (0..MAX_VOLUMES).map(nth).find(|p| !p.is_file())
}

/// A failed open of a split set probably means a volume is missing; say which one.
fn split_hint(path: &Path, e: Error) -> Error {
    match (e, next_volume(path)) {
        (Error::Msg(m), Some(n)) => Error::Msg(format!(
            "{m}\n{}",
            trf(
                "Volume {name} may be missing.",
                &[("name", &n.file_name().unwrap_or_default().to_string_lossy())]
            )
        )),
        (e, _) => e,
    }
}

/// Folder name for "Extract": the archive name without volume and format suffixes.
pub fn base_name(path: &Path) -> String {
    let first = first_volume(path);
    let mut s = first
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let known = ["zip", "zipx", "7z", "tar", "gz", "tgz", "rar", "alz", "egg"];
    let num = |n: &str| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit());
    loop {
        let lower = s.to_lowercase();
        let Some((stem, ext)) = lower.rsplit_once('.') else {
            break;
        };
        let vol = (ext.len() >= 3 && num(ext))
            || ext.strip_prefix("part").is_some_and(num)
            || ext.strip_prefix("vol").is_some_and(num);
        if stem.is_empty() || !(known.contains(&ext) || vol) {
            break;
        }
        s.truncate(stem.len());
    }
    s
}

pub trait Src: Read + Seek + Send {}
impl<T: Read + Seek + Send> Src for T {}

/// Several files (or memory blocks) read as one stream.
struct Concat {
    parts: Vec<(Box<dyn Src>, u64)>,
    pos: u64,
}

impl Concat {
    fn total(&self) -> u64 {
        self.parts.iter().map(|p| p.1).sum()
    }
}

impl Read for Concat {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut start = 0u64;
        for (r, len) in &mut self.parts {
            if self.pos < start + *len {
                let off = self.pos - start;
                r.seek(SeekFrom::Start(off))?;
                let n = (*len - off).min(buf.len() as u64) as usize;
                let n = r.read(&mut buf[..n])?;
                self.pos += n as u64;
                return Ok(n);
            }
            start += *len;
        }
        Ok(0)
    }
}

impl Seek for Concat {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let to = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
            SeekFrom::End(d) => self.total().checked_add_signed(d),
        };
        self.pos = to.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad seek"))?;
        Ok(self.pos)
    }
}

fn open_files(parts: &[PathBuf]) -> Res<Vec<(Box<dyn Src>, u64)>> {
    parts
        .iter()
        .map(|p| {
            let f = File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let len = f.metadata().map_err(err)?.len();
            Ok((Box::new(f) as Box<dyn Src>, len))
        })
        .collect()
}

/// The archive bytes, joining split volumes. Split ZIPs get a rewritten central directory
/// whose offsets point into the joined stream.
fn source(path: &Path) -> Res<Box<dyn Src>> {
    let parts = volumes(path)?;
    if parts.len() == 1 {
        return Ok(Box::new(BufReader::new(
            File::open(&parts[0]).map_err(err)?,
        )));
    }
    let joined = Concat {
        parts: open_files(&parts)?,
        pos: 0,
    };
    let zip_split = matches!(scheme(path), Some((_, Some(_))));
    Ok(if zip_split {
        Box::new(BufReader::new(split_zip(joined, &parts)?))
    } else {
        Box::new(BufReader::new(joined))
    })
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2)
        .map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4)
        .map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// Split ZIP (.z01 ... .zip): offsets in the central directory are per disk. Present the joined
/// volumes with a patched central directory that uses offsets into the whole stream.
fn split_zip(mut joined: Concat, parts: &[PathBuf]) -> Res<Concat> {
    let bad = || Error::Msg(tr("The split ZIP archive is damaged.").into());
    let zip64 = || Error::Msg(tr("Split ZIP64 archives are not supported.").into());
    let starts: Vec<u64> = joined
        .parts
        .iter()
        .scan(0u64, |s, p| {
            let at = *s;
            *s += p.1;
            Some(at)
        })
        .collect();
    let total = joined.total();
    // End of central directory: in the last 64 KB + 22 bytes.
    let tail_len = total.min(65_557);
    let mut tail = vec![0u8; tail_len as usize];
    joined
        .seek(SeekFrom::Start(total - tail_len))
        .map_err(err)?;
    joined.read_exact(&mut tail).map_err(err)?;
    let at = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| u32_at(&tail, i) == 0x0605_4b50)
        .ok_or_else(bad)?;
    let mut eocd = tail[at..].to_vec();
    let disk = u16_at(&eocd, 4) as usize;
    let cd_disk = u16_at(&eocd, 6) as usize;
    let n_total = u16_at(&eocd, 10);
    let cd_size = u32_at(&eocd, 12) as u64;
    let cd_off = u32_at(&eocd, 16) as u64;
    if disk + 1 > parts.len() {
        // name.z01 .. name.zNN are taken while they exist, so the next number is the missing one.
        let last = &parts[parts.len() - 1];
        return Err(missing(
            &last.with_extension(format!("z{:02}", parts.len())),
        ));
    }
    if cd_off == 0xFFFF_FFFF || n_total == 0xFFFF {
        // shortcut: ZIP64 split archives are refused; patch the ZIP64 records if users need them.
        return Err(zip64());
    }
    let cd_abs = starts.get(cd_disk).ok_or_else(bad)? + cd_off;
    if cd_abs.checked_add(cd_size).is_none_or(|e| e > total) {
        return Err(bad());
    }
    let mut cd = vec![0u8; cd_size as usize];
    joined.seek(SeekFrom::Start(cd_abs)).map_err(err)?;
    joined.read_exact(&mut cd).map_err(err)?;
    let mut i = 0;
    while i + 46 <= cd.len() && u32_at(&cd, i) == 0x0201_4b50 {
        let (n, m, k) = (
            u16_at(&cd, i + 28),
            u16_at(&cd, i + 30),
            u16_at(&cd, i + 32),
        );
        let d = u16_at(&cd, i + 34) as usize;
        let off = u32_at(&cd, i + 42) as u64;
        let abs = u32::try_from(starts.get(d).ok_or_else(bad)? + off).map_err(|_| zip64())?;
        cd[i + 34..i + 36].copy_from_slice(&0u16.to_le_bytes());
        cd[i + 42..i + 46].copy_from_slice(&abs.to_le_bytes());
        i += 46 + n as usize + m as usize + k as usize;
    }
    let cd_abs32 = u32::try_from(cd_abs).map_err(|_| zip64())?;
    eocd[4..8].fill(0);
    eocd[8..10].copy_from_slice(&n_total.to_le_bytes());
    eocd[16..20].copy_from_slice(&cd_abs32.to_le_bytes());
    // Keep the volume bytes before the central directory, then the patched records.
    let mut out = Vec::new();
    for ((r, len), start) in joined.parts.into_iter().zip(starts) {
        if start < cd_abs {
            out.push((r, len.min(cd_abs - start)));
        }
    }
    cd.extend_from_slice(&eocd);
    let len = cd.len() as u64;
    out.push((Box::new(io::Cursor::new(cd)) as Box<dyn Src>, len));
    Ok(Concat { parts: out, pos: 0 })
}

// ---------- list ----------

pub fn list(path: &Path, pw: Option<&str>) -> Res<(Kind, Vec<Entry>)> {
    let kind = Kind::detect(path).ok_or(tr("Unsupported archive format."))?;
    let mut entries = match kind {
        Kind::Zip => list_zip(path)?,
        Kind::SevenZ => list_7z(path, pw)?,
        Kind::Tar => list_tar(tar::Archive::new(source(path)?), "Stored")?,
        Kind::Tgz => list_tar(tar::Archive::new(GzDecoder::new(source(path)?)), "GZip")?,
        Kind::Rar => list_rar(path, pw)?,
        Kind::Alz => list_alz(path)?,
        Kind::Egg => list_egg(path)?,
    };
    add_missing_dirs(&mut entries);
    Ok((kind, entries))
}

fn zip_open(path: &Path) -> Res<ZipArchive<Box<dyn Src>>> {
    ZipArchive::new(source(path)?).map_err(|e| {
        let m = err(e);
        // The last part of a split ZIP opened without its .z01, .z02, ...
        if m.to_lowercase().contains("multi-disk") && !is_split(path) {
            missing(&path.with_extension("z01"))
        } else {
            split_hint(path, m.into())
        }
    })
}

fn list_zip(path: &Path) -> Res<Vec<Entry>> {
    let mut za = zip_open(path)?;
    let mut out = Vec::with_capacity(za.len().min(1 << 16));
    for i in 0..za.len() {
        let f = za.by_index_raw(i).map_err(err)?;
        let modified = f
            .last_modified()
            .map(|t| {
                format!(
                    "{:04}-{:02}-{:02} {:02}:{:02}",
                    t.year(),
                    t.month(),
                    t.day(),
                    t.hour(),
                    t.minute()
                )
            })
            .unwrap_or_default();
        out.push(Entry {
            path: zname(&f),
            is_dir: f.is_dir(),
            packed: f.compressed_size(),
            size: f.size(),
            modified,
            method: method_name(f.compression()),
            crc: (!f.is_dir()).then(|| f.crc32()),
            encrypted: f.encrypted(),
        });
    }
    Ok(out)
}

/// Entry name, '/' separated, no trailing slash; undecodable names fall back to lossy UTF-8.
fn zname<R: Read>(f: &zip::read::ZipFile<'_, R>) -> String {
    let n = match f.name() {
        Ok(n) => n.into_owned(),
        Err(_) => String::from_utf8_lossy(f.name_raw()).into_owned(),
    };
    norm(&n)
}

fn norm(n: &str) -> String {
    n.replace('\\', "/").trim_end_matches('/').to_string()
}

fn method_name(m: CompressionMethod) -> String {
    match m {
        CompressionMethod::Stored => "Store".into(),
        CompressionMethod::Deflated => "Deflate".into(),
        other => format!("{other:?}"),
    }
}

fn list_tar<R: Read>(mut ar: tar::Archive<R>, method: &str) -> Res<Vec<Entry>> {
    let mut out = Vec::new();
    for e in ar.entries().map_err(err)? {
        let e = e.map_err(err)?;
        let h = e.header();
        let p = e.path().map_err(err)?.to_string_lossy().replace('\\', "/");
        let p = p.trim_start_matches("./").trim_end_matches('/').to_string();
        if p.is_empty() {
            continue;
        }
        let size = h.size().unwrap_or(0);
        out.push(Entry {
            path: p,
            is_dir: h.entry_type().is_dir(),
            packed: size,
            size,
            modified: h.mtime().map(|t| fmt_unix(t as i64)).unwrap_or_default(),
            method: method.into(),
            crc: None,
            encrypted: false,
        });
    }
    Ok(out)
}

fn sz_err(e: sevenz_rust2::Error, pw: Option<&str>) -> Error {
    match e {
        sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_) => {
            if pw.is_some() {
                Error::WrongPassword
            } else {
                Error::NeedPassword
            }
        }
        e => Error::Msg(e.to_string()),
    }
}

fn open_7z(path: &Path, pw: Option<&str>) -> Res<ArchiveReader<Box<dyn Src>>> {
    let pass = pw.map_or_else(Password::empty, Password::new);
    ArchiveReader::new(source(path)?, pass).map_err(|e| {
        // An encrypted file list read with a wrong key fails in odd ways; check without a password.
        if pw.is_some() {
            if let Ok(s) = source(path) {
                if matches!(
                    ArchiveReader::new(s, Password::empty()),
                    Err(sevenz_rust2::Error::PasswordRequired)
                ) {
                    return Error::WrongPassword;
                }
            }
        }
        split_hint(path, sz_err(e, pw))
    })
}

/// Whether any 7z block is AES encrypted.
fn sz_encrypted<R: Read + Seek>(r: &ArchiveReader<R>) -> bool {
    r.archive().blocks.iter().any(|b| {
        b.coders
            .iter()
            .any(|c| c.encoder_method_id() == EncoderMethod::ID_AES256_SHA256)
    })
}

/// Decoding garbage after a wrong AES key fails in many ways (bad LZMA data, short reads).
fn sz_decode_err(e: Error, enc: bool, pw: Option<&str>) -> Error {
    match e {
        Error::Msg(_) if enc && pw.is_some() => Error::WrongPassword,
        e => e,
    }
}

fn sz_name(e: &ArchiveEntry) -> String {
    norm(e.name())
}

fn list_7z(path: &Path, pw: Option<&str>) -> Res<Vec<Entry>> {
    let r = open_7z(path, pw)?;
    let enc = sz_encrypted(&r);
    let a = r.archive();
    Ok(a.files
        .iter()
        .map(|e| {
            let secs = std::time::SystemTime::from(e.last_modified_date())
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            Entry {
                path: sz_name(e),
                is_dir: e.is_directory(),
                packed: e.compressed_size,
                size: e.size(),
                modified: if e.has_last_modified_date {
                    fmt_unix(secs)
                } else {
                    String::new()
                },
                method: "LZMA2".into(),
                crc: e.has_crc.then_some(e.crc as u32),
                encrypted: enc && !e.is_directory(),
            }
        })
        .collect())
}

// RAR (read-only, via the UnRAR library).

fn rar_err(e: unrar::error::UnrarError, path: &Path, encrypted: bool, pw: Option<&str>) -> Error {
    use unrar::error::Code;
    match e.code {
        Code::MissingPassword => Error::NeedPassword,
        Code::BadPassword => Error::WrongPassword,
        Code::BadData if encrypted && pw.is_some() => Error::WrongPassword,
        Code::EOpen if is_split(path) => {
            next_volume(path).map_or_else(|| Error::Msg(e.to_string()), |n| missing(&n))
        }
        _ => Error::Msg(e.to_string()),
    }
}

fn rar_archive<'a>(first: &'a Path, pw: Option<&'a str>) -> unrar::Archive<'a> {
    match pw {
        Some(p) => unrar::Archive::with_password(first, p),
        None => unrar::Archive::new(first),
    }
}

fn list_rar(path: &Path, pw: Option<&str>) -> Res<Vec<Entry>> {
    let first = first_volume(path);
    let ar = rar_archive(&first, pw)
        .open_for_listing()
        .map_err(|e| rar_err(e, path, true, pw))?;
    let mut out = Vec::new();
    for h in ar {
        let h = h.map_err(|e| rar_err(e, path, true, pw))?;
        out.push(Entry {
            path: norm(&h.filename.to_string_lossy()),
            is_dir: h.is_directory(),
            packed: 0,
            size: h.unpacked_size,
            modified: fmt_dos(h.file_time),
            method: if h.method == 0x30 { "Store" } else { "RAR" }.into(),
            crc: (!h.is_directory()).then_some(h.file_crc),
            encrypted: h.is_encrypted(),
        });
    }
    Ok(out)
}

/// Extract (`dest` Some) or test (None) RAR entries.
fn process_rar(
    path: &Path,
    dest: Option<&Path>,
    wanted: &dyn Fn(&str) -> bool,
    pw: Option<&str>,
) -> Res<usize> {
    let first = first_volume(path);
    let mut ar = rar_archive(&first, pw)
        .open_for_processing()
        .map_err(|e| rar_err(e, path, true, pw))?;
    let mut count = 0;
    loop {
        let Some(h) = ar.read_header().map_err(|e| rar_err(e, path, true, pw))? else {
            break;
        };
        let (name, dir, enc) = {
            let e = h.entry();
            (
                norm(&e.filename.to_string_lossy()),
                e.is_directory(),
                e.is_encrypted(),
            )
        };
        let rel = safe_rel(&name).filter(|_| wanted(&name));
        let fail = |e| named(&name, rar_err(e, path, enc, pw));
        ar = match (rel, dest) {
            (Some(_), None) if !dir => {
                tick()?;
                count += 1;
                h.test().map_err(fail)?
            }
            (Some(rel), Some(d)) => {
                let out = d.join(rel);
                if dir {
                    fs::create_dir_all(&out).map_err(err)?;
                    h.skip().map_err(fail)?
                } else {
                    tick()?;
                    if let Some(p) = out.parent() {
                        fs::create_dir_all(p).map_err(err)?;
                    }
                    count += 1;
                    // UnRAR itself refuses symlinks that point outside the destination.
                    h.extract_to(&out).map_err(fail)?
                }
            }
            _ => h.skip().map_err(fail)?,
        };
    }
    Ok(count)
}

// ALZ (read-only, via unalz).

fn alz_err(e: unalz::error::AlzError, encrypted: bool) -> Error {
    use unalz::error::AlzError as A;
    match e {
        A::PasswordNotSet => Error::NeedPassword,
        A::InvalidPassword => Error::WrongPassword,
        // ZipCrypto passes a wrong password 1 time in 256; the CRC then fails.
        A::InvalidFileCrc { .. } if encrypted => Error::WrongPassword,
        e => Error::Msg(e.to_string()),
    }
}

fn alz_open(path: &Path) -> Res<unalz::archive::AlzArchive> {
    let first = first_volume(path);
    let ar = unalz::archive::AlzArchive::open(&first.to_string_lossy())
        .map_err(|e| alz_err(e, false))?;
    if ar.truncated {
        return Err(match next_volume(path) {
            Some(n) => missing(&n),
            None => tr("The archive is incomplete.").into(),
        });
    }
    Ok(ar)
}

fn list_alz(path: &Path) -> Res<Vec<Entry>> {
    Ok(alz_open(path)?
        .entries
        .iter()
        .map(|e| Entry {
            path: norm(&e.file_name),
            is_dir: e.is_directory(),
            packed: e.compressed_size,
            size: e.uncompressed_size,
            modified: fmt_dos(e.file_time_date),
            method: e.compression_method.to_string(),
            crc: (!e.is_directory()).then_some(e.file_crc),
            encrypted: e.is_encrypted(),
        })
        .collect())
}

fn extract_alz(
    path: &Path,
    dest: &Path,
    wanted: &dyn Fn(&str) -> bool,
    pw: Option<&str>,
) -> Res<usize> {
    let mut ar = alz_open(path)?;
    let mut count = 0;
    for e in ar.entries.clone() {
        let name = norm(&e.file_name);
        if !wanted(&name) || safe_rel(&name).is_none() {
            continue;
        }
        if !e.is_directory() {
            tick()?;
            count += 1;
        }
        unalz::extract::extract_entry(&mut ar, &e, dest, pw, false)
            .map_err(|x| named(&name, alz_err(x, e.is_encrypted())))?;
    }
    Ok(count)
}

// EGG (read-only, via unegg).

fn egg_err(e: unegg::error::EggError, encrypted: bool) -> Error {
    use unegg::error::EggError as G;
    match e {
        G::PasswordNotSet => Error::NeedPassword,
        G::InvalidPassword | G::AuthenticationFailed => Error::WrongPassword,
        G::InvalidFileCrc { .. } if encrypted => Error::WrongPassword,
        e => Error::Msg(e.to_string()),
    }
}

fn egg_open(path: &Path) -> Res<unegg::archive::EggArchive<Box<dyn Src>>> {
    let first = first_volume(path);
    let src: Box<dyn Src> = match unegg::volume::MultiVolumeReader::try_open(&first) {
        Ok(Some(m)) => Box::new(BufReader::new(m)),
        Ok(None) => Box::new(BufReader::new(File::open(&first).map_err(err)?)),
        Err(unegg::error::EggError::CantOpenFile(e)) if e.kind() == io::ErrorKind::NotFound => {
            return Err(next_volume(path).map_or_else(|| err(e).into(), |n| missing(&n)))
        }
        Err(e) => return Err(egg_err(e, false)),
    };
    unegg::archive::EggArchive::open(src).map_err(|e| split_hint(path, egg_err(e, false)))
}

fn list_egg(path: &Path) -> Res<Vec<Entry>> {
    let ar = egg_open(path)?;
    Ok(ar
        .entries
        .iter()
        .map(|e| {
            let first = e.blocks.first();
            Entry {
                path: norm(&e.file_name),
                is_dir: e.is_directory(),
                packed: e.blocks.iter().map(|b| b.compressed_size as u64).sum(),
                size: e.uncompressed_size,
                // FILETIME: 100 ns ticks since 1601.
                modified: e
                    .file_time
                    .map(|t| fmt_unix((t / 10_000_000) as i64 - 11_644_473_600))
                    .unwrap_or_default(),
                method: first
                    .map(|b| b.compression_method.name())
                    .unwrap_or("")
                    .into(),
                crc: first.filter(|_| e.blocks.len() == 1).map(|b| b.crc32),
                encrypted: e.encrypt_info.is_some(),
            }
        })
        .collect())
}

fn extract_egg(
    path: &Path,
    dest: &Path,
    wanted: &dyn Fn(&str) -> bool,
    pw: Option<&str>,
) -> Res<usize> {
    let mut ar = egg_open(path)?;
    let picked: Vec<_> = ar
        .entries
        .iter()
        .filter(|e| {
            let n = norm(&e.file_name);
            wanted(&n) && safe_rel(&n).is_some()
        })
        .cloned()
        .collect();
    let encrypted = picked.iter().any(|e| e.encrypt_info.is_some());
    if encrypted {
        let p = pw.ok_or(Error::NeedPassword)?;
        if !unegg::extract::verify_password(&picked, p).map_err(|e| egg_err(e, true))? {
            return Err(Error::WrongPassword);
        }
    }
    let count = picked.iter().filter(|e| !e.is_directory()).count();
    if ar.is_solid {
        // One shared stream: the library decodes it in a single pass.
        let names: Vec<String> = picked.iter().map(|e| e.file_name.clone()).collect();
        if !names.is_empty() {
            unegg::extract::extract_files(&mut ar, dest, pw, false, &names)
                .map_err(|e| egg_err(e, encrypted))?;
        }
        PROGRESS.done.fetch_add(count, Ordering::Relaxed);
    } else {
        for e in &picked {
            if !e.is_directory() {
                tick()?;
            }
            unegg::extract::extract_entry(&mut ar, e, dest, pw, false)
                .map_err(|x| named(&e.file_name, egg_err(x, e.encrypt_info.is_some())))?;
        }
    }
    Ok(count)
}

/// Relative path for an archive name, or None if it tries to leave the destination.
fn safe_rel(name: &str) -> Option<PathBuf> {
    if name.starts_with(['/', '\\']) {
        return None;
    }
    let mut p = PathBuf::new();
    for c in name.split(['/', '\\']) {
        match c {
            "" | "." => {}
            ".." => return None,
            c if c.contains(':') => return None,
            c => p.push(c),
        }
    }
    (!p.as_os_str().is_empty()).then_some(p)
}

/// Archives often omit folder entries; add them so the folder tree is complete.
fn add_missing_dirs(entries: &mut Vec<Entry>) {
    let known: std::collections::HashSet<String> = entries
        .iter()
        .filter(|e| e.is_dir)
        .map(|e| e.path.clone())
        .collect();
    let mut missing = std::collections::BTreeSet::new();
    for e in entries.iter() {
        let mut p = e.parent();
        while !p.is_empty() {
            if !known.contains(p) {
                missing.insert(p.to_string());
            }
            p = p.rsplit_once('/').map(|(a, _)| a).unwrap_or("");
        }
    }
    for p in missing {
        entries.push(Entry {
            path: p,
            is_dir: true,
            packed: 0,
            size: 0,
            modified: String::new(),
            method: String::new(),
            crc: None,
            encrypted: false,
        });
    }
}

// ---------- extract / test ----------

type ZipEntry<'a> = (String, bool, zip::read::ZipFile<'a, Box<dyn Src>>);

/// Open entry `i` for reading, decrypting when needed: (name, encrypted, reader).
fn zip_entry<'a>(
    za: &'a mut ZipArchive<Box<dyn Src>>,
    i: usize,
    pw: Option<&str>,
) -> Res<ZipEntry<'a>> {
    let (name, enc, method, dir) = {
        let f = za.by_index_raw(i).map_err(err)?;
        (zname(&f), f.encrypted(), f.compression(), f.is_dir())
    };
    // The crate decodes LZMA but leaves it out of its list.
    let known =
        zip::SUPPORTED_COMPRESSION_METHODS.contains(&method) || method == CompressionMethod::Lzma;
    if !dir && !known {
        return Err(trf(
            "{name}: compression method {m} is not supported.",
            &[("name", &name), ("m", &method_name(method))],
        )
        .into());
    }
    let f = if enc && !dir {
        let p = pw.ok_or(Error::NeedPassword)?;
        za.by_index_decrypt(i, p.as_bytes()).map_err(|e| match e {
            ZipError::InvalidPassword => Error::WrongPassword,
            e => named(&name, err(e).into()),
        })?
    } else {
        za.by_index(i).map_err(|e| named(&name, err(e).into()))?
    };
    Ok((name, enc, f))
}

/// Read failures of an encrypted entry are almost always a wrong ZipCrypto password.
fn zip_read_err(name: &str, enc: bool, e: io::Error) -> Error {
    if enc {
        Error::WrongPassword
    } else {
        Error::Msg(format!("{name}: {e}"))
    }
}

/// `only`: archive paths to extract (a folder selects everything under it); None = all.
pub fn extract(path: &Path, dest: &Path, only: Option<&[String]>, pw: Option<&str>) -> Res<usize> {
    let wanted = |p: &str| match only {
        None => true,
        Some(sel) => sel
            .iter()
            .any(|s| p == s || p.starts_with(&format!("{s}/"))),
    };
    fs::create_dir_all(dest).map_err(err)?;
    let mut count = 0;
    match Kind::detect(path).ok_or(tr("Unsupported archive format."))? {
        Kind::Zip => {
            let mut za = zip_open(path)?;
            for i in 0..za.len() {
                let name = zname(&za.by_index_raw(i).map_err(err)?);
                if !wanted(&name) {
                    continue;
                }
                let (name, enc, mut f) = zip_entry(&mut za, i, pw)?;
                // enclosed_name() rejects "../"; safe_rel() also refuses absolute names (zip-slip).
                let Some(rel) = f.enclosed_name().filter(|_| safe_rel(&name).is_some()) else {
                    continue;
                };
                let out = dest.join(rel);
                if f.is_dir() {
                    fs::create_dir_all(&out).map_err(err)?;
                    continue;
                }
                tick()?;
                if let Some(p) = out.parent() {
                    fs::create_dir_all(p).map_err(err)?;
                }
                // Symlink entries are written as plain files holding the link text.
                let mut w = BufWriter::new(File::create(&out).map_err(err)?);
                io::copy(&mut f, &mut w).map_err(|e| zip_read_err(&name, enc, e))?;
                count += 1;
            }
        }
        Kind::SevenZ => {
            let mut r = open_7z(path, pw)?;
            let enc = sz_encrypted(&r);
            let mut fail = None;
            r.for_each_entries(|e, rd| {
                let name = sz_name(e);
                let Some(rel) = safe_rel(&name).filter(|_| wanted(&name)) else {
                    io::copy(rd, &mut io::sink())?;
                    return Ok(true);
                };
                let out = dest.join(rel);
                let res = if e.is_directory() {
                    fs::create_dir_all(&out).map_err(|x| named(&name, err(x).into()))
                } else {
                    tick().and_then(|_| {
                        out.parent()
                            .map_or(Ok(()), fs::create_dir_all)
                            .and_then(|_| {
                                let mut w = BufWriter::new(File::create(&out)?);
                                io::copy(rd, &mut w).map(|_| count += 1)
                            })
                            .map_err(|x| named(&name, err(x).into()))
                    })
                };
                if let Err(x) = res {
                    fail = Some(x);
                    return Ok(false);
                }
                Ok(true)
            })
            .map_err(|e| sz_decode_err(sz_err(e, pw), enc, pw))?;
            if let Some(f) = fail {
                return Err(sz_decode_err(f, enc, pw));
            }
        }
        Kind::Tar => count = extract_tar(tar::Archive::new(source(path)?), dest, &wanted)?,
        Kind::Tgz => {
            count = extract_tar(
                tar::Archive::new(GzDecoder::new(source(path)?)),
                dest,
                &wanted,
            )?
        }
        Kind::Rar => count = process_rar(path, Some(dest), &wanted, pw)?,
        Kind::Alz => count = extract_alz(path, dest, &wanted, pw)?,
        Kind::Egg => count = extract_egg(path, dest, &wanted, pw)?,
    }
    Ok(count)
}

/// A link target that stays inside the extraction folder.
fn link_inside(entry: &str, target: &Path) -> bool {
    let t = target.to_string_lossy().replace('\\', "/");
    if t.starts_with('/') || t.contains(':') {
        return false;
    }
    let mut depth = entry.matches('/').count() as i64;
    for c in t.split('/') {
        match c {
            "" | "." => {}
            ".." => depth -= 1,
            _ => depth += 1,
        }
        if depth < 0 {
            return false;
        }
    }
    true
}

fn extract_tar<R: Read>(
    mut ar: tar::Archive<R>,
    dest: &Path,
    wanted: &dyn Fn(&str) -> bool,
) -> Res<usize> {
    let mut count = 0;
    for e in ar.entries().map_err(err)? {
        let mut e = e.map_err(err)?;
        let p = e.path().map_err(err)?.to_string_lossy().replace('\\', "/");
        let p = p.trim_start_matches("./").trim_end_matches('/').to_string();
        if p.is_empty() || !wanted(&p) {
            continue;
        }
        let ty = e.header().entry_type();
        if ty.is_symlink() || ty.is_hard_link() {
            let target = e.link_name().map_err(err)?.unwrap_or_default().into_owned();
            // Hard link targets are archive paths; symlink targets are relative to the link.
            let ok = if ty.is_hard_link() {
                safe_rel(&target.to_string_lossy()).is_some()
            } else {
                link_inside(&p, &target)
            };
            if !ok {
                continue;
            }
        }
        tick()?;
        // unpack_in refuses paths that escape `dest`.
        if e.unpack_in(dest).map_err(|x| format!("{p}: {x}"))? && !ty.is_dir() {
            count += 1;
        }
    }
    Ok(count)
}

/// Scratch folder for formats whose library only extracts to disk.
fn scratch() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "tailzip-check-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Read every file fully and check CRCs. Returns the number of files tested.
pub fn test(path: &Path, pw: Option<&str>) -> Res<usize> {
    let mut count = 0;
    let kind = Kind::detect(path).ok_or(tr("Unsupported archive format."))?;
    match kind {
        Kind::Zip => {
            let mut za = zip_open(path)?;
            for i in 0..za.len() {
                let (name, enc, mut f) = zip_entry(&mut za, i, pw)?;
                if f.is_dir() {
                    continue;
                }
                tick()?;
                io::copy(&mut f, &mut io::sink()).map_err(|e| zip_read_err(&name, enc, e))?;
                count += 1;
            }
        }
        Kind::SevenZ => {
            let mut cancelled = false;
            let mut r = open_7z(path, pw)?;
            let enc = sz_encrypted(&r);
            r.for_each_entries(|e, rd| {
                io::copy(rd, &mut io::sink())?;
                if !e.is_directory() {
                    if tick().is_err() {
                        cancelled = true;
                        return Ok(false);
                    }
                    count += 1;
                }
                Ok(true)
            })
            .map_err(|e| sz_decode_err(sz_err(e, pw), enc, pw))?;
            if cancelled {
                return Err(Error::Cancelled);
            }
        }
        Kind::Tar | Kind::Tgz => {
            let r: Box<dyn Read> = if kind == Kind::Tgz {
                Box::new(GzDecoder::new(source(path)?))
            } else {
                Box::new(source(path)?)
            };
            let mut ar = tar::Archive::new(r);
            for e in ar.entries().map_err(err)? {
                let mut e = e.map_err(err)?;
                if !e.header().entry_type().is_dir() {
                    tick()?;
                    io::copy(&mut e, &mut io::sink()).map_err(err)?;
                    count += 1;
                }
            }
        }
        Kind::Rar => count = process_rar(path, None, &|_| true, pw)?,
        Kind::Alz | Kind::Egg => {
            // The libraries verify CRCs while writing; extract to a scratch folder and drop it.
            let dir = scratch();
            let r = if kind == Kind::Alz {
                extract_alz(path, &dir, &|_| true, pw)
            } else {
                extract_egg(path, &dir, &|_| true, pw)
            };
            let _ = fs::remove_dir_all(&dir);
            count = r?;
        }
    }
    Ok(count)
}

// ---------- create / add / delete ----------

/// Files on disk to store: (path on disk, name inside archive).
fn gather(inputs: &[PathBuf], prefix: &str) -> Vec<(PathBuf, String)> {
    fn walk(p: &Path, name: String, out: &mut Vec<(PathBuf, String)>) {
        out.push((p.to_path_buf(), name.clone()));
        if p.is_dir() {
            if let Ok(rd) = fs::read_dir(p) {
                let mut kids: Vec<_> = rd.flatten().map(|d| d.path()).collect();
                kids.sort();
                for k in kids {
                    let n = k
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    walk(&k, format!("{name}/{n}"), out);
                }
            }
        }
    }
    let mut out = Vec::new();
    for p in inputs {
        let n = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let n = if prefix.is_empty() {
            n
        } else {
            format!("{prefix}/{n}")
        };
        walk(p, n, &mut out);
    }
    out
}

/// `enc`: (password, ZipCrypto instead of AES-256).
fn zip_opts<'k>(
    level: u8,
    src: &Path,
    enc: Option<(&'k str, bool)>,
) -> Res<FileOptions<'k, 'static, ()>> {
    let base = FileOptions::<'k, 'static, ()>::default()
        .last_modified_time(zip_time(src))
        .large_file(true);
    let base = if level == 0 {
        base.compression_method(CompressionMethod::Stored)
    } else {
        base.compression_method(CompressionMethod::Deflated)
            .compression_level(Some(deflate_level(level) as i64))
    };
    Ok(match enc {
        None => base,
        Some((pw, false)) => base.with_aes_encryption(AesMode::Aes256, pw),
        Some((pw, true)) => base
            .with_deprecated_encryption(pw.as_bytes())
            .map_err(err)?,
    })
}

fn write_zip_items<W: Write + Seek>(
    zw: &mut ZipWriter<W>,
    items: &[(PathBuf, String)],
    level: u8,
    enc: Option<(&str, bool)>,
) -> Res<()> {
    for (src, name) in items {
        tick()?;
        if src.is_dir() {
            zw.add_directory(name.as_str(), zip_opts(level, src, None)?)
                .map_err(err)?;
        } else {
            zw.start_file(name.as_str(), zip_opts(level, src, enc)?)
                .map_err(err)?;
            io::copy(&mut BufReader::new(File::open(src).map_err(err)?), zw).map_err(err)?;
        }
    }
    Ok(())
}

fn sz_writer<W: Write + Seek>(
    w: W,
    level: u8,
    pw: Option<&str>,
    encrypt_names: bool,
) -> Res<ArchiveWriter<W>> {
    let mut sw = ArchiveWriter::new(w).map_err(err)?;
    let method = if level == 0 {
        EncoderMethod::COPY.into()
    } else {
        EncoderConfiguration::new(EncoderMethod::LZMA2)
            .with_options(Lzma2Options::from_level(deflate_level(level)).into())
    };
    let mut methods = vec![method];
    if let Some(p) = pw {
        methods.insert(0, AesEncoderOptions::new(Password::new(p)).into());
    }
    sw.set_content_methods(methods);
    sw.set_encrypt_header(pw.is_some() && encrypt_names);
    Ok(sw)
}

fn write_7z_items<W: Write + Seek>(
    sw: &mut ArchiveWriter<W>,
    items: &[(PathBuf, String)],
) -> Res<()> {
    for (src, name) in items {
        tick()?;
        if src.is_dir() {
            sw.push_archive_entry::<&[u8]>(ArchiveEntry::new_directory(name), None)
                .map_err(err)?;
        } else {
            let f = BufReader::new(File::open(src).map_err(err)?);
            sw.push_archive_entry(ArchiveEntry::from_path(src, name.clone()), Some(f))
                .map_err(err)?;
        }
    }
    Ok(())
}

fn write_tar_items<W: Write>(b: &mut tar::Builder<W>, items: &[(PathBuf, String)]) -> Res<()> {
    for (src, name) in items {
        tick()?;
        if src.is_dir() {
            b.append_dir(name, src).map_err(err)?;
        } else {
            b.append_path_with_name(src, name).map_err(err)?;
        }
    }
    Ok(())
}

/// Path of raw volume `i` (0-based) of `out`: name.7z.001, name.7z.002, ...
pub fn volume_path(out: &Path, i: usize) -> PathBuf {
    let mut s = out.as_os_str().to_owned();
    s.push(format!(".{:03}", i + 1));
    PathBuf::from(s)
}

/// Cut `tmp` into raw volumes of `size` bytes next to `out`, removing stale ones first.
fn split_into(tmp: &Path, out: &Path, size: u64) -> Res<()> {
    let size = size.max(1);
    let mut i = 0;
    while i < MAX_VOLUMES && volume_path(out, i).is_file() {
        let _ = fs::remove_file(volume_path(out, i));
        i += 1;
    }
    let len = fs::metadata(tmp).map_err(err)?.len();
    let mut r = BufReader::new(File::open(tmp).map_err(err)?);
    for i in 0..len.div_ceil(size).max(1) as usize {
        let mut w = BufWriter::new(File::create(volume_path(out, i)).map_err(err)?);
        io::copy(&mut (&mut r).take(size), &mut w).map_err(err)?;
        w.flush().map_err(err)?;
    }
    Ok(())
}

pub fn create(out: &Path, kind: Kind, inputs: &[PathBuf], level: u8, opts: &NewOpts) -> Res<usize> {
    let items = gather(inputs, "");
    PROGRESS.total.store(items.len(), Ordering::Relaxed);
    let pw = opts.password.as_deref().filter(|p| !p.is_empty());
    let tmp = out.with_extension("part");
    let res = (|| -> Res<()> {
        let f = BufWriter::new(File::create(&tmp).map_err(err)?);
        match kind {
            Kind::Zip => {
                let mut zw = ZipWriter::new(f);
                write_zip_items(&mut zw, &items, level, pw.map(|p| (p, opts.zip_crypto)))?;
                zw.finish().map_err(err)?.flush().map_err(err)?;
            }
            Kind::SevenZ => {
                let mut sw = sz_writer(f, level, pw, opts.encrypt_names)?;
                write_7z_items(&mut sw, &items)?;
                sw.finish().map_err(err)?.flush().map_err(err)?;
            }
            Kind::Tar => {
                let mut b = tar::Builder::new(f);
                write_tar_items(&mut b, &items)?;
                b.into_inner().map_err(err)?.flush().map_err(err)?;
            }
            Kind::Tgz => {
                let mut b =
                    tar::Builder::new(GzEncoder::new(f, Compression::new(deflate_level(level))));
                write_tar_items(&mut b, &items)?;
                b.into_inner()
                    .map_err(err)?
                    .finish()
                    .map_err(err)?
                    .flush()
                    .map_err(err)?;
            }
            Kind::Rar | Kind::Alz | Kind::Egg => return Err(read_only_msg().into()),
        }
        Ok(())
    })();
    match opts.volume_size {
        Some(size) if res.is_ok() => {
            let r = split_into(&tmp, out, size);
            let _ = fs::remove_file(&tmp);
            r?;
        }
        _ => finish_tmp(res, &tmp, out)?,
    }
    Ok(items.iter().filter(|(p, _)| !p.is_dir()).count())
}

fn finish_tmp(res: Res<()>, tmp: &Path, out: &Path) -> Res<()> {
    match res {
        Ok(()) => fs::rename(tmp, out).map_err(|e| err(e).into()),
        Err(e) => {
            let _ = fs::remove_file(tmp);
            Err(e)
        }
    }
}

pub fn read_only_msg() -> &'static str {
    tr("RAR, ALZ and EGG archives are read-only. Extract the files and create a new archive to change them.")
}

/// Why the archive cannot be changed, if it cannot.
pub fn edit_block(path: &Path, kind: Kind) -> Option<&'static str> {
    if kind.read_only() {
        Some(read_only_msg())
    } else if is_split(path) {
        Some(tr(
            "Split archives cannot be changed. Extract the files and create a new archive instead.",
        ))
    } else {
        None
    }
}

/// Rewrite the archive keeping entries where `keep` is true, then append `add` under `prefix`.
/// The original file is replaced only after the new one is complete.
fn rewrite(
    path: &Path,
    keep: &dyn Fn(&str) -> bool,
    add: &[PathBuf],
    prefix: &str,
    level: u8,
    pw: Option<&str>,
) -> Res<()> {
    let kind = Kind::detect(path).ok_or(tr("Unsupported archive format."))?;
    if let Some(m) = edit_block(path, kind) {
        return Err(m.into());
    }
    let items = gather(add, prefix);
    let replaced: std::collections::HashSet<&str> = items.iter().map(|(_, n)| n.as_str()).collect();
    let keep = |p: &str| keep(p) && !replaced.contains(p);
    let tmp = path.with_extension("part");
    let res = (|| -> Res<()> {
        let f = BufWriter::new(File::create(&tmp).map_err(err)?);
        match kind {
            Kind::Zip => {
                // Encrypted entries are copied as they are; new files get AES-256 when a password is known.
                let mut za = zip_open(path)?;
                let mut zw = ZipWriter::new(f);
                for i in 0..za.len() {
                    let e = za.by_index_raw(i).map_err(err)?;
                    if keep(&zname(&e)) {
                        zw.raw_copy_file(e).map_err(err)?;
                    }
                }
                write_zip_items(&mut zw, &items, level, pw.map(|p| (p, false)))?;
                zw.finish().map_err(err)?.flush().map_err(err)?;
            }
            Kind::SevenZ => {
                // ponytail: re-compresses every kept file; fine for normal sizes, slow for huge archives.
                // Keep the file list encrypted if it was.
                let hidden_names = pw.is_some()
                    && matches!(
                        ArchiveReader::new(source(path)?, Password::empty()),
                        Err(sevenz_rust2::Error::PasswordRequired)
                    );
                let mut sw = sz_writer(f, level, pw, hidden_names)?;
                let mut fail = None;
                open_7z(path, pw)?
                    .for_each_entries(|e, rd| {
                        let res = if !keep(&sz_name(e)) {
                            io::copy(rd, &mut io::sink()).map(|_| ()).map_err(err)
                        } else if e.is_directory() {
                            sw.push_archive_entry::<&[u8]>(e.clone(), None)
                                .map(|_| ())
                                .map_err(err)
                        } else {
                            sw.push_archive_entry(e.clone(), Some(rd))
                                .map(|_| ())
                                .map_err(err)
                        };
                        if let Err(x) = res {
                            fail = Some(x);
                            return Ok(false);
                        }
                        Ok(true)
                    })
                    .map_err(|e| sz_err(e, pw))?;
                if let Some(f) = fail {
                    return Err(f.into());
                }
                write_7z_items(&mut sw, &items)?;
                sw.finish().map_err(err)?.flush().map_err(err)?;
            }
            Kind::Tar => {
                let mut b = tar::Builder::new(f);
                copy_tar(tar::Archive::new(source(path)?), &mut b, &keep)?;
                write_tar_items(&mut b, &items)?;
                b.into_inner().map_err(err)?.flush().map_err(err)?;
            }
            Kind::Tgz => {
                let mut b =
                    tar::Builder::new(GzEncoder::new(f, Compression::new(deflate_level(level))));
                copy_tar(
                    tar::Archive::new(GzDecoder::new(source(path)?)),
                    &mut b,
                    &keep,
                )?;
                write_tar_items(&mut b, &items)?;
                b.into_inner()
                    .map_err(err)?
                    .finish()
                    .map_err(err)?
                    .flush()
                    .map_err(err)?;
            }
            Kind::Rar | Kind::Alz | Kind::Egg => return Err(read_only_msg().into()),
        }
        Ok(())
    })();
    finish_tmp(res, &tmp, path)
}

fn copy_tar<R: Read, W: Write>(
    mut ar: tar::Archive<R>,
    b: &mut tar::Builder<W>,
    keep: &dyn Fn(&str) -> bool,
) -> Res<()> {
    for e in ar.entries().map_err(err)? {
        let mut e = e.map_err(err)?;
        let p = e.path().map_err(err)?.to_string_lossy().replace('\\', "/");
        if keep(p.trim_start_matches("./").trim_end_matches('/')) {
            let mut h = e.header().clone();
            b.append_data(&mut h, p.as_str(), &mut e).map_err(err)?;
        }
    }
    Ok(())
}

pub fn add(path: &Path, inputs: &[PathBuf], prefix: &str, level: u8, pw: Option<&str>) -> Res<()> {
    rewrite(path, &|_| true, inputs, prefix, level, pw)
}

pub fn delete(path: &Path, sel: &[String], pw: Option<&str>) -> Res<()> {
    let hit = |p: &str| {
        sel.iter()
            .any(|s| p == s || p.starts_with(&format!("{s}/")))
    };
    rewrite(path, &|p| !hit(p), &[], "", 2, pw)
}

#[cfg(test)]
#[path = "archive_tests.rs"]
mod fixture_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_formats() {
        let dir = std::env::temp_dir().join(format!("tailzip-test-{}", std::process::id()));
        let src = dir.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), b"hello hello hello").unwrap();
        fs::write(src.join("sub/b.txt"), b"world").unwrap();
        fs::write(dir.join("extra.txt"), b"extra").unwrap();
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(951782400), (2000, 2, 29, 0, 0, 0));
        for kind in Kind::ALL {
            let ar = dir.join(format!("t.{}", kind.ext()));
            let n = create(
                &ar,
                kind,
                std::slice::from_ref(&src),
                2,
                &NewOpts::default(),
            )
            .unwrap();
            assert_eq!(n, 2);
            assert_eq!(Kind::detect(&ar), Some(kind));
            add(&ar, &[dir.join("extra.txt")], "src/sub", 2, None).unwrap();
            delete(&ar, &["src/a.txt".into()], None).unwrap();
            let (_, list) = list(&ar, None).unwrap();
            let names: Vec<_> = list.iter().map(|e| e.path.as_str()).collect();
            assert!(
                names.contains(&"src/sub/extra.txt") && !names.contains(&"src/a.txt"),
                "{kind:?} {names:?}"
            );
            assert_eq!(test(&ar, None).unwrap(), 2);
            let out = dir.join(format!("out-{}", kind.ext()));
            extract(&ar, &out, Some(&["src/sub".into()]), None).unwrap();
            assert_eq!(fs::read(out.join("src/sub/b.txt")).unwrap(), b"world");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn volume_names() {
        let dir = std::env::temp_dir().join(format!("tailzip-vols-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let touch = |n: &str| fs::write(dir.join(n), b"x").unwrap();
        let names = |p: &str| -> Vec<String> {
            volumes(&dir.join(p))
                .unwrap()
                .iter()
                .map(|v| v.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        for n in [
            "a.7z.001",
            "a.7z.002",
            "b.z01",
            "b.z02",
            "b.zip",
            "c.part01.rar",
            "c.part02.rar",
            "d.rar",
            "d.r00",
            "d.r01",
            "e.alz",
            "e.a00",
            "f.vol1.egg",
            "f.vol2.egg",
            "G.ZIP",
            "G.Z01",
        ] {
            touch(n);
        }
        assert_eq!(names("a.7z.002"), ["a.7z.001", "a.7z.002"]);
        assert_eq!(names("b.z02"), ["b.z01", "b.z02", "b.zip"]);
        assert_eq!(names("b.zip"), ["b.z01", "b.z02", "b.zip"]);
        assert_eq!(names("c.part02.rar"), ["c.part01.rar", "c.part02.rar"]);
        assert_eq!(names("d.r01"), ["d.rar", "d.r00", "d.r01"]);
        assert_eq!(names("e.a00"), ["e.alz", "e.a00"]);
        assert_eq!(names("f.vol2.egg"), ["f.vol1.egg", "f.vol2.egg"]);
        assert_eq!(names("G.Z01"), ["G.Z01", "G.ZIP"]);
        assert_eq!(
            next_volume(&dir.join("a.7z.001")),
            Some(dir.join("a.7z.003"))
        );
        assert_eq!(base_name(&dir.join("c.part02.rar")), "c");
        assert_eq!(base_name(&dir.join("a.7z.002")), "a");
        assert_eq!(base_name(&dir.join("x.tar.gz")), "x");
        fs::remove_file(dir.join("a.7z.001")).unwrap();
        let e = volumes(&dir.join("a.7z.002")).unwrap_err().to_string();
        assert!(e.contains("a.7z.001"), "{e}");
        assert!(!is_split(&dir.join("plain.zip")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn links_must_stay_inside() {
        assert!(link_inside("a/b", Path::new("c")));
        assert!(link_inside("a/b", Path::new("../c")));
        assert!(!link_inside("a/b", Path::new("../../c")));
        assert!(!link_inside("a", Path::new("/etc/passwd")));
        assert_eq!(safe_rel("/abs"), None);
        assert_eq!(safe_rel("a/../../b"), None);
        assert_eq!(safe_rel("C:/x"), None);
    }
}
