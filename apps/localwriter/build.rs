//! Windows only: embed the app icon and version info (VERSIONINFO) into `localwriter.exe`, so it
//! shows in Explorer, the taskbar, the Start menu and Alt-Tab.
//!
//! On every other target this does nothing. A missing resource compiler is a warning, so a
//! cross-compile from macOS or Linux still links, unless `LOCALWRITER_REQUIRE_WINRES=1` turns it
//! into an error (for release builds).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/localwriter.ico");
    println!("cargo:rerun-if-env-changed=LOCALWRITER_REQUIRE_WINRES");
    println!("cargo:rerun-if-env-changed=LOCALOFFICE_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/localwriter.ico")
        .set("ProductName", "Local Writer")
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
