//! Build helper: `okzip-shellmenu manifest <version a.b.c.d> <publisher>`
//! prints the sparse package's AppxManifest.xml; `clsid` and `dll` print the
//! COM class and the DLL's file name (used by CI and the installer).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["manifest", version, publisher] => print!("{}", okzip_shellmenu::windows_sparse_manifest(version, publisher)),
        ["clsid"] => println!("{}", okzip_shellmenu::clsid()),
        ["dll"] => println!("{}", okzip_shellmenu::native_dll_name()),
        ["package"] => println!("{}", okzip_shellmenu::PACKAGE_NAME),
        _ => {
            eprintln!("usage: okzip-shellmenu manifest <a.b.c.d> <publisher> | clsid | dll | package");
            std::process::exit(2);
        }
    }
}
