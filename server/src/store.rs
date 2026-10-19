use std::io::Write;
use std::path::Path;

use serde::Serialize;

/// Write JSON so a crash leaves either the old file or the new one, never half of one.
/// `private` limits the file to its owner on macOS and Linux.
pub fn save_json(path: &Path, value: &impl Serialize, private: bool) -> std::io::Result<()> {
    save_bytes(path, &serde_json::to_vec(value)?, private)
}

/// `save_json` for JSON already serialized.
pub fn save_bytes(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut temp_name = path.as_os_str().to_owned();
    temp_name.push(".tmp");
    let temp = std::path::PathBuf::from(temp_name);
    {
        let mut file = std::fs::File::create(&temp)?;
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        let _ = private;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path)
}
