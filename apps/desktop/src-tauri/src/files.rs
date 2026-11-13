use crate::error::Result;
use std::path::Path;

/// Reads a small text file, or nothing if it isn't there yet.
pub fn read(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Writes a whole file that only this account can read. It's written beside
/// the old one and renamed over it, so a crash never leaves half a file.
pub fn write_private(path: &Path, text: &str) -> Result<()> {
    if let Some(folder) = path.parent() {
        create_private_folder(folder)?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, text)?;
    restrict(&partial, 0o600)?;
    std::fs::rename(partial, path)?;
    Ok(())
}

pub fn create_private_folder(folder: &Path) -> Result<()> {
    std::fs::create_dir_all(folder)?;
    restrict(folder, 0o700)
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}
