//! Windows only: embed the app icon and version info (VERSIONINFO) into `localwriter.exe`, so it
//! shows in Explorer, the taskbar, the Start menu and Alt-Tab. The document-type icons of
//! `assets/file-icons/` ride along as resources 2 to 13 (the app icon is 1); the installer points each
//! ProgId's DefaultIcon at one of them as `localwriter.exe,-<id>`, so keep `FILE_ICONS` in step with
//! `packaging/windows/localwriter.wxs`. Resource IDs never change once shipped: append, don't reorder.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `LOCALWRITER_REQUIRE_WINRES=1` turns it
//! into an error (for release builds).

/// Resource ID of each document icon, in `assets/file-icons/<ext>.ico`.
const FILE_ICONS: [(&str, u16); 12] = [
    ("docx", 2),
    ("docm", 3),
    ("dotx", 4),
    ("dotm", 5),
    ("doc", 6),
    ("dot", 7),
    ("odt", 8),
    ("rtf", 9),
    ("md", 10),
    ("html", 11),
    ("tex", 12),
    ("txt", 13),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/localwriter.ico");
    for (ext, _) in FILE_ICONS {
        println!("cargo:rerun-if-changed=../../assets/file-icons/{ext}.ico");
    }
    println!("cargo:rerun-if-env-changed=LOCALWRITER_REQUIRE_WINRES");
    println!("cargo:rerun-if-env-changed=LOCALOFFICE_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/localwriter.ico");
    for (ext, id) in FILE_ICONS {
        res.set_icon_with_id(&format!("../../assets/file-icons/{ext}.ico"), &id.to_string());
    }
    res.set("ProductName", "Local Writer")
        .set("FileDescription", "Local Writer word processor")
        .set("CompanyName", "Learning Machines LLC")
        .set("LegalCopyright", "Copyright (c) the Local Writer authors. MIT OR Apache-2.0.")
        .set("OriginalFilename", "localwriter.exe")
        .set("InternalName", "localwriter");
    if let Err(e) = res.compile() {
        if std::env::var_os("LOCALWRITER_REQUIRE_WINRES").or_else(|| std::env::var_os("LOCALOFFICE_REQUIRE_WINRES")).is_some() {
            eprintln!("embedding Windows resources failed: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=localwriter.exe built without icon/version resources: {e}");
    }
}
