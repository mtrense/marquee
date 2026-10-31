//! [`write_atomic`]: writing an edited file back.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Replaces the file at `path` with `contents` atomically: writes a temp file in the same
/// directory, gives it the original's permissions, then renames it over the original.
/// Readers see the old file or the new one, never half of it.
///
/// # Errors
///
/// Creating, writing or renaming the temp file failed, e.g. because the directory does
/// not exist or is not writable. The original is untouched then.
pub fn write_atomic(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let mut temp = tempfile::Builder::new()
        .prefix(".mrq-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    temp.write_all(contents.as_ref())?;
    temp.as_file().sync_all()?;
    if let Ok(metadata) = fs::metadata(path) {
        temp.as_file().set_permissions(metadata.permissions())?;
    }
    temp.persist(path).map_err(|err| err.error)?;
    Ok(())
}
