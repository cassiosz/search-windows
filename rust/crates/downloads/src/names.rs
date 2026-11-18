//! Where a download goes: a name that is safe on Windows, and a free one.

use std::path::{Path, PathBuf};

/// Longest name kept, in characters, extension included: far under MAX_PATH
/// for any Downloads folder, and nobody reads more.
const LONGEST: usize = 150;

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
    "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A file name a page suggested, made safe to create: its last path
/// component only (`Browser.fileName`, which falls back to "download" for
/// "", ".", ".."), and for Windows: no `<>:"/\|?*` or control characters, no
/// trailing dot or space, no reserved device name (CON.txt is the console).
pub fn sanitize_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut clean: String =
        last.chars().map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c }).collect();
    clean.truncate(clean.trim_end_matches(['.', ' ']).len());
    let clean = clean.trim_start().to_string();
    if clean.is_empty() {
        return "download".to_string();
    }
    let stem = clean.split('.').next().unwrap_or("").to_ascii_uppercase();
    let mut clean = if RESERVED.contains(&stem.as_str()) { format!("_{clean}") } else { clean };
    if clean.chars().count() > LONGEST {
        // Keep the extension: it is what opens the file.
        let (stem, ext) = split_ext(&clean);
        let room = LONGEST.saturating_sub(ext.chars().count()).max(1);
        clean = format!("{}{ext}", stem.chars().take(room).collect::<String>());
    }
    clean
}

/// ("report", ".pdf"); ("archive.tar", ".gz"); (".bashrc", "") — a leading
/// dot is part of the stem, as `NSString.pathExtension` reads it.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// The name to use in a folder where `exists(name)` says which are taken: the
/// name itself if free, else "stem (1).ext", "stem (2).ext", ... Upstream's
/// `Browser.free` numbers from " 2" ("report 2.pdf") because WebKit refuses
/// to overwrite; here it is Windows' own "(1)" convention, which Explorer
/// and every other browser show. The name is made safe first.
pub fn free_name(name: &str, exists: impl Fn(&str) -> bool) -> String {
    let name = sanitize_file_name(name);
    if !exists(&name) {
        return name;
    }
    let (stem, ext) = split_ext(&name);
    for n in 1..10_000 {
        let next = format!("{stem} ({n}){ext}");
        if !exists(&next) {
            return next;
        }
    }
    // Ten thousand copies: not a real case. Fail the way the loop would.
    format!("{stem} (10000){ext}")
}

/// The same against a folder on disk.
pub fn free_path(folder: &Path, name: &str) -> PathBuf {
    folder.join(free_name(name, |n| folder.join(n).exists()))
}
