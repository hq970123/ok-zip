//! Round-trip tests: create → list → test → extract → compare, for every
//! writable format, with and without passwords, split volumes and edits.

use okzip_core::*;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Build a small but awkward source tree. Returns the folder to compress.
fn make_tree(root: &Path) -> PathBuf {
    let src = root.join("项目 data");
    fs::create_dir_all(src.join("sub/deeper")).unwrap();
    fs::create_dir_all(src.join("empty_dir")).unwrap();
    fs::write(src.join("hello.txt"), b"hello, OK Zip!\n".repeat(200)).unwrap();
    fs::write(src.join("中文 文件.txt"), "压缩测试 unicode ✓".repeat(50)).unwrap();
    fs::write(src.join("empty.bin"), b"").unwrap();
    // pseudo-random, poorly compressible 300 KiB
    let mut x: u32 = 0x1234_5678;
    let noise: Vec<u8> = (0..300 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x & 0xFF) as u8
        })
        .collect();
    fs::write(src.join("sub/noise.bin"), &noise).unwrap();
    fs::write(src.join("sub/deeper/z.txt"), b"z").unwrap();
    src
}

/// Map of relative path → contents (None for dirs).
fn snapshot(dir: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut m = BTreeMap::new();
    for e in walkdir(dir) {
        let rel = e.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        if e.is_dir() {
            m.insert(rel, None);
        } else {
            m.insert(rel, Some(fs::read(&e).unwrap()));
        }
    }
    m
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![dir.to_path_buf()];
    let mut i = 0;
    while i < out.len() {
        let p = out[i].clone();
        if p.is_dir() {
            let mut kids: Vec<PathBuf> = fs::read_dir(&p).unwrap().map(|e| e.unwrap().path()).collect();
            kids.sort();
            out.extend(kids);
        }
        i += 1;
    }
    out
}

fn roundtrip(opts: CreateOptions, file_name: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let src = make_tree(tmp.path());
    let archive = tmp.path().join(file_name);
    let calls = Arc::new(AtomicU64::new(0));
    let c2 = calls.clone();
    let p = Progress::with_callback(move |_| {
        c2.fetch_add(1, Ordering::Relaxed);
    });
    let stats = create(std::slice::from_ref(&src), &archive, &opts, &p).unwrap();
    assert!(calls.load(Ordering::Relaxed) > 0, "progress callback fired");
    assert_eq!(stats.files, 5);

    // Volumes?
    let open_path = if opts.split_size.is_some() {
        assert!(stats.outputs.len() > 1, "split produced several volumes");
        assert!(!archive.exists());
        stats.outputs[0].clone()
    } else {
        archive.clone()
    };

    assert_eq!(detect_format(&open_path).unwrap(), opts.format);
    let pw = opts.password.clone();
    let info = list(&open_path, &OpenOptions { password: pw.clone() }, &Progress::new()).unwrap();
    assert_eq!(info.format, opts.format);
    assert_eq!(info.file_count(), 5, "entries: {:?}", info.entries.iter().map(|e| &e.path).collect::<Vec<_>>());
    assert!(info.entries.iter().any(|e| e.path == "项目 data/中文 文件.txt"));
    assert!(info.entries.iter().any(|e| e.path == "项目 data/empty_dir" && e.is_dir));
    assert_eq!(info.encrypted, pw.is_some());
    if opts.split_size.is_some() {
        assert!(info.volumes > 1);
    }

    let report = okzip_core::test(&open_path, pw.as_deref(), &Progress::new()).unwrap();
    assert!(report.ok(), "test errors: {:?}", report.errors);
    assert!(report.files_tested >= 4);

    let out = tmp.path().join("out");
    let xs = extract(&open_path, &out, &ExtractOptions::new().password(pw.clone()), &Progress::new()).unwrap();
    assert_eq!(xs.files, 5);
    assert_eq!(snapshot(&src), snapshot(&out.join("项目 data")));

    // mtime preserved (to the format's resolution)
    let a = fs::metadata(src.join("hello.txt")).unwrap().modified().unwrap();
    let b = fs::metadata(out.join("项目 data/hello.txt")).unwrap().modified().unwrap();
    let diff = a.duration_since(b).unwrap_or_else(|e| e.duration());
    assert!(diff.as_secs() <= 2, "mtime drift {diff:?}");

    if let Some(good) = pw {
        // Missing and wrong passwords are reported distinctly.
        let out2 = tmp.path().join("out2");
        let e = extract(&open_path, &out2, &ExtractOptions::new(), &Progress::new()).unwrap_err();
        assert!(matches!(e, Error::PasswordRequired), "got {e:?}");
        let e = extract(&open_path, &out2, &ExtractOptions::new().password(Some(format!("{good}x"))), &Progress::new())
            .unwrap_err();
        assert!(e.needs_password(), "got {e:?}");
    }
}

#[test]
fn zip_roundtrip() {
    roundtrip(CreateOptions::new(ArchiveFormat::Zip), "a.zip");
}

#[test]
fn zip_store_roundtrip() {
    roundtrip(CreateOptions { level: 0, ..CreateOptions::new(ArchiveFormat::Zip) }, "a.zip");
}

#[test]
fn zip_aes_roundtrip() {
    roundtrip(CreateOptions { password: Some("p@ss 密码".into()), ..CreateOptions::new(ArchiveFormat::Zip) }, "a.zip");
}

#[test]
fn zip_zipcrypto_roundtrip() {
    roundtrip(
        CreateOptions {
            password: Some("legacy".into()),
            zip_encryption: ZipEncryption::ZipCrypto,
            ..CreateOptions::new(ArchiveFormat::Zip)
        },
        "a.zip",
    );
}

#[test]
fn zip_split_roundtrip() {
    roundtrip(CreateOptions { level: 0, split_size: Some(100 * 1024), ..CreateOptions::new(ArchiveFormat::Zip) }, "s.zip");
}

#[test]
fn sevenz_roundtrip() {
    roundtrip(CreateOptions::new(ArchiveFormat::SevenZ), "a.7z");
}

#[test]
fn sevenz_non_solid_store_roundtrip() {
    roundtrip(CreateOptions { level: 0, solid: false, ..CreateOptions::new(ArchiveFormat::SevenZ) }, "a.7z");
}

#[test]
fn sevenz_aes_roundtrip() {
    roundtrip(CreateOptions { password: Some("secret".into()), level: 3, ..CreateOptions::new(ArchiveFormat::SevenZ) }, "a.7z");
}

#[test]
fn sevenz_encrypted_headers() {
    let tmp = tempfile::tempdir().unwrap();
    let src = make_tree(tmp.path());
    let archive = tmp.path().join("h.7z");
    let opts = CreateOptions {
        password: Some("hdr".into()),
        encrypt_headers: true,
        level: 1,
        ..CreateOptions::new(ArchiveFormat::SevenZ)
    };
    create(&[src], &archive, &opts, &Progress::new()).unwrap();
    let e = list(&archive, &OpenOptions::default(), &Progress::new()).unwrap_err();
    assert!(e.needs_password(), "listing without password must ask for one, got {e:?}");
    let info = list(&archive, &OpenOptions { password: Some("hdr".into()) }, &Progress::new()).unwrap();
    assert_eq!(info.file_count(), 5);
}

#[test]
fn sevenz_split_roundtrip() {
    roundtrip(CreateOptions { level: 1, split_size: Some(64 * 1024), ..CreateOptions::new(ArchiveFormat::SevenZ) }, "s.7z");
}

#[test]
fn tar_roundtrip() {
    roundtrip(CreateOptions::new(ArchiveFormat::Tar), "a.tar");
}

#[test]
fn tgz_roundtrip() {
    roundtrip(CreateOptions::new(ArchiveFormat::TarGz), "a.tgz");
}

#[test]
fn tbz2_roundtrip() {
    roundtrip(CreateOptions::new(ArchiveFormat::TarBz2), "a.tar.bz2");
}

#[test]
fn txz_roundtrip() {
    roundtrip(CreateOptions::new(ArchiveFormat::TarXz), "a.txz");
}

#[test]
fn tzst_roundtrip() {
    roundtrip(CreateOptions::new(ArchiveFormat::TarZst), "a.tar.zst");
}

#[test]
fn tar_family_detected_without_extension() {
    let tmp = tempfile::tempdir().unwrap();
    let src = make_tree(tmp.path());
    for fmt in [ArchiveFormat::TarGz, ArchiveFormat::TarXz, ArchiveFormat::TarZst, ArchiveFormat::TarBz2] {
        let a = tmp.path().join(format!("noext-{}", fmt.extension().replace('.', "-")));
        create(std::slice::from_ref(&src), &a, &CreateOptions::new(fmt), &Progress::new()).unwrap();
        assert_eq!(detect_format(&a).unwrap(), fmt);
    }
}

#[test]
fn single_stream_formats() {
    let tmp = tempfile::tempdir().unwrap();
    let data = b"single stream payload ".repeat(1000);
    // gz with embedded file name
    let gz = tmp.path().join("payload.txt.gz");
    {
        let f = fs::File::create(&gz).unwrap();
        let mut e = flate2::GzBuilder::new().filename("payload.txt").write(f, flate2::Compression::default());
        e.write_all(&data).unwrap();
        e.finish().unwrap();
    }
    let xz = tmp.path().join("payload.txt.xz");
    {
        let f = fs::File::create(&xz).unwrap();
        let mut e = liblzma::write::XzEncoder::new(f, 6);
        e.write_all(&data).unwrap();
        e.finish().unwrap();
    }
    let zst = tmp.path().join("payload.txt.zst");
    fs::write(&zst, zstd::encode_all(&data[..], 3).unwrap()).unwrap();
    let bz = tmp.path().join("payload.txt.bz2");
    {
        let f = fs::File::create(&bz).unwrap();
        let mut e = bzip2::write::BzEncoder::new(f, bzip2::Compression::default());
        e.write_all(&data).unwrap();
        e.finish().unwrap();
    }
    for (path, fmt) in [(&gz, ArchiveFormat::Gz), (&xz, ArchiveFormat::Xz), (&zst, ArchiveFormat::Zst), (&bz, ArchiveFormat::Bz2)] {
        assert_eq!(detect_format(path).unwrap(), fmt);
        let info = list(path, &OpenOptions::default(), &Progress::new()).unwrap();
        assert_eq!(info.entries.len(), 1);
        assert_eq!(info.entries[0].path, "payload.txt");
        if fmt == ArchiveFormat::Gz {
            assert_eq!(info.entries[0].size, data.len() as u64);
        }
        let out = tmp.path().join(format!("out-{}", fmt.extension()));
        extract(path, &out, &ExtractOptions::new(), &Progress::new()).unwrap();
        assert_eq!(fs::read(out.join("payload.txt")).unwrap(), data);
        assert!(okzip_core::test(path, None, &Progress::new()).unwrap().ok());
    }
}

#[test]
fn selective_extract_and_strip_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let src = make_tree(tmp.path());
    let a = tmp.path().join("sel.zip");
    create(&[src], &a, &CreateOptions::new(ArchiveFormat::Zip), &Progress::new()).unwrap();
    let out = tmp.path().join("sel");
    let mut o = ExtractOptions::new();
    o.selection = Some(vec!["项目 data/sub".into()]);
    o.strip_prefix = Some("项目 data".into());
    let s = extract(&a, &out, &o, &Progress::new()).unwrap();
    assert_eq!(s.files, 2);
    assert!(out.join("sub/noise.bin").is_file());
    assert!(out.join("sub/deeper/z.txt").is_file());
    assert!(!out.join("hello.txt").exists());
}

#[test]
fn overwrite_policies() {
    let tmp = tempfile::tempdir().unwrap();
    let src = make_tree(tmp.path());
    let a = tmp.path().join("ow.7z");
    create(&[src], &a, &CreateOptions::new(ArchiveFormat::SevenZ), &Progress::new()).unwrap();
    let out = tmp.path().join("ow");
    extract(&a, &out, &ExtractOptions::new(), &Progress::new()).unwrap();
    let f = out.join("项目 data/hello.txt");
    fs::write(&f, b"changed").unwrap();

    let s = extract(&a, &out, &ExtractOptions::new().overwrite(OverwriteMode::Skip), &Progress::new()).unwrap();
    assert_eq!(s.skipped, 5);
    assert_eq!(fs::read(&f).unwrap(), b"changed");

    let s = extract(&a, &out, &ExtractOptions::new().overwrite(OverwriteMode::Rename), &Progress::new()).unwrap();
    assert_eq!(s.files, 5);
    assert!(out.join("项目 data/hello (2).txt").is_file());

    let mut ask = ExtractOptions::new().overwrite(OverwriteMode::Ask);
    let asked = Arc::new(AtomicU64::new(0));
    let a2 = asked.clone();
    ask.on_conflict = Some(Arc::new(move |c: &ConflictInfo<'_>| {
        assert!(c.target.is_file());
        assert!(c.new_size.is_some());
        a2.fetch_add(1, Ordering::Relaxed);
        ConflictChoice::OverwriteAll
    }));
    extract(&a, &out, &ask, &Progress::new()).unwrap();
    assert_eq!(asked.load(Ordering::Relaxed), 1, "OverwriteAll answers once");
    assert_ne!(fs::read(&f).unwrap(), b"changed");

    let mut ren = ExtractOptions::new().overwrite(OverwriteMode::Ask);
    let asked = Arc::new(AtomicU64::new(0));
    let a3 = asked.clone();
    ren.on_conflict = Some(Arc::new(move |_c: &ConflictInfo<'_>| {
        a3.fetch_add(1, Ordering::Relaxed);
        ConflictChoice::RenameAll
    }));
    let s = extract(&a, &out, &ren, &Progress::new()).unwrap();
    assert_eq!(asked.load(Ordering::Relaxed), 1, "RenameAll answers once");
    assert_eq!(s.files, 5);
    assert!(out.join("项目 data/hello (3).txt").is_file());
}

#[test]
fn cancellation_stops_work() {
    let tmp = tempfile::tempdir().unwrap();
    let src = make_tree(tmp.path());
    for fmt in [ArchiveFormat::Zip, ArchiveFormat::SevenZ, ArchiveFormat::TarGz] {
        let a = tmp.path().join(format!("c.{}", fmt.extension()));
        let p = Progress::new();
        p.cancel();
        let e = create(std::slice::from_ref(&src), &a, &CreateOptions::new(fmt), &p).unwrap_err();
        assert!(matches!(e, Error::Cancelled), "{fmt}: {e:?}");
        assert!(!a.exists(), "{fmt}: partial archive removed");
    }
}

#[test]
fn modify_add_and_delete() {
    for (fmt, name) in [(ArchiveFormat::Zip, "m.zip"), (ArchiveFormat::SevenZ, "m.7z"), (ArchiveFormat::TarGz, "m.tar.gz")] {
        let tmp = tempfile::tempdir().unwrap();
        let src = make_tree(tmp.path());
        let a = tmp.path().join(name);
        let opts = CreateOptions::new(fmt);
        create(&[src], &a, &opts, &Progress::new()).unwrap();
        let extra = tmp.path().join("extra.txt");
        fs::write(&extra, b"added later").unwrap();
        let ops = ModifyOps {
            add: vec![extra],
            add_into: "项目 data/sub".into(),
            delete: vec!["项目 data/hello.txt".into(), "项目 data/sub/deeper".into()],
        };
        modify(&a, &ops, &opts, &Progress::new()).unwrap();
        let info = list(&a, &OpenOptions::default(), &Progress::new()).unwrap();
        let paths: Vec<&str> = info.entries.iter().map(|e| e.path.as_str()).collect();
        assert!(paths.contains(&"项目 data/sub/extra.txt"), "{fmt}: {paths:?}");
        assert!(!paths.contains(&"项目 data/hello.txt"), "{fmt}: {paths:?}");
        assert!(!paths.iter().any(|p| p.starts_with("项目 data/sub/deeper")), "{fmt}: {paths:?}");
        assert!(paths.contains(&"项目 data/中文 文件.txt"), "{fmt}: {paths:?}");
        assert!(okzip_core::test(&a, None, &Progress::new()).unwrap().ok());
    }
}

#[test]
fn smart_extract_target() {
    let tmp = tempfile::tempdir().unwrap();
    let src = make_tree(tmp.path());
    // Archive named like its only root folder → extract into parent.
    let a = tmp.path().join("项目 data.zip");
    create(std::slice::from_ref(&src), &a, &CreateOptions::new(ArchiveFormat::Zip), &Progress::new()).unwrap();
    let info = list(&a, &OpenOptions::default(), &Progress::new()).unwrap();
    assert_eq!(smart_extract_dir(&info, tmp.path()), tmp.path());
    // Different name → into a new sub-folder.
    let b = tmp.path().join("other.zip");
    create(&[src.join("hello.txt"), src.join("sub")], &b, &CreateOptions::new(ArchiveFormat::Zip), &Progress::new()).unwrap();
    let info = list(&b, &OpenOptions::default(), &Progress::new()).unwrap();
    assert_eq!(smart_extract_dir(&info, tmp.path()), tmp.path().join("other"));
}

#[test]
fn corrupt_archive_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let src = make_tree(tmp.path());
    let a = tmp.path().join("bad.zip");
    create(&[src], &a, &CreateOptions { level: 0, ..CreateOptions::new(ArchiveFormat::Zip) }, &Progress::new()).unwrap();
    let mut bytes = fs::read(&a).unwrap();
    // Flip a byte inside the stored hello.txt data.
    let needle = b"hello, OK Zip!";
    let pos = bytes.windows(needle.len()).position(|w| w == needle).unwrap();
    bytes[pos + 3] ^= 0xFF;
    fs::write(&a, bytes).unwrap();
    let r = okzip_core::test(&a, None, &Progress::new()).unwrap();
    assert!(!r.ok());
    assert!(r.errors.iter().any(|(n, _)| n.ends_with("hello.txt")), "{:?}", r.errors);
}
