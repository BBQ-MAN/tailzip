// Tests against real archives in tests/fixtures (see tests/fixtures/make.sh and
// THIRD_PARTY_NOTICES.md for where each one comes from).
use super::*;

const PW: &str = "Test1234";
/// Password of the upstream ALZ/EGG samples.
const PW_AE: &str = "test1234";

fn fx(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn cache(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/cache")
        .join(name)
}

fn tmp(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "tailzip-fx-{}-{tag}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn hello() -> String {
    (1..=20)
        .map(|i| format!("Hello from tailzip {i}\n"))
        .collect()
}

fn notes() -> String {
    (1..=10)
        .map(|i| format!("암호 테스트 / password test {i}\n"))
        .collect()
}

/// List, test and extract with `pw`, then compare the files make.sh put in.
fn check_hello(path: &Path, pw: Option<&str>, with_notes: bool) {
    let (_, entries) = list(path, pw).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    assert!(entries.iter().any(|e| e.path == "hello.txt"), "{path:?}");
    let files = entries.iter().filter(|e| !e.is_dir).count();
    assert_eq!(
        test(path, pw).unwrap_or_else(|e| panic!("{path:?}: {e}")),
        files
    );
    let out = tmp("hello");
    assert_eq!(extract(path, &out, None, pw).unwrap(), files);
    assert_eq!(fs::read_to_string(out.join("hello.txt")).unwrap(), hello());
    if with_notes {
        assert_eq!(
            fs::read_to_string(out.join("sub/notes.txt")).unwrap(),
            notes()
        );
    }
    let _ = fs::remove_dir_all(&out);
}

/// Without a password: NeedPassword; wrong one: WrongPassword; right one: works.
fn check_passwords(path: &Path, right: &str, list_needs_pw: bool) {
    let out = tmp("pw");
    if list_needs_pw {
        assert_eq!(
            list(path, None).unwrap_err(),
            Error::NeedPassword,
            "{path:?}"
        );
        assert_eq!(
            list(path, Some("nope")).unwrap_err(),
            Error::WrongPassword,
            "{path:?}"
        );
    } else {
        let (_, entries) = list(path, None).unwrap();
        assert!(
            entries.iter().any(|e| e.encrypted),
            "{path:?} not marked encrypted"
        );
        assert_eq!(
            extract(path, &out, None, None).unwrap_err(),
            Error::NeedPassword,
            "{path:?}"
        );
        assert_eq!(
            test(path, None).unwrap_err(),
            Error::NeedPassword,
            "{path:?}"
        );
    }
    assert_eq!(
        extract(path, &out, None, Some("wrong-password")).unwrap_err(),
        Error::WrongPassword,
        "{path:?}"
    );
    assert_eq!(
        test(path, Some("wrong-password")).unwrap_err(),
        Error::WrongPassword,
        "{path:?}"
    );
    assert!(test(path, Some(right)).is_ok(), "{path:?}");
    let _ = fs::remove_dir_all(&out);
}

/// Extract and compare (name, length, CRC-32) against the upstream sample tables.
fn check_crcs(path: &Path, pw: Option<&str>, want: &[(&str, u64, u32)]) {
    let out = tmp("crc");
    let n = extract(path, &out, None, pw).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    assert_eq!(n, want.len(), "{path:?}");
    for (name, len, crc) in want {
        let got = fs::read(out.join(name)).unwrap_or_else(|_| panic!("{path:?}: no {name}"));
        assert_eq!(got.len() as u64, *len, "{path:?} {name}");
        assert_eq!(crc32fast::hash(&got), *crc, "{path:?} {name}");
    }
    assert_eq!(test(path, pw).unwrap(), want.len(), "{path:?}");
    let _ = fs::remove_dir_all(&out);
}

const FIVE: &[(&str, u64, u32)] = &[
    ("empty.txt", 0, 0x00000000),
    ("precompressed.gz", 95, 0x6a94d1bc),
    ("prog.sys", 2060, 0xb54ccab2),
    ("random.bin", 4096, 0x1bb92dfc),
    ("text.txt", 3150, 0x7a553a9d),
];
const AB: &[(&str, u64, u32)] = &[("a.txt", 3150, 0x7a553a9d), ("b.bin", 512, 0x7735137b)];
const PARTS: &[(&str, u64, u32)] = &[
    ("part0.bin", 40000, 0xcee0afee),
    ("part1.bin", 40000, 0x00bd73ad),
    ("part2.bin", 40000, 0x18b848e0),
    ("part3.bin", 40000, 0xd0333732),
];

#[test]
fn detect_by_signature() {
    for (f, k) in [
        ("zip_aes256.zip", Kind::Zip),
        ("zipx_ppmd.zipx", Kind::Zip),
        ("7z_aes_hdr.7z", Kind::SevenZ),
        ("vol.7z.003", Kind::SevenZ),
        ("vol.zip.002", Kind::Zip),
        ("rar5.rar", Kind::Rar),
        ("rar5_vol.part2.rar", Kind::Rar),
        ("rar4/version.rar", Kind::Rar),
        ("alz/deflate_pure.alz", Kind::Alz),
        ("alz/split_store.a01", Kind::Alz),
        ("egg/azo.egg", Kind::Egg),
    ] {
        assert_eq!(Kind::detect(&fx(f)), Some(k), "{f}");
    }
}

#[test]
fn zip_aes_and_zipcrypto() {
    for f in [
        "zip_aes128.zip",
        "zip_aes192.zip",
        "zip_aes256.zip",
        "zip_zipcrypto.zip",
    ] {
        check_passwords(&fx(f), PW, false);
        check_hello(&fx(f), Some(PW), true);
    }
}

#[test]
fn zipx_methods() {
    for f in [
        "zipx_ppmd.zipx",
        "zipx_lzma.zipx",
        "zipx_bzip2.zipx",
        "zipx_deflate64.zipx",
    ] {
        check_hello(&fx(f), None, true);
    }
}

#[test]
fn unsupported_zip_method_names_the_file() {
    // Rewrite the method field of a stored entry to 97 (WavPack) in both headers.
    let dir = tmp("wavpack");
    let src = dir.join("in.txt");
    fs::write(&src, "plain").unwrap();
    let z = dir.join("w.zip");
    create(&z, Kind::Zip, &[src], 0, &NewOpts::default()).unwrap();
    let mut b = fs::read(&z).unwrap();
    for sig in [&b"PK\x03\x04"[..], &b"PK\x01\x02"[..]] {
        let at = b.windows(4).position(|w| w == sig).unwrap();
        let off = if sig[2] == 3 { 8 } else { 10 };
        b[at + off] = 97;
    }
    fs::write(&z, &b).unwrap();
    let e = extract(&z, &dir.join("o"), None, None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("in.txt"), "{e}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn sevenz_aes() {
    check_passwords(&fx("7z_aes.7z"), PW, false);
    check_hello(&fx("7z_aes.7z"), Some(PW), true);
    check_passwords(&fx("7z_aes_hdr.7z"), PW, true);
    check_hello(&fx("7z_aes_hdr.7z"), Some(PW), true);
}

#[test]
fn raw_volumes_from_a_middle_volume() {
    for f in ["vol.7z.002", "vol.zip.003"] {
        check_hello(&fx(f), None, false);
        let (_, e) = list(&fx(f), None).unwrap();
        assert!(e.iter().any(|e| e.path == "noise.bin" && e.size == 3000));
        assert!(edit_block(&fx(f), Kind::detect(&fx(f)).unwrap()).is_some());
    }
}

#[test]
fn missing_volumes_are_named() {
    let dir = tmp("missing");
    for f in [
        "vol.7z.001",
        "vol.7z.002",
        "vol.7z.003",
        "vol.zip.001",
        "vol.zip.002",
    ] {
        fs::copy(fx(f), dir.join(f)).unwrap();
    }
    let e = list(&dir.join("vol.7z.001"), None).unwrap_err().to_string();
    assert!(e.contains("vol.7z.004"), "{e}");
    let e = list(&dir.join("vol.zip.002"), None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("vol.zip.003"), "{e}");
    // RAR: only the first part.
    fs::copy(fx("rar5_vol.part1.rar"), dir.join("rar5_vol.part1.rar")).unwrap();
    let e = test(&dir.join("rar5_vol.part1.rar"), None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("rar5_vol.part2.rar"), "{e}");
    // ALZ: .a01 missing.
    for f in ["split_store.alz", "split_store.a00"] {
        fs::copy(fx(&format!("alz/{f}")), dir.join(f)).unwrap();
    }
    let e = list(&dir.join("split_store.a00"), None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("split_store.a01"), "{e}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn rar_read_only() {
    check_hello(&fx("rar5.rar"), None, true);
    check_passwords(&fx("rar5_enc.rar"), PW, false);
    check_hello(&fx("rar5_enc.rar"), Some(PW), true);
    check_passwords(&fx("rar5_hdr.rar"), PW, true);
    check_hello(&fx("rar5_hdr.rar"), Some(PW), true);
    // Volumes, opened from the second part.
    check_hello(&fx("rar5_vol.part2.rar"), None, false);
    // RAR4 (from the unrar crate's test data).
    let (_, e) = list(&fx("rar4/version.rar"), None).unwrap();
    assert_eq!(e.len(), 1);
    assert_eq!(test(&fx("rar4/version.rar"), None).unwrap(), 1);
    check_passwords(&fx("rar4/crypted.rar"), "unrar", false);
    let out = tmp("rar4");
    assert!(extract(&fx("rar4/crypted.rar"), &out, None, Some("unrar")).unwrap() >= 1);
    // Read-only: no add/delete.
    let copy = out.join("r.rar");
    fs::copy(fx("rar5.rar"), &copy).unwrap();
    assert!(delete(&copy, &["hello.txt".into()], None).is_err());
    assert!(add(&copy, &[fx("make.sh")], "", 2, None).is_err());
    assert_eq!(list(&copy, None).unwrap().1.len(), 3);
    let _ = fs::remove_dir_all(&out);
}

#[test]
fn alz_samples() {
    check_crcs(
        &fx("alz/deflate_pure.alz"),
        None,
        &[("text.txt", 9000, 0x1859c604)],
    );
    check_crcs(
        &fx("alz/bzip2_dlz.alz"),
        None,
        &[("text.txt", 3600, 0x2c19aec0)],
    );
    check_crcs(&fx("alz/enc_deflate.alz"), Some(PW_AE), AB);
    check_passwords(&fx("alz/enc_deflate.alz"), PW_AE, false);
    // Split store archive, opened from a middle volume.
    check_crcs(&fx("alz/split_store.a00"), None, PARTS);
    let (k, e) = list(&fx("alz/split_store.alz"), None).unwrap();
    assert_eq!((k, e.len()), (Kind::Alz, 4));
    assert!(e.iter().all(|e| e.method == "Store"));
}

#[test]
fn egg_samples() {
    check_crcs(&fx("egg/deflate5.egg"), None, FIVE);
    check_crcs(&fx("egg/lzma5.egg"), None, FIVE);
    check_crcs(
        &fx("egg/azo.egg"),
        None,
        &[("azo_fix.sys", 1602, 0x4a2ab0d2)],
    );
    check_crcs(&fx("egg/empty_noblock.egg"), None, &[("empty.txt", 0, 0)]);
    check_crcs(
        &fx("egg/solid.egg"),
        None,
        &[
            ("a", 1, 0xe8b7be43),
            ("empty.txt", 0, 0x00000000),
            ("hello.txt", 45, 0xeb50cc6a),
            ("lorem.txt", 2200, 0xcc0c6d8d),
            ("random.bin", 4096, 0xb4c1d904),
        ],
    );
    let (_, e) = list(&fx("egg/azo.egg"), None).unwrap();
    assert_eq!(e[0].method, "AZO");
    for (f, want) in [
        ("egg/enc_aes128.egg", AB),
        ("egg/enc_aes256.egg", AB),
        ("egg/enc_zip20.egg", AB),
        (
            "egg/enc_aes256_bzip2.egg",
            &[("text.txt", 9000, 0x1859c604)][..],
        ),
        (
            "egg/enc_zip20_bzip2.egg",
            &[("text.txt", 9000, 0x1859c604)][..],
        ),
        (
            "egg/enc_lea_bzip2.egg",
            &[("text.txt", 3600, 0x2c19aec0)][..],
        ),
    ] {
        check_crcs(&fx(f), Some(PW_AE), want);
        check_passwords(&fx(f), PW_AE, false);
    }
}

/// Larger upstream samples, downloaded to tests/cache (gitignored); skipped when absent.
#[test]
fn cached_large_samples() {
    if !cache("egg/split_store.vol2.egg").is_file() {
        eprintln!("tests/cache not present, skipping");
        return;
    }
    check_crcs(&cache("egg/split_store.vol2.egg"), None, PARTS);
    for f in [
        "egg/aes128.egg",
        "egg/aes256.egg",
        "egg/lea128.egg",
        "egg/lea256.egg",
        "egg/zip20.egg",
    ] {
        assert!(test(&cache(f), Some(PW_AE)).unwrap() > 0, "{f}");
        assert_eq!(
            test(&cache(f), Some("bad")).unwrap_err(),
            Error::WrongPassword,
            "{f}"
        );
    }
    assert_eq!(
        list(&cache("egg/split.vol4.egg"), None).unwrap().0,
        Kind::Egg
    );
    assert!(test(&cache("egg/split.vol1.egg"), None).unwrap() > 0);
    assert!(test(&cache("alz/split.a03"), None).unwrap() > 0);
    check_crcs(
        &cache("egg/bzip2_big.egg"),
        None,
        &[("huge.txt", 8100000, 0xf4710e9f)],
    );
    check_crcs(
        &cache("egg/azo_pe.egg"),
        None,
        &[("hello.sys", 148781, 0xb9a7aa7c)],
    );
}

/// Source tree with hello.txt, sub/notes.txt and 5000 incompressible bytes.
fn sample_src(dir: &Path) -> Vec<PathBuf> {
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("hello.txt"), hello()).unwrap();
    fs::write(dir.join("sub/notes.txt"), notes()).unwrap();
    let mut x = 0x1234_5678u32;
    let noise: Vec<u8> = (0..5000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    fs::write(dir.join("noise.bin"), noise).unwrap();
    vec![
        dir.join("hello.txt"),
        dir.join("sub"),
        dir.join("noise.bin"),
    ]
}

#[test]
fn create_encrypted_and_reopen() {
    let dir = tmp("create");
    let inputs = sample_src(&dir.join("src"));
    let opts = |zip_crypto, encrypt_names| NewOpts {
        password: Some(PW.into()),
        zip_crypto,
        encrypt_names,
        volume_size: None,
    };
    for (name, kind, o, hidden) in [
        ("aes.zip", Kind::Zip, opts(false, false), false),
        ("zc.zip", Kind::Zip, opts(true, false), false),
        ("aes.7z", Kind::SevenZ, opts(false, false), false),
        ("hdr.7z", Kind::SevenZ, opts(false, true), true),
    ] {
        let p = dir.join(name);
        assert_eq!(create(&p, kind, &inputs, 2, &o).unwrap(), 3);
        check_passwords(&p, PW, hidden);
        check_hello(&p, Some(PW), true);
    }
    // ZIP AES is really AES (method 99 in the local header).
    let b = fs::read(dir.join("aes.zip")).unwrap();
    assert_eq!(u16_at(&b, 8), 99);
    // Adding to an encrypted 7z keeps it encrypted, file list included.
    let hdr = dir.join("hdr.7z");
    add(&hdr, &[fx("make.sh")], "", 2, Some(PW)).unwrap();
    assert_eq!(list(&hdr, None).unwrap_err(), Error::NeedPassword);
    let (_, e) = list(&hdr, Some(PW)).unwrap();
    assert!(e.iter().any(|e| e.path == "make.sh"));
    delete(&hdr, &["sub".into()], Some(PW)).unwrap();
    assert!(!list(&hdr, Some(PW))
        .unwrap()
        .1
        .iter()
        .any(|e| e.path.starts_with("sub")));
    // Adding to an AES ZIP with the password encrypts the new file too.
    let z = dir.join("aes.zip");
    add(&z, &[fx("make.sh")], "", 2, Some(PW)).unwrap();
    assert!(list(&z, None)
        .unwrap()
        .1
        .iter()
        .filter(|e| !e.is_dir)
        .all(|e| e.encrypted));
    assert_eq!(test(&z, Some(PW)).unwrap(), 4);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn create_split_and_reopen() {
    let dir = tmp("split");
    let inputs = sample_src(&dir.join("src"));
    for (name, kind) in [("s.7z", Kind::SevenZ), ("s.zip", Kind::Zip)] {
        let out = dir.join(name);
        let o = NewOpts {
            volume_size: Some(1500),
            ..NewOpts::default()
        };
        create(&out, kind, &inputs, 2, &o).unwrap();
        assert!(!out.exists());
        let vols = volumes(&volume_path(&out, 1)).unwrap();
        assert!(vols.len() >= 4, "{vols:?}");
        assert!(vols.iter().all(|v| fs::metadata(v).unwrap().len() <= 1500));
        check_hello(&volume_path(&out, 2), None, true);
        let out2 = dir.join(format!("x-{name}"));
        extract(&volume_path(&out, 0), &out2, None, None).unwrap();
        assert_eq!(
            fs::read(out2.join("noise.bin")).unwrap(),
            fs::read(&inputs[2]).unwrap()
        );
        // Split archives refuse changes.
        let v1 = volume_path(&out, 0);
        assert!(add(&v1, &[fx("make.sh")], "", 2, None).is_err());
        assert!(delete(&v1, &["hello.txt".into()], None).is_err());
    }
    // An encrypted split 7z with an encrypted file list.
    let out = dir.join("e.7z");
    let o = NewOpts {
        password: Some(PW.into()),
        encrypt_names: true,
        volume_size: Some(2000),
        ..NewOpts::default()
    };
    create(&out, Kind::SevenZ, &inputs, 2, &o).unwrap();
    check_passwords(&volume_path(&out, 1), PW, true);
    check_hello(&volume_path(&out, 1), Some(PW), true);
    let _ = fs::remove_dir_all(&dir);
}

/// Info-ZIP split archives (name.z01 ... name.zip). Needs `zip`; skipped when it is missing.
#[test]
fn info_zip_split() {
    let dir = tmp("z01");
    sample_src(&dir.join("src"));
    // Two more 40 KB incompressible files: Info-ZIP's smallest volume is 64 KB.
    for n in ["big1.bin", "big2.bin"] {
        let mut x = n.len() as u32 * 7919 + n.as_bytes()[3] as u32;
        let v: Vec<u8> = (0..40_000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (x >> 16) as u8
            })
            .collect();
        fs::write(dir.join("src").join(n), v).unwrap();
    }
    let ok = std::process::Command::new("zip")
        .current_dir(dir.join("src"))
        .args(["-q", "-r", "-0", "-s", "64k", "../sp.zip", "."])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("Info-ZIP `zip` not available, skipping");
        return;
    }
    assert!(dir.join("sp.z01").is_file());
    let vols = volumes(&dir.join("sp.z01")).unwrap();
    assert_eq!(vols.last().unwrap(), &dir.join("sp.zip"));
    check_hello(&dir.join("sp.z01"), None, true);
    let out = dir.join("out");
    extract(&dir.join("sp.zip"), &out, None, None).unwrap();
    for n in ["big1.bin", "big2.bin", "noise.bin"] {
        assert_eq!(
            fs::read(out.join(n)).unwrap(),
            fs::read(dir.join("src").join(n)).unwrap()
        );
    }
    assert!(edit_block(&dir.join("sp.zip"), Kind::Zip).is_some());
    // The .zip alone names the missing first volume.
    fs::rename(dir.join("sp.z01"), dir.join("moved.z01")).unwrap();
    let e = list(&dir.join("sp.zip"), None).unwrap_err().to_string();
    assert!(e.contains("sp.z01"), "{e}");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn path_traversal_is_refused() {
    let dir = tmp("slip");
    let dest = dir.join("dest");
    // ZIP
    let z = dir.join("evil.zip");
    {
        let mut w = ZipWriter::new(File::create(&z).unwrap());
        for n in ["../evil.txt", "/abs.txt", "ok.txt", "a/../../evil2.txt"] {
            w.start_file(n, FileOptions::<'static, 'static, ()>::default())
                .unwrap();
            w.write_all(b"x").unwrap();
        }
        w.finish().unwrap();
    }
    assert_eq!(extract(&z, &dest, None, None).unwrap(), 1);
    // 7z
    let s = dir.join("evil.7z");
    {
        let mut w = ArchiveWriter::create(&s).unwrap();
        for n in ["../evil.txt", "ok7.txt"] {
            w.push_archive_entry(ArchiveEntry::new_file(n), Some(&b"x"[..]))
                .unwrap();
        }
        w.finish().unwrap();
    }
    assert_eq!(extract(&s, &dest, None, None).unwrap(), 1);
    // TAR with a raw "../" name and symlinks pointing out of the tree.
    let t = dir.join("evil.tar");
    {
        let mut b = tar::Builder::new(File::create(&t).unwrap());
        let mut h = tar::Header::new_gnu();
        h.as_old_mut().name[..10].copy_from_slice(b"../evil.tx");
        h.set_size(1);
        h.set_cksum();
        b.append(&h, &b"x"[..]).unwrap();
        for (name, target) in [("up", "../../outside"), ("abs", "/etc")] {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            b.append_link(&mut h, name, target).unwrap();
        }
        let mut h = tar::Header::new_gnu();
        h.set_size(1);
        b.append_data(&mut h, "okt.txt", &b"x"[..]).unwrap();
        b.finish().unwrap();
    }
    assert_eq!(extract(&t, &dest, None, None).unwrap(), 1);
    let mut names: Vec<_> = fs::read_dir(&dest)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["ok.txt", "ok7.txt", "okt.txt"]);
    for n in ["evil.txt", "evil2.txt", "evil.tx"] {
        assert!(!dir.join(n).exists() && !std::env::temp_dir().join(n).exists());
    }
    let _ = fs::remove_dir_all(&dir);
}

/// Every prefix (and some corrupted copies) of every fixture: errors are fine, panics are not.
#[test]
fn truncated_and_garbage_input_never_panics() {
    let dir = tmp("fuzz");
    let mut files = Vec::new();
    for sub in ["", "alz", "egg", "rar4"] {
        for e in fs::read_dir(fx(sub)).unwrap() {
            let p = e.unwrap().path();
            if p.is_file() && p.extension().is_some_and(|x| x != "sh") {
                files.push(p);
            }
        }
    }
    assert!(files.len() > 30);
    for f in files {
        let data = fs::read(&f).unwrap();
        let p = dir.join(f.file_name().unwrap());
        let mut cases: Vec<Vec<u8>> = (0..data.len())
            .step_by((data.len() / 24).max(1))
            .map(|n| data[..n].to_vec())
            .collect();
        for i in (0..data.len()).step_by((data.len() / 6).max(1)) {
            let mut d = data.clone();
            d[i] ^= 0x5A;
            cases.push(d);
        }
        for d in cases {
            fs::write(&p, &d).unwrap();
            for pw in [None, Some(PW), Some(PW_AE)] {
                let _ = list(&p, pw);
                let _ = test(&p, pw);
            }
        }
        let _ = fs::remove_file(&p);
    }
    // Signatures followed by junk.
    for (n, sig) in [
        ("g.zip", &b"PK\x03\x04"[..]),
        ("g.7z", &[b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C][..]),
        ("g.rar", &b"Rar!\x1a\x07\x01\x00"[..]),
        ("g.alz", &b"ALZ\x01"[..]),
        ("g.egg", &b"EGGA"[..]),
        ("g.tgz", &[0x1f, 0x8b][..]),
    ] {
        let mut d = sig.to_vec();
        d.extend((0..3000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8));
        let p = dir.join(n);
        fs::write(&p, &d).unwrap();
        let _ = list(&p, None);
        let _ = test(&p, None);
    }
    let _ = fs::remove_dir_all(&dir);
}

/// XZ and Zstandard entries (written by the zip crate itself: no tool here makes them).
#[test]
fn zip_xz_and_zstd_entries() {
    let dir = tmp("xz");
    let z = dir.join("m.zipx");
    {
        let mut w = ZipWriter::new(File::create(&z).unwrap());
        for (n, m) in [
            ("x.txt", CompressionMethod::Xz),
            ("z.txt", CompressionMethod::Zstd),
        ] {
            let o = FileOptions::<'static, 'static, ()>::default().compression_method(m);
            w.start_file(n, o).unwrap();
            w.write_all(hello().as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }
    let (_, e) = list(&z, None).unwrap();
    assert_eq!(
        e.iter().map(|e| e.method.as_str()).collect::<Vec<_>>(),
        ["Xz", "Zstd"]
    );
    assert_eq!(test(&z, None).unwrap(), 2);
    extract(&z, &dir.join("o"), None, None).unwrap();
    assert_eq!(fs::read_to_string(dir.join("o/z.txt")).unwrap(), hello());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn tar_raw_volumes() {
    let dir = tmp("tarvol");
    let inputs = sample_src(&dir.join("src"));
    let out = dir.join("t.tar");
    let o = NewOpts {
        volume_size: Some(4096),
        ..NewOpts::default()
    };
    create(&out, Kind::Tar, &inputs, 0, &o).unwrap();
    assert_eq!(Kind::detect(&volume_path(&out, 2)), Some(Kind::Tar));
    check_hello(&volume_path(&out, 1), None, true);
    let _ = fs::remove_dir_all(&dir);
}
