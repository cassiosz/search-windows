//! The files: `<folder chosen>\SearchWin Sync\<scope>\<device>.json`, one per
//! PC per scope, written only by that PC, so two PCs never write the same
//! file; and, in the app's own data folder, `sync-<scope>.json`, what this PC
//! last agreed with the folder.

use std::io;
use std::path::Path;

use crate::{File, Items, Map, decode, encode, is_id, live, pass, understood};

/// The folder made inside the one chosen. Changes with the app's name.
pub const INSIDE: &str = "SearchWin Sync";
/// Bigger than any real file by far (ten thousand bookmarks are about 2 MB);
/// past it a file is not read.
const FILE_MAX: u64 = 4 << 20;
/// More PCs than anyone has: past it the rest wait, so a pass stays small.
const FILES_MAX: usize = 32;
/// A stamp further ahead than any clock is wrong by: an entry stamped later
/// would win every merge for good.
const AHEAD_MAX: f64 = 24.0 * 60.0 * 60.0;

pub struct Outcome {
    /// What this PC should have now.
    pub live: Items,
    /// What it had when the pass began.
    pub here: Items,
    /// The names of the other PCs read.
    pub others: Vec<String>,
    /// The new base, still to be kept (see `agree`); None when unchanged.
    agreed: Option<(std::path::PathBuf, Map)>,
}

impl Outcome {
    /// Called once `live` is applied: the merged map becomes what this PC
    /// agreed. Kept before, a failed apply would leave the old values here
    /// looking like fresh edits, to be sent over the other PCs' changes.
    pub fn agree(&self) -> io::Result<()> {
        match &self.agreed {
            Some((state, merged)) => write(state, &serde_json::to_vec(merged).map_err(io::Error::other)?),
            None => Ok(()),
        }
    }
}

/// One scope: stamped, merged with the other PCs' files, and this PC's file
/// written. The caller applies `live`, then calls `agree`.
/// `current` gets what was agreed last time (some items keep what was
/// agreed) and returns the items as this PC has them now.
pub fn run(
    root: &Path,
    scope: &str,
    state: &Path,
    device: &str,
    name: &str,
    now: f64,
    current: impl FnOnce(&Items) -> Items,
) -> io::Result<Outcome> {
    let folder = root.join(scope);
    std::fs::create_dir_all(&folder)?;
    let base: Option<Map> = std::fs::read(state).ok().and_then(|d| serde_json::from_slice(&d).ok());
    let mut theirs = Vec::new();
    let mut others = Vec::new();
    for entry in std::fs::read_dir(&folder)?.flatten() {
        // Only "<id>.json": conflict copies, temp files and anything else are passed over.
        let file_name = entry.file_name();
        let Some(id) = file_name.to_str().and_then(|n| n.strip_suffix(".json")).filter(|id| is_id(id)) else {
            continue;
        };
        if id == device {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() || meta.len() > FILE_MAX || in_the_cloud(&meta) {
            continue;
        }
        let Some(mut file) = std::fs::read(entry.path()).ok().and_then(|d| decode(&d, scope)) else { continue };
        if file.device == id {
            file.items.retain(|_, e| e.t <= now + AHEAD_MAX);
            let name = file.name.map(|n| crate::clean(&n).chars().take(64).collect::<String>());
            others.push(name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Outro PC".into()));
            theirs.push(file.items);
            if theirs.len() >= FILES_MAX {
                break;
            }
        }
    }
    let here = current(&base.as_ref().map(live).unwrap_or_default());
    let merged = pass(&here, base.as_ref(), &theirs, device, now, &understood);
    let agreed = (base.as_ref() != Some(&merged)).then(|| (state.to_path_buf(), merged.clone()));
    // This PC's file, only when it says something new.
    let own = folder.join(format!("{device}.json"));
    let written = std::fs::read(&own).ok().and_then(|d| decode(&d, scope));
    if written.is_none_or(|f| f.items != merged || f.name.as_deref() != Some(name)) {
        let file =
            File { format: crate::FORMAT, device: device.into(), name: Some(name.into()), scope: scope.into(), written: now, items: merged.clone() };
        write(&own, &encode(&file))?;
    }
    Ok(Outcome { live: live(&merged), here, others, agreed })
}

/// Sync turned off: this PC's files leave the folder.
pub fn leave(root: &Path, device: &str) {
    for scope in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let _ = std::fs::remove_file(scope.path().join(format!("{device}.json")));
    }
}

/// What this PC agreed with a folder, forgotten: turned on again, or with
/// another folder, it starts as a PC joining.
pub fn forget(data: &Path) {
    for entry in std::fs::read_dir(data).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if name.to_str().is_some_and(|n| n.starts_with("sync-") && n.ends_with(".json")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A temp file beside it, then renamed over. The temp name doesn't end in
/// .json, so another PC never reads half of it.
fn write(path: &Path, data: &[u8]) -> io::Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("sync");
    let temp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&temp, data).and_then(|()| std::fs::rename(&temp, path)).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// A OneDrive file that is only in the cloud: reading it would download it
/// and hold the window meanwhile. Passed over until it is here.
#[cfg(windows)]
fn in_the_cloud(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x40000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
    meta.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
}

#[cfg(not(windows))]
fn in_the_cloud(_: &std::fs::Metadata) -> bool {
    false
}
