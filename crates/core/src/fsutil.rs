//! Мелкие файловые помощники.

use std::fs;
use std::path::Path;

use crate::Result;

/// Запись через временный файл и переименование: сбой посреди записи
/// не оставляет испорченный файл.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)?;
    Ok(())
}
