//! `cargo xtask parity`: write target/parity-checklist.md from the live command registry compared with the
//! word-processor feature catalog (via `localwriter-cli parity --markdown`). The output is a work tool,
//! not a doc: `target/` keeps it out of the repository.

use std::path::Path;

pub fn run(root: &Path) -> Result<(), String> {
    let out = crate::cargo()
        .args(["run", "-q", "-p", "localwriter-cli", "--", "parity", "--markdown"])
        .output()
        .map_err(|e| format!("cargo run localwriter-cli: {e}"))?;
    if !out.status.success() {
        return Err(format!("localwriter-cli parity failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let md = String::from_utf8_lossy(&out.stdout).to_string();
    let path = root.join("target/parity-checklist.md");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    std::fs::write(&path, &md).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(line) = md.lines().find(|l| l.starts_with("**")) {
        println!("{line}");
    }
    println!("wrote {}", path.display());
    Ok(())
}
