//! Documents written by another application: each one is opened, saved to a temporary `.docx`
//! file, read back from disk and compared whole. Set `LOCALWRITER_DOCX_IN` to the files (separated
//! the way the platform separates the paths of `PATH`); the corpus lives outside the repository, so
//! CI skips the test.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use localwriter_doc::Document;

fn temp_file(extension: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("localwriter-real-file-{}-{n}.{extension}", std::process::id()))
}

/// Both documents without the verbatim copy of `word/settings.xml` (`SETTINGS_ORIG`, which
/// `crates/docx/src/package.rs` declares private to the crate).
///
/// The reader keeps that part as it found it only when it holds elements the reader does not model,
/// and the writer merges them into the settings it generates: a saved document that had them
/// reopens with a different original of that same part (the merge's own output). That difference is
/// older than the `zip` change — the three files in `C:\Dev\test-files\LocalOffice` differ the same
/// way, byte for byte, with `zip` 3.0.0. Only that one entry is dropped: every other part kept
/// verbatim must be equal already, and any other difference still fails the comparison.
fn without_the_settings_copy_the_first_save_replaced(opened: Document, reopened: Document) -> (Document, Document) {
    const SETTINGS_ORIG: &str = "localwriter:settings.orig";
    let only_the_settings_copy = opened.passthrough.keys().eq(reopened.passthrough.keys())
        && opened.passthrough.iter().filter(|(k, v)| reopened.passthrough.get(*k) != Some(*v)).all(|(k, _)| k == SETTINGS_ORIG);
    assert!(only_the_settings_copy, "the kept parts differ beyond the copy of word/settings.xml");
    let mut opened = opened;
    let mut reopened = reopened;
    opened.passthrough.remove(SETTINGS_ORIG);
    reopened.passthrough.remove(SETTINGS_ORIG);
    (opened, reopened)
}

#[test]
fn every_real_file_opens_saves_to_a_temporary_document_and_reopens() {
    let Ok(list) = std::env::var("LOCALWRITER_DOCX_IN") else { return };
    let paths: Vec<PathBuf> = std::env::split_paths(&list).filter(|p| !p.as_os_str().is_empty()).collect();
    assert!(!paths.is_empty(), "LOCALWRITER_DOCX_IN names no file");
    for path in paths {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let opened = localwriter_docx::read(&bytes).unwrap_or_else(|e| panic!("read {} as a document: {e}", path.display()));
        assert!(!opened.body.is_empty(), "{} opened without a body", path.display());

        let doc = temp_file("docx");
        std::fs::write(&doc, localwriter_docx::write(&opened).expect("write the document"))
            .unwrap_or_else(|e| panic!("write {}: {e}", doc.display()));
        let saved = std::fs::read(&doc).unwrap_or_else(|e| panic!("read {}: {e}", doc.display()));
        std::fs::remove_file(&doc).expect("remove the temporary document");

        let reopened = localwriter_docx::read(&saved).unwrap_or_else(|e| panic!("reopen {}: {e}", path.display()));
        let (opened, reopened) = without_the_settings_copy_the_first_save_replaced(opened, reopened);
        assert_eq!(reopened, opened, "the saved {} must hold everything the opened one had", path.display());
    }
}
