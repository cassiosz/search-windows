//! Which browsers are on this PC, and their profiles, by the names they give
//! them (Import.swift `Chromium.known`, `Mozilla.known`, `profiles`, `usual`).
//! Every place looked in is under the two folders `Bases` names, so a test
//! hands in folders of its own and no real browser is ever looked at.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Where Windows keeps per-user program data: %LOCALAPPDATA% and %APPDATA%.
#[derive(Clone, Debug)]
pub struct Bases {
    pub local: PathBuf,
    pub roaming: PathBuf,
}

impl Bases {
    /// This user's, from the environment. None when either is unset.
    /// SEARCHWIN_IMPORT_BASES (a test hook) puts them in a made-up world
    /// instead — its `local` and `roaming` — so a test run never sees a real browser.
    pub fn from_env() -> Option<Bases> {
        if let Some(world) = std::env::var_os("SEARCHWIN_IMPORT_BASES").filter(|w| !w.is_empty()) {
            let world = PathBuf::from(world);
            return Some(Bases { local: world.join("local"), roaming: world.join("roaming") });
        }
        Some(Bases {
            local: std::env::var_os("LOCALAPPDATA")?.into(),
            roaming: std::env::var_os("APPDATA")?.into(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Chromium,
    Firefox,
}

/// A browser with at least one profile to read.
#[derive(Clone, Debug, PartialEq)]
pub struct Browser {
    /// "Chrome", "Edge", "Arc"…: also the key of its `Record`.
    pub name: String,
    pub engine: Engine,
    /// Chromium's "User Data" (or Opera's own folder); Firefox's folder
    /// holding profiles.ini.
    pub root: PathBuf,
    /// Guest and System profiles left out; Default first.
    pub profiles: Vec<Profile>,
    /// The profile used last, brought unless another is chosen.
    pub usual: Option<String>,
    /// Arc only: its StorableSidebar.json, beside "User Data".
    pub arc_sidebar: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// The folder's name ("Default", "Profile 1", "abcd.default-release"):
    /// what the readers take.
    pub id: String,
    /// What the browser calls it ("Work", "Person 1"), else the folder's name.
    pub name: String,
    pub path: PathBuf,
}

impl Browser {
    /// The profiles to read: the one named, or every one for None.
    pub fn profiles(&self, only: Option<&str>) -> impl Iterator<Item = &Profile> {
        self.profiles.iter().filter(move |p| only.is_none_or(|id| p.id == id))
    }
}

#[derive(Clone, Copy)]
enum Base {
    Local,
    Roaming,
}

/// Chromium browsers: name, base, folder of "User Data". Chrome's channels
/// keep their own data. Opera keeps its profile in its own folder.
const CHROMIUM: &[(&str, Base, &str)] = &[
    ("Chrome", Base::Local, r"Google\Chrome\User Data"),
    ("Edge", Base::Local, r"Microsoft\Edge\User Data"),
    ("Brave", Base::Local, r"BraveSoftware\Brave-Browser\User Data"),
    ("Vivaldi", Base::Local, r"Vivaldi\User Data"),
    ("Opera", Base::Roaming, r"Opera Software\Opera Stable"),
    ("Opera GX", Base::Roaming, r"Opera Software\Opera GX Stable"),
    ("Helium", Base::Local, r"imput\Helium\User Data"),
    ("Comet", Base::Local, r"Perplexity\Comet\User Data"),
    ("Chromium", Base::Local, r"Chromium\User Data"),
    ("Chrome Beta", Base::Local, r"Google\Chrome Beta\User Data"),
    ("Chrome Dev", Base::Local, r"Google\Chrome Dev\User Data"),
    ("Chrome Canary", Base::Local, r"Google\Chrome SxS\User Data"),
    ("Edge Beta", Base::Local, r"Microsoft\Edge Beta\User Data"),
    ("Edge Dev", Base::Local, r"Microsoft\Edge Dev\User Data"),
    ("Edge Canary", Base::Local, r"Microsoft\Edge SxS\User Data"),
];

/// Firefox and its kin: the folder holding profiles.ini and Profiles\.
const MOZILLA: &[(&str, &str)] = &[
    ("Firefox", r"Mozilla\Firefox"),
    ("Zen", "zen"),
    ("LibreWolf", "librewolf"),
    ("Floorp", "Floorp"),
    ("Waterfox", "Waterfox"),
];

/// Files that make a folder a Chromium profile worth offering.
const CHROMIUM_MARKS: [&str; 4] = ["Bookmarks", "History", "Login Data", "Extensions"];

/// Every browser found under `bases`, in the order of the tables, Arc after
/// the Chromium ones and Firefox's kin last.
pub fn discover(bases: &Bases) -> Vec<Browser> {
    let mut out = Vec::new();
    for &(name, base, folder) in CHROMIUM {
        let base = match base {
            Base::Local => &bases.local,
            Base::Roaming => &bases.roaming,
        };
        if let Some(found) = chromium(name, base.join(folder)) {
            out.push(found);
        }
    }
    out.extend(arc(bases));
    for &(name, folder) in MOZILLA {
        if let Some(found) = mozilla(name, bases.roaming.join(folder)) {
            out.push(found);
        }
    }
    out
}

fn chromium(name: &str, root: PathBuf) -> Option<Browser> {
    let mut folders = vec![root.clone()];
    if let Ok(dir) = std::fs::read_dir(&root) {
        folders.extend(dir.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()));
    }
    let (names, last) = local_state(&root);
    let mut profiles: Vec<Profile> = folders
        .into_iter()
        .filter(|f| CHROMIUM_MARKS.iter().any(|m| f.join(m).is_file()))
        .filter_map(|path| {
            let id = path.file_name()?.to_string_lossy().into_owned();
            if id == "Guest Profile" || id == "System Profile" {
                return None;
            }
            let own = if path == root { name.to_string() } else { id.clone() };
            Some(Profile { name: names.iter().find(|(k, _)| *k == id).map_or(own, |(_, v)| v.clone()), id, path })
        })
        .collect();
    if profiles.is_empty() {
        return None;
    }
    profiles.sort_by_key(|p| natural(&p.id));
    let usual = last
        .into_iter()
        .find(|l| profiles.iter().any(|p| &p.id == l))
        .or_else(|| newest(&profiles, &["History", "Bookmarks", "Login Data"]));
    Some(Browser { name: name.to_string(), engine: Engine::Chromium, root, profiles, usual, arc_sidebar: None })
}

/// What "Local State" says of the profiles: each folder's name, and the
/// ones used last ("last_used", then "last_active_profiles").
fn local_state(root: &Path) -> (Vec<(String, String)>, Vec<String>) {
    let Some(doc) = read_json(&root.join("Local State"), 16 << 20) else { return (Vec::new(), Vec::new()) };
    let Some(profile) = doc.get("profile") else { return (Vec::new(), Vec::new()) };
    let mut names = Vec::new();
    if let Some(cache) = profile.get("info_cache").and_then(Value::as_object) {
        for (folder, info) in cache {
            let name = info.get("name").and_then(Value::as_str).unwrap_or("").trim();
            if !name.is_empty() {
                names.push((folder.clone(), name.chars().take(60).collect()));
            }
        }
    }
    let mut last: Vec<String> = profile.get("last_used").and_then(Value::as_str).map(String::from).into_iter().collect();
    if let Some(list) = profile.get("last_active_profiles").and_then(Value::as_array) {
        last.extend(list.iter().filter_map(Value::as_str).map(String::from));
    }
    (names, last)
}

/// Arc for Windows is a Store package: its data is under
/// Packages\TheBrowserCompany.Arc_<publisher>\LocalCache\Local\Arc, with
/// "User Data" and StorableSidebar.json side by side as on the Mac. A copy
/// installed outside the Store would be under %LOCALAPPDATA%\Arc.
fn arc(bases: &Bases) -> Option<Browser> {
    let mut homes = Vec::new();
    if let Ok(dir) = std::fs::read_dir(bases.local.join("Packages")) {
        let mut packages: Vec<PathBuf> = dir
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("TheBrowserCompany.Arc"))
            .map(|e| e.path().join(r"LocalCache\Local\Arc"))
            .collect();
        packages.sort();
        homes.extend(packages);
    }
    homes.push(bases.local.join("Arc"));
    homes.into_iter().find_map(|home| {
        let mut found = chromium("Arc", home.join("User Data"))?;
        let sidebar = home.join("StorableSidebar.json");
        found.arc_sidebar = sidebar.is_file().then_some(sidebar);
        Some(found)
    })
}

fn mozilla(name: &str, root: PathBuf) -> Option<Browser> {
    let ini = std::fs::read_to_string(root.join("profiles.ini")).unwrap_or_default();
    let parsed = profiles_ini(&ini);
    let mut paths: Vec<(PathBuf, Option<String>)> = parsed
        .profiles
        .iter()
        .map(|(path, relative, nick)| (if *relative { root.join(path) } else { PathBuf::from(path) }, nick.clone()))
        .collect();
    if let Ok(dir) = std::fs::read_dir(root.join("Profiles")) {
        paths.extend(dir.flatten().map(|e| (e.path(), None)));
    }
    let mut profiles: Vec<Profile> = Vec::new();
    for (path, nick) in paths {
        let Some(id) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        if !path.join("places.sqlite").is_file() || profiles.iter().any(|p| p.id == id) {
            continue;
        }
        profiles.push(Profile { name: nick.filter(|n| !n.is_empty()).unwrap_or_else(|| id.clone()), id, path });
    }
    if profiles.is_empty() {
        return None;
    }
    // Newest first, so the one in use leads (Mozilla.Source.files).
    profiles.sort_by_key(|p| std::cmp::Reverse(modified(&p.path.join("places.sqlite"))));
    let usual = parsed
        .usual
        .filter(|u| profiles.iter().any(|p| &p.id == u))
        .or_else(|| profiles.first().map(|p| p.id.clone()));
    Some(Browser { name: name.to_string(), engine: Engine::Firefox, root, profiles, usual, arc_sidebar: None })
}

pub(crate) struct Ini {
    /// (Path, IsRelative, Name) of each [ProfileN].
    pub profiles: Vec<(String, bool, Option<String>)>,
    /// The folder name of the profile the browser opens: [Install…]
    /// Default, else the profile marked Default=1.
    pub usual: Option<String>,
}

pub(crate) fn profiles_ini(text: &str) -> Ini {
    let mut out = Ini { profiles: Vec::new(), usual: None };
    let (mut install, mut marked) = (None, None);
    let mut section = String::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    let leaf = |p: &str| p.rsplit(['/', '\\']).next().unwrap_or(p).to_string();
    let mut close = |section: &str, fields: &mut Vec<(String, String)>| {
        let get = |k: &str| fields.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        if section.starts_with("Install") {
            if let Some(at) = get("Default") {
                install = install.take().or(Some(leaf(&at)));
            }
        } else if section.starts_with("Profile")
            && let Some(path) = get("Path")
        {
            if get("Default").as_deref() == Some("1") {
                marked = marked.take().or(Some(leaf(&path)));
            }
            let relative = get("IsRelative").as_deref() != Some("0");
            out.profiles.push((path.replace('/', "\\"), relative, get("Name")));
        }
        fields.clear();
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            close(&section, &mut fields);
            section = line[1..line.len() - 1].to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            fields.push((k.to_string(), v.to_string()));
        }
    }
    close(&section, &mut fields);
    out.usual = install.or(marked);
    out
}

/// The profile whose files changed last.
fn newest(profiles: &[Profile], files: &[&str]) -> Option<String> {
    profiles
        .iter()
        .max_by_key(|p| files.iter().map(|f| modified(&p.path.join(f))).max())
        .map(|p| p.id.clone())
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// "Default" first, then "Profile 2" before "Profile 10".
fn natural(id: &str) -> (bool, String, u64) {
    let digits: String = id.chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
    let stem = id[..id.len() - digits.len()].to_lowercase();
    (id != "Default", stem, digits.parse().unwrap_or(0))
}

/// A JSON file no bigger than `limit`.
pub(crate) fn read_json(path: &Path, limit: u64) -> Option<Value> {
    serde_json::from_slice(&read_capped(path, limit)?).ok()
}

/// A regular file's bytes, None past `limit` or when it can't be read.
pub(crate) fn read_capped(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > limit {
        return None;
    }
    std::fs::read(path).ok()
}
