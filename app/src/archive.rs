// Archive engine: list, extract, test, create, add to and delete from ZIP, 7Z, TAR and TAR.GZ.
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::i18n::tr;
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use sevenz_rust2::{
    encoder_options::Lzma2Options, ArchiveEntry, ArchiveReader, ArchiveWriter,
    EncoderConfiguration, EncoderMethod, Password,
};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

pub type Res<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Zip,
    SevenZ,
    Tar,
    Tgz,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Zip, Kind::SevenZ, Kind::Tar, Kind::Tgz];

    pub fn ext(self) -> &'static str {
        match self {
            Kind::Zip => "zip",
            Kind::SevenZ => "7z",
            Kind::Tar => "tar",
            Kind::Tgz => "tgz",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Zip => "ZIP",
            Kind::SevenZ => "7Z",
            Kind::Tar => "TAR",
            Kind::Tgz => "TGZ (tar.gz)",
        }
    }

    /// Detect by file signature first, then by extension.
    pub fn detect(path: &Path) -> Option<Kind> {
        let mut head = [0u8; 512];
        let n = File::open(path)
            .and_then(|mut f| f.read(&mut head))
            .unwrap_or(0);
        if n >= 4 && &head[..2] == b"PK" {
            return Some(Kind::Zip);
        }
        if n >= 6 && head[..6] == [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C] {
            return Some(Kind::SevenZ);
        }
        if n >= 2 && head[0] == 0x1f && head[1] == 0x8b {
            return Some(Kind::Tgz);
        }
        if n >= 262 && &head[257..262] == b"ustar" {
            return Some(Kind::Tar);
        }
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        if name.ends_with(".zip") {
            Some(Kind::Zip)
        } else if name.ends_with(".7z") {
            Some(Kind::SevenZ)
        } else if name.ends_with(".tar") {
            Some(Kind::Tar)
        } else if name.ends_with(".tgz") || name.ends_with(".tar.gz") {
            Some(Kind::Tgz)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Path inside the archive, '/' separated, no trailing slash.
    pub path: String,
    pub is_dir: bool,
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

// ---------- list ----------

pub fn list(path: &Path) -> Res<(Kind, Vec<Entry>)> {
    let kind = Kind::detect(path).ok_or(tr("Unsupported archive format."))?;
    let mut entries = match kind {
        Kind::Zip => list_zip(path)?,
        Kind::SevenZ => list_7z(path)?,
        Kind::Tar => list_tar(
            tar::Archive::new(BufReader::new(File::open(path).map_err(err)?)),
            path,
            "Stored",
        )?,
        Kind::Tgz => list_tar(
            tar::Archive::new(GzDecoder::new(BufReader::new(
                File::open(path).map_err(err)?,
            ))),
            path,
            "GZip",
        )?,
    };
    add_missing_dirs(&mut entries);
    Ok((kind, entries))
}

fn list_zip(path: &Path) -> Res<Vec<Entry>> {
    let mut za = ZipArchive::new(BufReader::new(File::open(path).map_err(err)?)).map_err(err)?;
    let mut out = Vec::with_capacity(za.len());
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
    n.replace('\\', "/").trim_end_matches('/').to_string()
}

fn method_name(m: CompressionMethod) -> String {
    match m {
        CompressionMethod::Stored => "Store".into(),
        CompressionMethod::Deflated => "Deflate".into(),
        other => format!("{other:?}"),
    }
}

fn list_tar<R: Read>(mut ar: tar::Archive<R>, path: &Path, method: &str) -> Res<Vec<Entry>> {
    let _ = path;
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

fn open_7z(path: &Path) -> Res<ArchiveReader<File>> {
    ArchiveReader::open(path, Password::empty()).map_err(err)
}

fn sz_name(e: &ArchiveEntry) -> String {
    e.name()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn list_7z(path: &Path) -> Res<Vec<Entry>> {
    let r = open_7z(path)?;
    Ok(r.archive()
        .files
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
                encrypted: false,
            }
        })
        .collect())
}

/// Relative path for an archive name, or None if it tries to leave the destination.
fn safe_rel(name: &str) -> Option<PathBuf> {
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

/// `only`: archive paths to extract (a folder selects everything under it); None = all.
pub fn extract(path: &Path, dest: &Path, only: Option<&[String]>) -> Res<usize> {
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
            let mut za =
                ZipArchive::new(BufReader::new(File::open(path).map_err(err)?)).map_err(err)?;
            for i in 0..za.len() {
                let mut f = za.by_index(i).map_err(err)?;
                let name = zname(&f);
                if !wanted(&name) {
                    continue;
                }
                // enclosed_name() rejects "../" and absolute paths (zip-slip).
                let Some(rel) = f.enclosed_name() else {
                    continue;
                };
                let out = dest.join(rel);
                if f.is_dir() {
                    fs::create_dir_all(&out).map_err(err)?;
                    continue;
                }
                if let Some(p) = out.parent() {
                    fs::create_dir_all(p).map_err(err)?;
                }
                let mut w = BufWriter::new(File::create(&out).map_err(err)?);
                io::copy(&mut f, &mut w).map_err(|e| format!("{name}: {e}"))?;
                count += 1;
            }
        }
        Kind::SevenZ => {
            let mut r = open_7z(path)?;
            let mut fail = None;
            r.for_each_entries(|e, rd| {
                let name = sz_name(e);
                let Some(rel) = safe_rel(&name).filter(|_| wanted(&name)) else {
                    io::copy(rd, &mut io::sink())?;
                    return Ok(true);
                };
                let out = dest.join(rel);
                let res = if e.is_directory() {
                    fs::create_dir_all(&out)
                } else {
                    out.parent()
                        .map_or(Ok(()), fs::create_dir_all)
                        .and_then(|_| {
                            let mut w = BufWriter::new(File::create(&out)?);
                            io::copy(rd, &mut w).map(|_| count += 1)
                        })
                };
                if let Err(x) = res {
                    fail = Some(format!("{name}: {x}"));
                    return Ok(false);
                }
                Ok(true)
            })
            .map_err(err)?;
            if let Some(f) = fail {
                return Err(f);
            }
        }
        Kind::Tar => {
            count = extract_tar(
                tar::Archive::new(BufReader::new(File::open(path).map_err(err)?)),
                dest,
                &wanted,
            )?
        }
        Kind::Tgz => {
            count = extract_tar(
                tar::Archive::new(GzDecoder::new(BufReader::new(
                    File::open(path).map_err(err)?,
                ))),
                dest,
                &wanted,
            )?
        }
    }
    Ok(count)
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
        // unpack_in refuses paths that escape `dest`.
        if e.unpack_in(dest).map_err(|x| format!("{p}: {x}"))? && !e.header().entry_type().is_dir()
        {
            count += 1;
        }
    }
    Ok(count)
}

/// Read every file fully; ZIP checks each CRC. Returns the number of files tested.
pub fn test(path: &Path) -> Res<usize> {
    let mut count = 0;
    match Kind::detect(path).ok_or(tr("Unsupported archive format."))? {
        Kind::Zip => {
            let mut za =
                ZipArchive::new(BufReader::new(File::open(path).map_err(err)?)).map_err(err)?;
            for i in 0..za.len() {
                let mut f = za.by_index(i).map_err(err)?;
                if f.is_dir() {
                    continue;
                }
                let name = zname(&f);
                io::copy(&mut f, &mut io::sink()).map_err(|e| format!("{name}: {e}"))?;
                count += 1;
            }
        }
        Kind::SevenZ => {
            open_7z(path)?
                .for_each_entries(|e, rd| {
                    io::copy(rd, &mut io::sink())?;
                    count += !e.is_directory() as usize;
                    Ok(true)
                })
                .map_err(err)?;
        }
        Kind::Tar | Kind::Tgz => {
            let r: Box<dyn Read> = if Kind::detect(path) == Some(Kind::Tgz) {
                Box::new(GzDecoder::new(BufReader::new(
                    File::open(path).map_err(err)?,
                )))
            } else {
                Box::new(BufReader::new(File::open(path).map_err(err)?))
            };
            let mut ar = tar::Archive::new(r);
            for e in ar.entries().map_err(err)? {
                let mut e = e.map_err(err)?;
                if !e.header().entry_type().is_dir() {
                    io::copy(&mut e, &mut io::sink()).map_err(err)?;
                    count += 1;
                }
            }
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

fn zip_opts(level: u8, src: &Path) -> SimpleFileOptions {
    let base = SimpleFileOptions::default()
        .last_modified_time(zip_time(src))
        .large_file(true);
    if level == 0 {
        base.compression_method(CompressionMethod::Stored)
    } else {
        base.compression_method(CompressionMethod::Deflated)
            .compression_level(Some(deflate_level(level) as i64))
    }
}

fn write_zip_items<W: Write + Seek>(
    zw: &mut ZipWriter<W>,
    items: &[(PathBuf, String)],
    level: u8,
) -> Res<()> {
    for (src, name) in items {
        let o = zip_opts(level, src);
        if src.is_dir() {
            zw.add_directory(name.as_str(), o).map_err(err)?;
        } else {
            zw.start_file(name.as_str(), o).map_err(err)?;
            io::copy(&mut BufReader::new(File::open(src).map_err(err)?), zw).map_err(err)?;
        }
    }
    Ok(())
}

fn sz_writer<W: Write + Seek>(w: W, level: u8) -> Res<ArchiveWriter<W>> {
    let mut sw = ArchiveWriter::new(w).map_err(err)?;
    let method = if level == 0 {
        EncoderMethod::COPY.into()
    } else {
        EncoderConfiguration::new(EncoderMethod::LZMA2)
            .with_options(Lzma2Options::from_level(deflate_level(level)).into())
    };
    sw.set_content_methods(vec![method]);
    Ok(sw)
}

fn write_7z_items<W: Write + Seek>(
    sw: &mut ArchiveWriter<W>,
    items: &[(PathBuf, String)],
) -> Res<()> {
    for (src, name) in items {
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
        if src.is_dir() {
            b.append_dir(name, src).map_err(err)?;
        } else {
            b.append_path_with_name(src, name).map_err(err)?;
        }
    }
    Ok(())
}

pub fn create(out: &Path, kind: Kind, inputs: &[PathBuf], level: u8) -> Res<usize> {
    let items = gather(inputs, "");
    let tmp = out.with_extension("part");
    let res = (|| -> Res<()> {
        let f = BufWriter::new(File::create(&tmp).map_err(err)?);
        match kind {
            Kind::Zip => {
                let mut zw = ZipWriter::new(f);
                write_zip_items(&mut zw, &items, level)?;
                zw.finish().map_err(err)?.flush().map_err(err)?;
            }
            Kind::SevenZ => {
                let mut sw = sz_writer(f, level)?;
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
        }
        Ok(())
    })();
    finish_tmp(res, &tmp, out)?;
    Ok(items.iter().filter(|(p, _)| !p.is_dir()).count())
}

fn finish_tmp(res: Res<()>, tmp: &Path, out: &Path) -> Res<()> {
    match res {
        Ok(()) => fs::rename(tmp, out).map_err(err),
        Err(e) => {
            let _ = fs::remove_file(tmp);
            Err(e)
        }
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
) -> Res<()> {
    let kind = Kind::detect(path).ok_or(tr("Unsupported archive format."))?;
    let items = gather(add, prefix);
    let replaced: std::collections::HashSet<&str> = items.iter().map(|(_, n)| n.as_str()).collect();
    let keep = |p: &str| keep(p) && !replaced.contains(p);
    let tmp = path.with_extension("part");
    let res = (|| -> Res<()> {
        let f = BufWriter::new(File::create(&tmp).map_err(err)?);
        match kind {
            Kind::Zip => {
                let mut za =
                    ZipArchive::new(BufReader::new(File::open(path).map_err(err)?)).map_err(err)?;
                let mut zw = ZipWriter::new(f);
                for i in 0..za.len() {
                    let e = za.by_index_raw(i).map_err(err)?;
                    if keep(&zname(&e)) {
                        zw.raw_copy_file(e).map_err(err)?;
                    }
                }
                write_zip_items(&mut zw, &items, level)?;
                zw.finish().map_err(err)?.flush().map_err(err)?;
            }
            Kind::SevenZ => {
                // ponytail: re-compresses every kept file; fine for normal sizes, slow for huge archives.
                let mut sw = sz_writer(f, level)?;
                let mut fail = None;
                open_7z(path)?
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
                    .map_err(err)?;
                if let Some(f) = fail {
                    return Err(f);
                }
                write_7z_items(&mut sw, &items)?;
                sw.finish().map_err(err)?.flush().map_err(err)?;
            }
            Kind::Tar => {
                let mut b = tar::Builder::new(f);
                copy_tar(
                    tar::Archive::new(BufReader::new(File::open(path).map_err(err)?)),
                    &mut b,
                    &keep,
                )?;
                write_tar_items(&mut b, &items)?;
                b.into_inner().map_err(err)?.flush().map_err(err)?;
            }
            Kind::Tgz => {
                let mut b =
                    tar::Builder::new(GzEncoder::new(f, Compression::new(deflate_level(level))));
                copy_tar(
                    tar::Archive::new(GzDecoder::new(BufReader::new(
                        File::open(path).map_err(err)?,
                    ))),
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

pub fn add(path: &Path, inputs: &[PathBuf], prefix: &str, level: u8) -> Res<()> {
    rewrite(path, &|_| true, inputs, prefix, level)
}

pub fn delete(path: &Path, sel: &[String]) -> Res<()> {
    let hit = |p: &str| {
        sel.iter()
            .any(|s| p == s || p.starts_with(&format!("{s}/")))
    };
    rewrite(path, &|p| !hit(p), &[], "", 2)
}

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
            assert_eq!(create(&ar, kind, &[src.clone()], 2).unwrap(), 2);
            assert_eq!(Kind::detect(&ar), Some(kind));
            add(&ar, &[dir.join("extra.txt")], "src/sub", 2).unwrap();
            delete(&ar, &["src/a.txt".into()]).unwrap();
            let (_, list) = list(&ar).unwrap();
            let names: Vec<_> = list.iter().map(|e| e.path.as_str()).collect();
            assert!(
                names.contains(&"src/sub/extra.txt") && !names.contains(&"src/a.txt"),
                "{kind:?} {names:?}"
            );
            assert_eq!(test(&ar).unwrap(), 2);
            let out = dir.join(format!("out-{}", kind.ext()));
            extract(&ar, &out, Some(&["src/sub".into()])).unwrap();
            assert_eq!(fs::read(out.join("src/sub/b.txt")).unwrap(), b"world");
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
