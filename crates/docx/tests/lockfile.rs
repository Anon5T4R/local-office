//! The workspace locks exactly one `zip`, at a version without the known advisories:
//! GHSA-m665-v7pm-7grj affects 9.0.4 and below, GHSA-8hxf-qr8p-62x4 and GHSA-q7v9-x93c-xqv3 below 9.0.0.

use std::path::PathBuf;

/// A `key = "value"` line of a `Cargo.lock` package block.
fn field<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key} = \"");
    block.lines().find_map(|line| line.strip_prefix(&prefix)?.strip_suffix('"'))
}

/// The `version` of every package named `name` in the lock.
fn locked_versions<'a>(lock: &'a str, name: &str) -> Vec<&'a str> {
    lock.split("[[package]]").skip(1).filter(|block| field(block, "name") == Some(name)).filter_map(|block| field(block, "version")).collect()
}

fn numeric(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().unwrap_or_else(|e| panic!("version {version:?}: {e}")));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

#[test]
fn workspace_locks_a_single_zip_without_the_known_advisories() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let lock = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let versions = locked_versions(&lock, "zip");
    assert_eq!(versions.len(), 1, "the workspace must lock exactly one `zip`, found {versions:?}");
    let version = versions[0];
    assert!(numeric(version) >= (9, 0, 5), "`zip {version}` is in the range of GHSA-m665-v7pm-7grj (fixed in 9.0.5)");
}
