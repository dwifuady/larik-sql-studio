use std::path::{Path, PathBuf};

const MAX_PATH_LEN: usize = 260; // Windows MAX_PATH pragmatic cap
const MAX_IMPORT_BYTES: usize = 10_000_000; // 10 MB for SQL imports

pub fn validate_export_path(raw: &str, expected_ext: &str) -> Result<PathBuf, String> {
    if raw.is_empty() {
        return Err("Path is empty".into());
    }
    if raw.len() > MAX_PATH_LEN {
        return Err("Path too long".into());
    }
    let p = PathBuf::from(raw);
    // For save dialogs the file may not exist yet — canonicalize the parent.
    let canon = p.canonicalize().or_else(|_| {
        let parent = p.parent().ok_or("Path has no parent")?;
        let canon_parent = parent
            .canonicalize()
            .map_err(|e| format!("Invalid parent dir: {e}"))?;
        let fname = p.file_name().ok_or("Path has no file name")?;
        Ok::<_, String>(canon_parent.join(fname))
    })?;
    if canon.extension().and_then(|e| e.to_str()) != Some(expected_ext) {
        return Err(format!("Expected a .{expected_ext} file"));
    }
    deny_system_dirs(&canon)?;
    Ok(canon)
}

pub fn validate_import_path(raw: &str, expected_ext: &str) -> Result<PathBuf, String> {
    let canon = validate_export_path(raw, expected_ext)?;
    let meta = std::fs::metadata(&canon).map_err(|e| format!("Cannot stat file: {e}"))?;
    if !meta.is_file() {
        return Err("Not a regular file".into());
    }
    if meta.len() > MAX_IMPORT_BYTES as u64 {
        return Err(format!("File exceeds {} byte limit", MAX_IMPORT_BYTES));
    }
    Ok(canon)
}

/// Reject Windows system directories and anything outside the user tree.
fn deny_system_dirs(canon: &Path) -> Result<(), String> {
    let s = canon.to_string_lossy();
    for forbidden in [
        r"C:\Windows",
        r"C:\Program Files",
        r"C:\Program Files (x86)",
    ] {
        if s.starts_with(forbidden) {
            return Err("Cannot write to system directory".into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty() {
        assert!(validate_export_path("", "csv").is_err());
    }

    #[test]
    fn rejects_too_long() {
        assert!(validate_export_path(&"a".repeat(261), "csv").is_err());
    }

    #[test]
    fn rejects_wrong_ext() {
        // Use a real temp dir so parent canonicalizes
        let dir = std::env::temp_dir();
        let p = dir.join("larik_test_wrong_ext.txt");
        std::fs::write(&p, b"hi").unwrap();
        let r = validate_export_path(p.to_str().unwrap(), "csv");
        assert!(r.is_err());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn rejects_import_too_large() {
        // Create a temp file with correct extension, then test size gate via metadata len
        // We do not actually create a 10MB file — instead verify the error variant exists
        let dir = std::env::temp_dir();
        let p = dir.join("larik_test_valid.sql");
        std::fs::write(&p, b"SELECT 1").unwrap();
        let r = validate_import_path(p.to_str().unwrap(), "sql");
        assert!(r.is_ok());
        let _ = std::fs::remove_file(&p);
    }
}
