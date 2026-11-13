//! okzip-cli — a small command-line front end for okzip-core.
//!
//! ```text
//! okzip-cli list    <archive> [-p PASSWORD]
//! okzip-cli extract <archive> [-o DIR] [-p PASSWORD] [--skip|--rename] [--smart] [--only PATH]...
//! okzip-cli create  <archive> <files...> [-f FORMAT] [-l 0-9] [-p PASSWORD] [--zipcrypto]
//!                   [--encrypt-headers] [--no-solid] [--split SIZE]
//! okzip-cli test    <archive> [-p PASSWORD]
//! okzip-cli add     <archive> <files...> [--into DIR] [-p PASSWORD]
//! okzip-cli delete  <archive> <entries...> [-p PASSWORD]
//! ```

use okzip_core::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "OK Zip command line

USAGE:
  okzip-cli list    <archive> [-p PASSWORD]
  okzip-cli extract <archive> [-o DIR] [-p PASSWORD] [--skip|--rename] [--smart] [--only PATH]...
  okzip-cli create  <archive> <files...> [-f zip|7z|tar|tgz|tbz2|txz|tzst] [-l 0-9] [-p PASSWORD]
                    [--zipcrypto] [--encrypt-headers] [--no-solid] [--split SIZE e.g. 100M]
  okzip-cli test    <archive> [-p PASSWORD]
  okzip-cli add     <archive> <files...> [--into DIR] [-p PASSWORD] [-l 0-9]
  okzip-cli delete  <archive> <entry paths...> [-p PASSWORD]
  okzip-cli formats
";

struct Args {
    positional: Vec<String>,
    password: Option<String>,
    out: Option<PathBuf>,
    format: Option<ArchiveFormat>,
    level: Option<u32>,
    overwrite: OverwriteMode,
    smart: bool,
    only: Vec<String>,
    zipcrypto: bool,
    encrypt_headers: bool,
    solid: bool,
    split: Option<u64>,
    into: String,
    quiet: bool,
}

fn parse(mut it: impl Iterator<Item = String>) -> std::result::Result<Args, String> {
    let mut a = Args {
        positional: vec![],
        password: None,
        out: None,
        format: None,
        level: None,
        overwrite: OverwriteMode::Overwrite,
        smart: false,
        only: vec![],
        zipcrypto: false,
        encrypt_headers: false,
        solid: true,
        split: None,
        into: String::new(),
        quiet: false,
    };
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-p" | "--password" => a.password = Some(val("-p")?),
            "-o" | "--output" => a.out = Some(PathBuf::from(val("-o")?)),
            "-f" | "--format" => {
                let v = val("-f")?;
                a.format = Some(ArchiveFormat::parse(&v).ok_or(format!("unknown format {v}"))?)
            }
            "-l" | "--level" => a.level = Some(val("-l")?.parse().map_err(|_| "level must be 0-9")?),
            "--skip" => a.overwrite = OverwriteMode::Skip,
            "--rename" => a.overwrite = OverwriteMode::Rename,
            "--smart" => a.smart = true,
            "--only" => a.only.push(val("--only")?),
            "--zipcrypto" => a.zipcrypto = true,
            "--encrypt-headers" => a.encrypt_headers = true,
            "--no-solid" => a.solid = false,
            "--split" => {
                let v = val("--split")?;
                a.split = Some(parse_size(&v).ok_or(format!("bad size {v}"))?)
            }
            "--into" => a.into = val("--into")?,
            "-q" | "--quiet" => a.quiet = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option {s}")),
            _ => a.positional.push(arg),
        }
    }
    Ok(a)
}

fn progress(quiet: bool) -> Progress {
    if quiet {
        Progress::new()
    } else {
        Progress::with_callback(move |i: &ProgressInfo| {
            let pct = (i.fraction() * 100.0) as u32;
            let name: String = i.current.chars().rev().take(48).collect::<Vec<_>>().into_iter().rev().collect();
            eprint!("\r\x1b[2K{pct:3}%  {:>9}  {name}", format_size(i.bytes_done));
            let _ = std::io::stderr().flush();
        })
    }
}

fn done_line(quiet: bool) {
    if !quiet {
        eprint!("\r\x1b[2K");
    }
}

fn fmt_time(t: Option<i64>) -> String {
    let Some(t) = t else { return "                   ".into() };
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

fn run() -> std::result::Result<(), String> {
    let mut argv = std::env::args().skip(1);
    let Some(cmd) = argv.next() else { return Err(String::new()) };
    let a = parse(argv)?;
    let pos = |i: usize, what: &str| a.positional.get(i).cloned().ok_or_else(|| format!("missing {what}"));
    let e = |err: Error| match err {
        Error::PasswordRequired => "this archive is encrypted: pass -p PASSWORD".to_string(),
        Error::WrongPassword => "wrong password".to_string(),
        other => other.to_string(),
    };
    match cmd.as_str() {
        "formats" => {
            for f in ArchiveFormat::ALL {
                let rw = if f.is_writable() { "read/write" } else { "read" };
                let note = if f == ArchiveFormat::Rar && !RAR_SUPPORTED { " (not compiled in)" } else { "" };
                println!("{:8} .{:8} {rw}{note}", f.name(), f.extension());
            }
        }
        "list" | "l" => {
            let path = PathBuf::from(pos(0, "archive")?);
            let info = list(&path, &OpenOptions { password: a.password.clone() }, &Progress::new()).map_err(e)?;
            println!("{}  [{}{}{}]", path.display(), info.format, if info.encrypted { ", encrypted" } else { "" }, if info.solid { ", solid" } else { "" });
            println!("{:19}  {:>12}  {:>12}  {:8}  Name", "Modified", "Size", "Packed", "CRC32");
            for en in &info.entries {
                println!(
                    "{}  {:>12}  {:>12}  {:8}  {}{}{}",
                    fmt_time(en.modified),
                    if en.is_dir { "<DIR>".into() } else { en.size.to_string() },
                    en.packed_size.map(|s| s.to_string()).unwrap_or_default(),
                    en.crc32.map(|c| format!("{c:08X}")).unwrap_or_default(),
                    en.path,
                    if en.is_dir { "/" } else { "" },
                    if en.encrypted { " *" } else { "" },
                );
            }
            println!(
                "{} files, {} folders, {} → {} ({} volume(s))",
                info.file_count(),
                info.dir_count(),
                format_size(info.total_size()),
                format_size(info.total_packed()),
                info.volumes
            );
        }
        "extract" | "x" => {
            let path = PathBuf::from(pos(0, "archive")?);
            let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let dest = match (&a.out, a.smart) {
                (Some(o), _) => o.clone(),
                (None, true) => {
                    let info = list(&path, &OpenOptions { password: a.password.clone() }, &Progress::new()).map_err(e)?;
                    smart_extract_dir(&info, &parent)
                }
                (None, false) => parent.join(archive_stem(&path)),
            };
            let mut o = ExtractOptions::new().password(a.password.clone()).overwrite(a.overwrite);
            if !a.only.is_empty() {
                o.selection = Some(a.only.clone());
            }
            let s = extract(&path, &dest, &o, &progress(a.quiet)).map_err(e)?;
            done_line(a.quiet);
            println!("extracted {} files ({}) to {} — {} skipped", s.files, format_size(s.bytes), dest.display(), s.skipped);
        }
        "create" | "c" | "a" => {
            let target = PathBuf::from(pos(0, "archive path")?);
            let sources: Vec<PathBuf> = a.positional[1..].iter().map(PathBuf::from).collect();
            if sources.is_empty() {
                return Err("nothing to compress".into());
            }
            let format = a.format.or_else(|| format_from_name(&target)).unwrap_or(ArchiveFormat::Zip);
            let opts = CreateOptions {
                format,
                level: a.level.unwrap_or(5),
                password: a.password.clone(),
                zip_encryption: if a.zipcrypto { ZipEncryption::ZipCrypto } else { ZipEncryption::Aes256 },
                encrypt_headers: a.encrypt_headers,
                solid: a.solid,
                split_size: a.split,
            };
            let s = create(&sources, &target, &opts, &progress(a.quiet)).map_err(e)?;
            done_line(a.quiet);
            println!("{} files, {} folders, {} →", s.files, s.dirs, format_size(s.bytes));
            for out in &s.outputs {
                let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
                println!("  {} ({})", out.display(), format_size(size));
            }
        }
        "test" | "t" => {
            let path = PathBuf::from(pos(0, "archive")?);
            let r = test(&path, a.password.as_deref(), &progress(a.quiet)).map_err(e)?;
            done_line(a.quiet);
            if r.ok() {
                println!("OK — {} files, {} tested, no errors", r.files_tested, format_size(r.bytes_tested));
            } else {
                for (n, m) in &r.errors {
                    println!("ERROR {n}: {m}");
                }
                return Err(format!("{} error(s)", r.errors.len()));
            }
        }
        "add" | "delete" | "d" => {
            let path = PathBuf::from(pos(0, "archive")?);
            let rest = a.positional[1..].to_vec();
            let fmt = detect_format(&path).map_err(e)?;
            let opts = CreateOptions { format: fmt, level: a.level.unwrap_or(5), password: a.password.clone(), ..Default::default() };
            let ops = if cmd == "add" {
                ModifyOps { add: rest.iter().map(PathBuf::from).collect(), add_into: a.into.clone(), delete: vec![] }
            } else {
                ModifyOps { delete: rest, ..Default::default() }
            };
            modify(&path, &ops, &opts, &progress(a.quiet)).map_err(e)?;
            done_line(a.quiet);
            println!("updated {}", path.display());
        }
        other => return Err(format!("unknown command {other}")),
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) if msg.is_empty() => {
            print!("{USAGE}");
            ExitCode::from(2)
        }
        Err(msg) => {
            eprintln!("\nerror: {msg}");
            ExitCode::FAILURE
        }
    }
}
