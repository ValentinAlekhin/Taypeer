//! Single-writer durability checks. The local anchor must not travel with exported copies.
use crate::{Fault, Id, MAX_BYTES, Result, atomic_save, digest};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};
pub fn sync_file(file: &File) -> std::io::Result<()> {
    file.sync_all()?;
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: F_FULLFSYNC receives a live file descriptor and no pointer argument.
        if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_FULLFSYNC) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
pub fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|_| "read")?;
    if file.metadata().map_err(|_| "metadata")?.len() > MAX_BYTES as u64 {
        return Err("size");
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "read")?;
    if bytes.len() > MAX_BYTES {
        return Err("size");
    }
    Ok(bytes)
}
#[derive(Serialize, Deserialize)]
struct Anchor {
    committed: Id,
    pending: Option<Id>,
}
fn anchor_path(path: &Path) -> std::path::PathBuf {
    path.with_extension("local-anchor")
}
fn save_anchor(path: &Path, anchor: &Anchor) -> Result<()> {
    atomic_save(
        &anchor_path(path),
        &serde_json::to_vec(anchor).map_err(|_| "anchor")?,
        Fault::None,
    )
}
pub fn check_anchor(path: &Path, bytes: &[u8]) -> Result<()> {
    let anchor_file = anchor_path(path);
    if !anchor_file.exists() {
        return Ok(());
    } // first use is rooted by the caller, never global freshness
    let anchor: Anchor =
        serde_json::from_slice(&read_bounded(&anchor_file)?).map_err(|_| "anchor")?;
    let hash = digest(bytes);
    if anchor.pending == Some(hash) {
        return save_anchor(
            path,
            &Anchor {
                committed: hash,
                pending: None,
            },
        );
    }
    if anchor.committed != hash {
        return Err("local rollback or container modification");
    }
    Ok(())
}
pub fn commit(path: &Path, generation: u64, bytes: &[u8], fault: Fault) -> Result<()> {
    let old = read_bounded(path)?;
    check_anchor(path, &old)?;
    let backups = path.with_extension("backups");
    fs::create_dir_all(&backups).map_err(|_| "backups directory")?;
    let before = backups.join(format!("{:020}.snapshot", generation - 1));
    if !before.exists() {
        atomic_save(&before, &old, Fault::None)?;
    }
    // Prepare before rename: either whole old or whole new generation can be
    // recovered after uncertainty. Success requires final anchor directory sync too.
    save_anchor(
        path,
        &Anchor {
            committed: digest(&old),
            pending: Some(digest(bytes)),
        },
    )?;
    atomic_save(path, bytes, fault)?;
    save_anchor(
        path,
        &Anchor {
            committed: digest(bytes),
            pending: None,
        },
    )?;
    let mut entries: Vec<_> = fs::read_dir(&backups)
        .map_err(|_| "backups")?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "snapshot"))
        .map(|e| e.path())
        .collect();
    entries.sort();
    let remove = entries.len().saturating_sub(10);
    for entry in entries.into_iter().take(remove) {
        fs::remove_file(entry).map_err(|_| "backup retention")?;
    }
    File::open(&backups)
        .and_then(|f| f.sync_all())
        .map_err(|_| "backup sync")?;
    Ok(())
}

pub fn initialize(path: &Path) -> Result<()> {
    let bytes = read_bounded(path)?;
    save_anchor(
        path,
        &Anchor {
            committed: digest(&bytes),
            pending: None,
        },
    )
}
