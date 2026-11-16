//! RAR read tests (list / extract / test) against real archives made by
//! WinRAR/RAR. Fixtures live in `tests/data/rar/` — see the README there for
//! where each file comes from and its licence.

#![cfg(feature = "rar")]

use okzip_core::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

fn data(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/rar").join(rel)
}

fn sso(name: &str) -> PathBuf {
    data(&format!("ssokolow/{name}"))
}

fn rars_fx(name: &str) -> PathBuf {
    data(&format!("rars/{name}"))
}

fn expected(name: &str) -> Vec<u8> {
    fs::read(sso(&format!("expected/{name}"))).unwrap()
}

fn crc(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn list_pw(path: &Path, pw: Option<&str>) -> Result<ArchiveInfo> {
    list(path, &OpenOptions { password: pw.map(str::to_string) }, &Progress::new())
}

fn extract_pw(path: &Path, dest: &Path, pw: Option<&str>) -> Result<OpStats> {
    extract(path, dest, &ExtractOptions::new().password(pw.map(str::to_string)), &Progress::new())
}

/// (name, size, crc32) of every file entry, in archive order.
fn files(info: &ArchiveInfo) -> Vec<(String, u64, Option<u32>)> {
    info.entries.iter().filter(|e| !e.is_dir).map(|e| (e.path.clone(), e.size, e.crc32)).collect()
}

const TXT_CRC: u32 = 0x6EC1_8FFE; // "Testing 123\n"
const JPG_CRC: u32 = 0xDA70_B16C;
const PNG_CRC: u32 = 0xAFB0_AC62;

// ---------------------------------------------------------------------------
// ssokolow/rar-test-files (CC0): RAR3 + RAR5, plain / solid / locked / recovery record

#[test]
fn expected_sources_match_known_crcs() {
    assert_eq!(crc(&expected("testfile.txt")), TXT_CRC);
    assert_eq!(crc(&expected("testfile.jpg")), JPG_CRC);
    assert_eq!(crc(&expected("testfile.png")), PNG_CRC);
}

#[test]
fn single_file_archives_list_extract_test() {
    for name in [
        "testfile.rar3.rar",
        "testfile.rar3.locked.rar",
        "testfile.rar5.rar",
        "testfile.rar5.rr.rar",
    ] {
        let path = sso(name);
        assert_eq!(detect_format(&path).unwrap(), ArchiveFormat::Rar, "{name}");
        let info = list_pw(&path, None).unwrap();
        assert_eq!(info.format, ArchiveFormat::Rar);
        assert_eq!(files(&info), vec![("testfile.txt".to_string(), 12, Some(TXT_CRC))], "{name}");
        assert!(!info.encrypted && !info.solid, "{name}");
        assert_eq!(info.volumes, 1);
        assert_eq!(info.archive_size, fs::metadata(&path).unwrap().len());
        assert!(info.entries[0].modified.is_some(), "{name}: mtime");
        assert!(info.entries[0].attributes.is_some(), "{name}: attributes");

        let out = tmp();
        let stats = extract_pw(&path, out.path(), None).unwrap();
        assert_eq!((stats.files, stats.bytes), (1, 12), "{name}");
        let got = fs::read(out.path().join("testfile.txt")).unwrap();
        assert_eq!(got, expected("testfile.txt"), "{name}");
        assert_eq!(crc(&got), TXT_CRC);

        let report = test(&path, None, &Progress::new()).unwrap();
        assert!(report.ok(), "{name}: {:?}", report.errors);
        assert_eq!((report.files_tested, report.bytes_tested), (1, 12), "{name}");
    }
}

#[test]
fn rar3_and_rar5_solid_flag() {
    assert!(list_pw(&sso("testfile.rar3.solid.cbr"), None).unwrap().solid);
    assert!(list_pw(&sso("testfile.rar5.solid.cbr"), None).unwrap().solid);
    assert!(!list_pw(&sso("testfile.rar3.cbr"), None).unwrap().solid);
    assert!(!list_pw(&sso("testfile.rar5.cbr"), None).unwrap().solid);
}

#[test]
fn multi_file_archives_including_solid() {
    for name in [
        "testfile.rar3.cbr",
        "testfile.rar3.solid.cbr",
        "testfile.rar5.cbr",
        "testfile.rar5.solid.cbr",
    ] {
        let path = sso(name);
        let info = list_pw(&path, None).unwrap();
        let mut f = files(&info);
        f.sort();
        assert_eq!(
            f,
            vec![
                ("testfile.jpg".to_string(), 220, Some(JPG_CRC)),
                ("testfile.png".to_string(), 87, Some(PNG_CRC)),
            ],
            "{name}"
        );
        assert_eq!(info.total_size(), 307);

        let out = tmp();
        let stats = extract_pw(&path, out.path(), None).unwrap();
        assert_eq!((stats.files, stats.bytes), (2, 307), "{name}");
        for f in ["testfile.jpg", "testfile.png"] {
            assert_eq!(fs::read(out.path().join(f)).unwrap(), expected(f), "{name}/{f}");
        }
        let report = test(&path, None, &Progress::new()).unwrap();
        assert!(report.ok(), "{name}: {:?}", report.errors);
        assert_eq!(report.files_tested, 2);
    }
}

#[test]
fn selective_extract_from_solid_and_non_solid() {
    for name in ["testfile.rar3.solid.cbr", "testfile.rar5.solid.cbr", "testfile.rar5.cbr"] {
        let info = list_pw(&sso(name), None).unwrap();
        // Pick the *last* member so a solid archive has to decode its predecessor.
        let last = info.entries.last().unwrap().path.clone();
        let out = tmp();
        let opts = ExtractOptions { selection: Some(vec![last.clone()]), ..ExtractOptions::new() };
        let stats = extract(&sso(name), out.path(), &opts, &Progress::new()).unwrap();
        assert_eq!(stats.files, 1, "{name}");
        let names: Vec<_> = fs::read_dir(out.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "{name}: {names:?}");
        assert_eq!(fs::read(out.path().join(&last)).unwrap(), expected(&last), "{name}");
    }
}

#[test]
fn overwrite_policies() {
    let path = sso("testfile.rar5.rar");
    let out = tmp();
    let target = out.path().join("testfile.txt");
    fs::write(&target, b"old").unwrap();

    let skip = ExtractOptions::new().overwrite(OverwriteMode::Skip);
    let s = extract(&path, out.path(), &skip, &Progress::new()).unwrap();
    assert_eq!((s.files, s.skipped), (0, 1));
    assert_eq!(fs::read(&target).unwrap(), b"old");

    let rename = ExtractOptions::new().overwrite(OverwriteMode::Rename);
    let s = extract(&path, out.path(), &rename, &Progress::new()).unwrap();
    assert_eq!(s.files, 1);
    assert_ne!(s.outputs[0], target);
    assert_eq!(fs::read(&s.outputs[0]).unwrap(), expected("testfile.txt"));
    assert_eq!(fs::read(&target).unwrap(), b"old");

    let over = ExtractOptions::new().overwrite(OverwriteMode::Overwrite);
    extract(&path, out.path(), &over, &Progress::new()).unwrap();
    assert_eq!(fs::read(&target).unwrap(), expected("testfile.txt"));
}

#[test]
fn mtime_is_restored() {
    let path = sso("testfile.rar5.rar");
    let info = list_pw(&path, None).unwrap();
    let out = tmp();
    extract_pw(&path, out.path(), None).unwrap();
    let m = fs::metadata(out.path().join("testfile.txt")).unwrap().modified().unwrap();
    let secs = m.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    assert_eq!(Some(secs), info.entries[0].modified);
}

#[test]
fn progress_is_reported_per_byte_and_reaches_total() {
    let path = sso("testfile.rar5.cbr");
    let seen = Arc::new(AtomicU64::new(0));
    let s2 = seen.clone();
    let p = Progress::with_callback(move |i: &ProgressInfo| {
        s2.fetch_max(i.bytes_done, Ordering::Relaxed);
    });
    let out = tmp();
    extract(&path, out.path(), &ExtractOptions::new(), &p).unwrap();
    let snap = p.snapshot();
    assert_eq!(snap.bytes_total, 307);
    assert_eq!(snap.bytes_done, 307);
    assert_eq!(snap.files_done, 2);
    assert_eq!(seen.load(Ordering::Relaxed), 307);
}

#[test]
fn cancellation_stops_extract_and_test() {
    let p = Progress::new();
    p.cancel();
    let out = tmp();
    let err = extract(&sso("testfile.rar5.cbr"), out.path(), &ExtractOptions::new(), &p).unwrap_err();
    assert!(matches!(err, Error::Cancelled), "{err:?}");
    let err = test(&sso("testfile.rar3.solid.cbr"), None, &p).unwrap_err();
    assert!(matches!(err, Error::Cancelled), "{err:?}");
}

#[test]
fn corruption_is_detected_by_test() {
    let src = fs::read(sso("testfile.rar5.cbr")).unwrap();
    let dir = tmp();
    // Flip one byte inside the stored PNG payload (the last member's data).
    let pos = src.len() - 40;
    let mut bad = src.clone();
    bad[pos] ^= 0xFF;
    let path = dir.path().join("bad.cbr");
    fs::write(&path, &bad).unwrap();
    match test(&path, None, &Progress::new()) {
        Ok(r) => assert!(!r.ok(), "corruption went unnoticed"),
        Err(e) => assert!(matches!(e, Error::Corrupt(_) | Error::Other(_)), "{e:?}"),
    }
    // Extraction must fail as well and not leave the broken file behind.
    let out = tmp();
    assert!(extract(&path, out.path(), &ExtractOptions::new(), &Progress::new()).is_err());
    assert!(!out.path().join("testfile.png").exists());
}

// ---------------------------------------------------------------------------
// bitplane/rars fixtures (Apache-2.0): encryption, Unicode names, volumes

#[test]
fn rar4_unicode_names_and_aes_per_file_encryption() {
    let path = rars_fx("rar4_mixed_visible_names_password.rar");
    // File names are visible without a password.
    let info = list_pw(&path, None).unwrap();
    assert!(info.encrypted);
    assert_eq!(
        files(&info),
        vec![
            ("1File.txt".to_string(), 5, Some(0x578A_2019)),
            ("2中文.txt".to_string(), 32, Some(0x1E18_0200)),
            ("3Sec.txt".to_string(), 30, Some(0xBEF6_4217)),
        ]
    );
    let enc: Vec<bool> = info.entries.iter().map(|e| e.encrypted).collect();
    assert_eq!(enc, vec![false, true, true]);

    let out = tmp();
    assert!(matches!(extract_pw(&path, out.path(), None), Err(Error::PasswordRequired)));
    assert!(matches!(test(&path, None, &Progress::new()), Err(Error::PasswordRequired)));
    let out = tmp();
    assert!(matches!(extract_pw(&path, out.path(), Some("nope")), Err(Error::WrongPassword)));
    assert!(matches!(test(&path, Some("nope"), &Progress::new()), Err(Error::WrongPassword)));

    // The unencrypted member can still be taken out without a password.
    let out = tmp();
    let opts = ExtractOptions { selection: Some(vec!["1File.txt".into()]), ..ExtractOptions::new() };
    extract(&path, out.path(), &opts, &Progress::new()).unwrap();
    assert_eq!(fs::read(out.path().join("1File.txt")).unwrap(), b"1File");

    let out = tmp();
    let s = extract_pw(&path, out.path(), Some("known-pass")).unwrap();
    assert_eq!(s.files, 3);
    assert_eq!(fs::read(out.path().join("2中文.txt")).unwrap(), b"known encrypted unicode payload\n");
    assert_eq!(fs::read(out.path().join("3Sec.txt")).unwrap(), b"known encrypted ascii payload\n");
    let r = test(&path, Some("known-pass"), &Progress::new()).unwrap();
    assert!(r.ok(), "{:?}", r.errors);
    assert_eq!(r.files_tested, 3);
}

#[test]
fn rar5_aes_file_encryption() {
    let path = rars_fx("password_aes.rar");
    let info = list_pw(&path, None).unwrap();
    assert!(info.encrypted && info.entries[0].encrypted);
    assert_eq!(info.entries[0].path, "hello.txt");
    assert_eq!(info.entries[0].size, 30);

    let out = tmp();
    assert!(matches!(extract_pw(&path, out.path(), None), Err(Error::PasswordRequired)));
    assert!(matches!(extract_pw(&path, out.path(), Some("wrong")), Err(Error::WrongPassword)));
    assert!(matches!(test(&path, Some("wrong"), &Progress::new()), Err(Error::WrongPassword)));
    assert!(!out.path().join("hello.txt").exists());

    extract_pw(&path, out.path(), Some("password")).unwrap();
    assert_eq!(fs::read(out.path().join("hello.txt")).unwrap(), b"Hello, RAR 5.0 fixture world.\n");
    assert!(test(&path, Some("password"), &Progress::new()).unwrap().ok());
}

#[test]
fn encrypted_headers_rar4_and_rar5() {
    for (name, payload, crc32) in [
        ("header_rar420_password.rar", None, Some(0xA538_535E)),
        ("header_encrypted.rar", Some(&b"Hello, RAR 5.0 fixture world.\n"[..]), None),
    ] {
        let path = rars_fx(name);
        assert!(matches!(list_pw(&path, None), Err(Error::PasswordRequired)), "{name}");
        assert!(matches!(list_pw(&path, Some("bad")), Err(Error::WrongPassword)), "{name}");
        let info = list_pw(&path, Some("password")).unwrap();
        assert!(info.encrypted, "{name}");
        assert_eq!(info.entries.len(), 1);
        assert_eq!(info.entries[0].path, "hello.txt");
        let out = tmp();
        assert!(matches!(extract_pw(&path, out.path(), None), Err(Error::PasswordRequired)));
        extract_pw(&path, out.path(), Some("password")).unwrap();
        let got = fs::read(out.path().join("hello.txt")).unwrap();
        if let Some(p) = payload {
            assert_eq!(got, p, "{name}");
        }
        if let Some(c) = crc32 {
            assert_eq!(crc(&got), c, "{name}");
        }
        assert!(test(&path, Some("password"), &Progress::new()).unwrap().ok(), "{name}");
    }
}

#[test]
fn solid_fixtures_from_rars() {
    let info = list_pw(&rars_fx("solid_simple_rar300.rar"), None).unwrap();
    assert!(info.solid);
    assert_eq!(
        files(&info),
        vec![("one.txt".to_string(), 48, Some(0x11CC_9FBB)), ("two.txt".to_string(), 47, Some(0xF4FD_09E8))]
    );
    let out = tmp();
    extract_pw(&rars_fx("solid_simple_rar300.rar"), out.path(), None).unwrap();
    assert_eq!(crc(&fs::read(out.path().join("two.txt")).unwrap()), 0xF4FD_09E8);

    let info = list_pw(&rars_fx("solid.rar"), None).unwrap();
    assert!(info.solid);
    let out = tmp();
    extract_pw(&rars_fx("solid.rar"), out.path(), None).unwrap();
    assert_eq!(fs::read(out.path().join("hello.txt")).unwrap(), b"Hello, RAR 5.0 fixture world.\n");
    assert_eq!(fs::read(out.path().join("tiny.txt")).unwrap(), b"AAAAAAAA\n");
}

const RANDOM_4K_CRC: u32 = 0xB9C5_4415;

fn vol_size(names: &[&str]) -> u64 {
    names.iter().map(|n| fs::metadata(rars_fx(n)).unwrap().len()).sum()
}

#[test]
fn rar5_multivolume_part_naming() {
    let parts = ["multivol.part1.rar", "multivol.part2.rar", "multivol.part3.rar"];
    // Opening any part finds the whole set.
    for open in ["multivol.part1.rar", "multivol.part2.rar"] {
        let info = list_pw(&rars_fx(open), None).unwrap();
        assert_eq!(info.volumes, 3, "{open}");
        assert_eq!(info.archive_size, vol_size(&parts));
        assert_eq!(info.entries.len(), 1);
        assert_eq!((info.entries[0].path.as_str(), info.entries[0].size), ("random_4k.bin", 4096));
    }
    let out = tmp();
    let s = extract_pw(&rars_fx("multivol.part1.rar"), out.path(), None).unwrap();
    assert_eq!((s.files, s.bytes), (1, 4096));
    assert_eq!(crc(&fs::read(out.path().join("random_4k.bin")).unwrap()), RANDOM_4K_CRC);
    let r = test(&rars_fx("multivol.part1.rar"), None, &Progress::new()).unwrap();
    assert!(r.ok(), "{:?}", r.errors);
    assert_eq!((r.files_tested, r.bytes_tested), (1, 4096));
}

#[test]
fn rar5_encrypted_multivolume() {
    let path = rars_fx("encrypted_multivol.part1.rar");
    let info = list_pw(&path, None).unwrap();
    assert!(info.encrypted);
    assert_eq!(info.volumes, 3);
    let out = tmp();
    assert!(matches!(extract_pw(&path, out.path(), None), Err(Error::PasswordRequired)));
    assert!(matches!(extract_pw(&path, out.path(), Some("bad")), Err(Error::WrongPassword)));
    extract_pw(&path, out.path(), Some("password")).unwrap();
    assert_eq!(crc(&fs::read(out.path().join("random_4k.bin")).unwrap()), RANDOM_4K_CRC);
    assert!(test(&path, Some("password"), &Progress::new()).unwrap().ok());
}

/// CRC of `bigtext_64k.bin` from the rars repository (65536 bytes of lorem ipsum).
const BIGTEXT_CRC: u32 = 0xDDC9_5682;

#[test]
fn rar3_multivolume_old_naming() {
    let names = ["multivol_oldnaming_rar300.rar", "multivol_oldnaming_rar300.r00"];
    for open in names {
        let info = list_pw(&rars_fx(open), None).unwrap();
        assert_eq!(info.volumes, 2, "{open}");
        assert_eq!(info.archive_size, vol_size(&names));
        assert_eq!(files(&info), vec![("bigtext_64k.bin".to_string(), 65536, Some(BIGTEXT_CRC))]);
    }
    let out = tmp();
    extract_pw(&rars_fx("multivol_oldnaming_rar300.r00"), out.path(), None).unwrap();
    assert_eq!(crc(&fs::read(out.path().join("bigtext_64k.bin")).unwrap()), BIGTEXT_CRC);
    let r = test(&rars_fx("multivol_oldnaming_rar300.rar"), None, &Progress::new()).unwrap();
    assert!(r.ok(), "{:?}", r.errors);
}

#[test]
fn missing_volume_is_an_error_not_a_panic() {
    let dir = tmp();
    fs::copy(rars_fx("multivol.part1.rar"), dir.path().join("m.part1.rar")).unwrap();
    fs::copy(rars_fx("multivol.part2.rar"), dir.path().join("m.part2.rar")).unwrap();
    let path = dir.path().join("m.part1.rar");
    let out = tmp();
    assert!(extract_pw(&path, out.path(), None).is_err());
    if let Ok(r) = test(&path, None, &Progress::new()) {
        assert!(!r.ok(), "a missing last volume went unnoticed");
    }
}
