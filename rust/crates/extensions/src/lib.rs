//! Chrome extensions, as WebView2 takes them: unpacked folders, each kept as
//! a copy of its own under the app's data (WebView2 drops an extension whose
//! folder changes), and a list of them in `extensions.json`.
//!
//! What the window needs from a manifest is only what it shows: the name, the
//! popup page the button opens, and the icon beside the name.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// One extension added, as the list keeps it. Read and written through
/// serde_json's Value, which the app has anyway: a derive costs ~9 KB of exe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    /// Its folder under Extensions\, which is also what WebView2 knows it by.
    pub dir: String,
    /// WebView2's id (`chrome.runtime.id`), once a profile has taken it.
    pub id: String,
    pub name: String,
    /// The popup page, relative to the folder; None for an extension with no popup.
    pub popup: Option<String>,
    pub icon: Option<String>,
    pub on: bool,
}

impl Extension {
    /// One row of the list; None for a row without its folder or name.
    fn from_value(row: &Value) -> Option<Extension> {
        let text = |key: &str| row.get(key).and_then(Value::as_str).map(str::to_string);
        Some(Extension {
            dir: text("dir")?,
            id: text("id").unwrap_or_default(),
            name: text("name")?,
            popup: text("popup"),
            icon: text("icon"),
            on: row.get("on").and_then(Value::as_bool).unwrap_or(true),
        })
    }

    fn to_value(&self) -> Value {
        json!({ "dir": self.dir, "id": self.id, "name": self.name, "popup": self.popup, "icon": self.icon, "on": self.on })
    }
}

/// What a manifest says that the window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub popup: Option<String>,
    pub icon: Option<String>,
}

/// The manifest in `folder`, or why it won't do.
pub fn read_manifest(folder: &Path) -> Result<Manifest, String> {
    let text = std::fs::read_to_string(folder.join("manifest.json")).map_err(|_| "no manifest.json in that folder".to_string())?;
    let manifest: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("manifest.json does not read: {e}"))?;
    let raw = manifest.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).ok_or("the manifest has no name")?;
    let name = localized(folder, &manifest, raw).unwrap_or_else(|| raw.to_string());
    // MV3's action, MV2's browser_action or page_action: the same popup by another name.
    let action = ["action", "browser_action", "page_action"].iter().find_map(|k| manifest.get(*k));
    let popup = action.and_then(|a| a.get("default_popup")).and_then(Value::as_str).and_then(inside);
    let icon = action.and_then(|a| a.get("default_icon")).and_then(best_icon).or_else(|| manifest.get("icons").and_then(best_icon));
    Ok(Manifest { name, popup, icon })
}

/// `__MSG_name__` in the extension's default language, as Chrome shows it.
fn localized(folder: &Path, manifest: &Value, raw: &str) -> Option<String> {
    let key = raw.strip_prefix("__MSG_")?.strip_suffix("__")?;
    let locale = manifest.get("default_locale")?.as_str()?;
    let text = std::fs::read_to_string(folder.join("_locales").join(locale).join("messages.json")).ok()?;
    let messages: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    // Message names are matched without regard to case, as Chrome does.
    let (_, entry) = messages.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))?;
    entry.get("message")?.as_str().map(str::to_string)
}

/// A path the manifest gives, kept only if it stays in the folder.
fn inside(path: &str) -> Option<String> {
    let path = path.trim().trim_start_matches('/');
    let clean = !path.is_empty() && !path.contains(':') && !path.split(['/', '\\']).any(|part| part == "..");
    clean.then(|| path.to_string())
}

/// The icon nearest 32 pixels, from a size → path map or a single path.
fn best_icon(icons: &Value) -> Option<String> {
    if let Some(one) = icons.as_str() {
        return inside(one);
    }
    icons
        .as_object()?
        .iter()
        .filter_map(|(size, path)| Some((size.parse::<i32>().ok()?, inside(path.as_str()?)?)))
        .min_by_key(|(size, _)| ((size - 32).abs(), -size))
        .map(|(_, path)| path)
}

/// The list in `path`. None there, or none that reads, is no extensions.
pub fn load(path: &Path) -> Vec<Extension> {
    let rows = std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
    match rows {
        Some(Value::Array(rows)) => rows.iter().filter_map(Extension::from_value).collect(),
        _ => Vec::new(),
    }
}

/// Written beside and renamed over, so a crash leaves one list or the other.
pub fn save(path: &Path, list: &[Extension]) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    std::fs::write(&temp, Value::Array(list.iter().map(Extension::to_value).collect()).to_string())?;
    std::fs::rename(&temp, path)
}

/// Whether `dir` is a name free_dir gives: digits only, so it can only be a folder right under the root.
pub fn is_dir_name(dir: &str) -> bool {
    !dir.is_empty() && dir.len() <= 9 && dir.bytes().all(|b| b.is_ascii_digit())
}

/// A folder name under `root` that no extension has: 1, 2, 3...
pub fn free_dir(root: &Path, list: &[Extension]) -> String {
    (1..).map(|n: u32| n.to_string()).find(|d| !list.iter().any(|e| &e.dir == d) && !root.join(d).exists()).unwrap()
}

/// `from` copied into `to`, which must not exist yet. Links are not followed:
/// an extension is files, and a link could reach anything on the disk.
pub fn copy_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target: PathBuf = to.join(entry.file_name());
        if kind.is_dir() {
            copy_folder(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("searchwin-ext-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn write(folder: &Path, file: &str, value: Value) {
        let path = folder.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value.to_string()).unwrap();
    }

    #[test]
    fn mv3_popup_and_icon() {
        let f = temp("mv3");
        write(&f, "manifest.json", json!({"manifest_version": 3, "name": "Probe", "action": {"default_popup": "popup.html", "default_icon": {"16": "i16.png", "32": "i32.png", "128": "i128.png"}}}));
        let m = read_manifest(&f).unwrap();
        assert_eq!(m, Manifest { name: "Probe".into(), popup: Some("popup.html".into()), icon: Some("i32.png".into()) });
    }

    #[test]
    fn mv2_and_no_popup() {
        let f = temp("mv2");
        write(&f, "manifest.json", json!({"manifest_version": 2, "name": "Old", "browser_action": {"default_popup": "/ui/p.html"}, "icons": {"48": "a.png"}}));
        assert_eq!(read_manifest(&f).unwrap().popup.as_deref(), Some("ui/p.html"));
        assert_eq!(read_manifest(&f).unwrap().icon.as_deref(), Some("a.png"));
        write(&f, "manifest.json", json!({"name": "Quiet", "content_scripts": []}));
        let m = read_manifest(&f).unwrap();
        assert_eq!((m.popup, m.icon), (None, None));
    }

    #[test]
    fn localized_name() {
        let f = temp("msg");
        write(&f, "manifest.json", json!({"name": "__MSG_appName__", "default_locale": "en"}));
        write(&f, "_locales/en/messages.json", json!({"APPNAME": {"message": "Blocker"}}));
        assert_eq!(read_manifest(&f).unwrap().name, "Blocker");
    }

    #[test]
    fn refuses_what_wont_do() {
        let f = temp("bad");
        assert!(read_manifest(&f).is_err());
        write(&f, "manifest.json", json!({"version": "1"}));
        assert!(read_manifest(&f).is_err());
        write(&f, "manifest.json", json!({"name": "X", "action": {"default_popup": "../../evil.html"}}));
        assert_eq!(read_manifest(&f).unwrap().popup, None);
        assert_eq!(inside("C:/x.html"), None);
    }

    #[test]
    fn list_round_trip_and_free_dir() {
        let f = temp("list");
        let path = f.join("extensions.json");
        assert!(load(&path).is_empty());
        let list = vec![Extension { dir: "1".into(), id: String::new(), name: "A".into(), popup: None, icon: None, on: true }];
        save(&path, &list).unwrap();
        assert_eq!(load(&path), list);
        // A row missing its folder is dropped; a missing switch reads as on.
        std::fs::write(&path, r#"[{"name":"x"},{"dir":"4","name":"B","popup":"p.html"}]"#).unwrap();
        let read = load(&path);
        assert_eq!((read.len(), read[0].dir.as_str(), read[0].on, read[0].popup.as_deref()), (1, "4", true, Some("p.html")));
        std::fs::write(&path, "not json").unwrap();
        assert!(load(&path).is_empty());
        assert_eq!(free_dir(&f, &list), "2");
        std::fs::create_dir(f.join("2")).unwrap();
        assert_eq!(free_dir(&f, &list), "3");
        assert!(is_dir_name("3") && is_dir_name(&free_dir(&f, &list)));
        for bad in ["", "C:", "C:x", "..", "1/2", "a", "CON", "1234567890"] {
            assert!(!is_dir_name(bad), "{bad}");
        }
    }

    #[test]
    fn copies_a_folder() {
        let f = temp("copy");
        write(&f.join("src"), "manifest.json", json!({"name": "A"}));
        write(&f.join("src"), "js/a.js", json!(1));
        copy_folder(&f.join("src"), &f.join("dst")).unwrap();
        assert!(f.join("dst/manifest.json").is_file() && f.join("dst/js/a.js").is_file());
    }
}
