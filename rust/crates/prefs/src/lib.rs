//! What you set in Settings, kept in prefs.json (upstream Prefs.swift, only
//! what the Windows v1 has switches for). Pure: reading, writing and
//! changing one setting. What each one does is the app's business.
//!
//! The file is forgiving the way upstream's defaults are: missing means the
//! defaults, and a value that can't be read resets that one setting, never
//! the others.
//!
//! Read and written by hand through serde_json's Value rather than derived:
//! the derive, made lenient field by field, cost the exe 43 KB for six settings.

use std::io::Write;
use std::path::Path;

use serde_json::{Map, Value};

/// Where words that aren't an address go (Engine.swift).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Engine {
    #[default]
    Google,
    DuckDuckGo,
    Bing,
    Ecosia,
    Startpage,
    Kagi,
    Brave,
    Qwant,
    /// The template in `Prefs::custom_engine`.
    Custom,
}

impl Engine {
    /// In the order Settings lists them.
    pub const ALL: [Engine; 9] = [
        Engine::Google,
        Engine::DuckDuckGo,
        Engine::Bing,
        Engine::Ecosia,
        Engine::Startpage,
        Engine::Kagi,
        Engine::Brave,
        Engine::Qwant,
        Engine::Custom,
    ];

    /// As the file and the page write it.
    pub fn key(self) -> &'static str {
        match self {
            Engine::Google => "google",
            Engine::DuckDuckGo => "duckduckgo",
            Engine::Bing => "bing",
            Engine::Ecosia => "ecosia",
            Engine::Startpage => "startpage",
            Engine::Kagi => "kagi",
            Engine::Brave => "brave",
            Engine::Qwant => "qwant",
            Engine::Custom => "custom",
        }
    }

    pub fn from_key(key: &str) -> Option<Engine> {
        Engine::ALL.into_iter().find(|e| e.key() == key)
    }

    /// Its name as written by its makers. Custom is named by the page, in its language.
    pub fn name(self) -> &'static str {
        match self {
            Engine::Google => "Google",
            Engine::DuckDuckGo => "DuckDuckGo",
            Engine::Bing => "Bing",
            Engine::Ecosia => "Ecosia",
            Engine::Startpage => "Startpage",
            Engine::Kagi => "Kagi",
            Engine::Brave => "Brave Search",
            Engine::Qwant => "Qwant",
            Engine::Custom => "Custom",
        }
    }
}

/// How soon a tab left alone sleeps. The minutes are the tabs crate's
/// SleepPace; this is only the choice, as written in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Sleep {
    Later,
    #[default]
    Normal,
    Sooner,
}

impl Sleep {
    pub const ALL: [Sleep; 3] = [Sleep::Later, Sleep::Normal, Sleep::Sooner];

    pub fn key(self) -> &'static str {
        match self {
            Sleep::Later => "later",
            Sleep::Normal => "normal",
            Sleep::Sooner => "sooner",
        }
    }

    pub fn from_key(key: &str) -> Option<Sleep> {
        Sleep::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// The reading mode's colours (Reader.swift `Look`): the page's own light or
/// dark, or whatever Windows is set to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ReaderLook {
    Light,
    Dark,
    #[default]
    System,
}

impl ReaderLook {
    pub const ALL: [ReaderLook; 3] = [ReaderLook::Light, ReaderLook::Dark, ReaderLook::System];

    pub fn key(self) -> &'static str {
        match self {
            ReaderLook::Light => "light",
            ReaderLook::Dark => "dark",
            ReaderLook::System => "system",
        }
    }

    pub fn from_key(key: &str) -> Option<ReaderLook> {
        ReaderLook::ALL.into_iter().find(|l| l.key() == key)
    }
}

/// The click that peeks at a link (upstream Settings › Shortcuts):
/// Shift-click unless changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PeekWith {
    #[default]
    Shift,
    Alt,
}

impl PeekWith {
    pub const ALL: [PeekWith; 2] = [PeekWith::Shift, PeekWith::Alt];

    pub fn key(self) -> &'static str {
        match self {
            PeekWith::Shift => "shift",
            PeekWith::Alt => "alt",
        }
    }

    pub fn from_key(key: &str) -> Option<PeekWith> {
        PeekWith::ALL.into_iter().find(|p| p.key() == key)
    }
}

/// Where the tabs are. Only down the left for now; kept so a file written
/// once the others exist still reads here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TabsLayout {
    #[default]
    Left,
}

impl TabsLayout {
    pub fn key(self) -> &'static str {
        "left"
    }

    pub fn from_key(key: &str) -> Option<TabsLayout> {
        (key == "left").then_some(TabsLayout::Left)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    pub engine: Engine,
    /// A search address with %s where the words go, used when `engine` is Custom.
    pub custom_engine: String,
    /// Shield blocks ads and trackers.
    pub shield: bool,
    pub sleep: Sleep,
    pub tabs: TabsLayout,
    /// Closing a tab goes back to the one viewed before, not the neighbour.
    pub close_goes_back: bool,
    /// Chosen in the reader itself, not in Settings; kept so it is there next time.
    pub reader_look: ReaderLook,
    /// Settings › Tabs › Split View: off until turned on, as upstream.
    pub split_view: bool,
    /// A playing video floats out when you go to another app, and back when
    /// you return (upstream's floatsAway). Off until turned on.
    pub float_away: bool,
    /// A modified click on a link opens it in a panel over the page. Off until turned on.
    pub peek: bool,
    pub peek_with: PeekWith,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            engine: Engine::default(),
            custom_engine: String::new(),
            shield: true,
            sleep: Sleep::default(),
            tabs: TabsLayout::default(),
            close_goes_back: false,
            reader_look: ReaderLook::default(),
            split_view: false,
            float_away: false,
            peek: false,
            peek_with: PeekWith::default(),
        }
    }
}

impl Prefs {
    /// Reads the file. None there, or none that reads, is the defaults.
    pub fn load(path: &Path) -> Prefs {
        let mut prefs = Prefs::default();
        let read = std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str::<Value>(&text).ok());
        if let Some(Value::Object(fields)) = read {
            for (key, value) in fields {
                // One setting that won't read keeps its default; the rest of the file stands.
                let _ = prefs.set(&key, value);
            }
        }
        prefs
    }

    /// Written to a file beside it and renamed over, so a crash leaves the
    /// old settings or the new ones, never half of either.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let data = self.to_value().to_string();
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        let mut temp = path.as_os_str().to_owned();
        temp.push(".tmp");
        let temp = Path::new(&temp);
        let written = (|| {
            let mut file = std::fs::File::create(temp)?;
            file.write_all(data.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(temp, path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        written
    }

    /// Every setting by name, as the file and Settings write them.
    pub fn to_value(&self) -> Value {
        let mut fields = Map::new();
        fields.insert("engine".into(), self.engine.key().into());
        fields.insert("custom_engine".into(), self.custom_engine.clone().into());
        fields.insert("shield".into(), self.shield.into());
        fields.insert("sleep".into(), self.sleep.key().into());
        fields.insert("tabs".into(), self.tabs.key().into());
        fields.insert("close_goes_back".into(), self.close_goes_back.into());
        fields.insert("reader_look".into(), self.reader_look.key().into());
        fields.insert("split_view".into(), self.split_view.into());
        fields.insert("float_away".into(), self.float_away.into());
        fields.insert("peek".into(), self.peek.into());
        fields.insert("peek_with".into(), self.peek_with.key().into());
        Value::Object(fields)
    }

    /// One setting changed by name, as Settings sends it. A value that
    /// doesn't fit is refused and nothing changes.
    pub fn set(&mut self, key: &str, value: Value) -> Result<(), String> {
        let wrong = || format!("{value} is not a value for {key}");
        let text = value.as_str();
        let on = value.as_bool();
        match key {
            "engine" => self.engine = text.and_then(Engine::from_key).ok_or_else(wrong)?,
            "custom_engine" => self.custom_engine = text.ok_or_else(wrong)?.to_string(),
            "shield" => self.shield = on.ok_or_else(wrong)?,
            "sleep" => self.sleep = text.and_then(Sleep::from_key).ok_or_else(wrong)?,
            "tabs" => self.tabs = text.and_then(TabsLayout::from_key).ok_or_else(wrong)?,
            "close_goes_back" => self.close_goes_back = on.ok_or_else(wrong)?,
            "reader_look" => self.reader_look = text.and_then(ReaderLook::from_key).ok_or_else(wrong)?,
            "split_view" => self.split_view = on.ok_or_else(wrong)?,
            "float_away" => self.float_away = on.ok_or_else(wrong)?,
            "peek" => self.peek = on.ok_or_else(wrong)?,
            "peek_with" => self.peek_with = text.and_then(PeekWith::from_key).ok_or_else(wrong)?,
            _ => return Err(format!("no setting called {key}")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> std::path::PathBuf {
        let folder = std::env::temp_dir().join(format!("searchwin-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder.join("prefs.json")
    }

    #[test]
    fn missing_file_is_the_defaults() {
        let prefs = Prefs::load(&temp("missing"));
        assert_eq!(prefs, Prefs::default());
        assert!(prefs.shield);
        assert_eq!(prefs.engine, Engine::Google);
        assert_eq!(prefs.sleep, Sleep::Normal);
    }

    #[test]
    fn corrupt_file_is_the_defaults() {
        let path = temp("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Prefs::load(&path), Prefs::default());
        std::fs::write(&path, b"[1, 2]").unwrap();
        assert_eq!(Prefs::load(&path), Prefs::default());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn saved_and_read_back() {
        let path = temp("round");
        let prefs = Prefs {
            engine: Engine::Custom,
            custom_engine: "https://s.example/?q=%s".into(),
            shield: false,
            sleep: Sleep::Sooner,
            tabs: TabsLayout::Left,
            close_goes_back: true,
            reader_look: ReaderLook::Dark,
            split_view: true,
            float_away: true,
            peek: true,
            peek_with: PeekWith::Alt,
        };
        prefs.save(&path).unwrap();
        assert_eq!(Prefs::load(&path), prefs);
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn one_bad_value_resets_only_itself() {
        let path = temp("bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"engine":"altavista","shield":"maybe","sleep":"sooner","later":1}"#).unwrap();
        let prefs = Prefs::load(&path);
        assert_eq!(prefs.engine, Engine::Google);
        assert!(prefs.shield);
        assert_eq!(prefs.sleep, Sleep::Sooner);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn written_keys() {
        let value = Prefs { engine: Engine::DuckDuckGo, ..Prefs::default() }.to_value();
        assert_eq!(
            value,
            json!({ "engine": "duckduckgo", "custom_engine": "", "shield": true, "sleep": "normal", "tabs": "left", "close_goes_back": false, "reader_look": "system", "split_view": false, "float_away": false, "peek": false, "peek_with": "shift" })
        );
    }

    #[test]
    fn set_one_setting() {
        let mut prefs = Prefs::default();
        prefs.set("engine", json!("bing")).unwrap();
        prefs.set("shield", json!(false)).unwrap();
        prefs.set("sleep", json!("later")).unwrap();
        prefs.set("custom_engine", json!("https://x.example/?q=%s")).unwrap();
        prefs.set("close_goes_back", json!(true)).unwrap();
        assert_eq!(prefs.engine, Engine::Bing);
        assert!(!prefs.shield);
        assert_eq!(prefs.sleep, Sleep::Later);
        assert_eq!(prefs.custom_engine, "https://x.example/?q=%s");
        assert!(prefs.close_goes_back);
    }

    #[test]
    fn set_refuses_what_does_not_fit() {
        let mut prefs = Prefs::default();
        assert!(prefs.set("engine", json!("altavista")).is_err());
        assert!(prefs.set("shield", json!("off")).is_err());
        assert!(prefs.set("volume", json!(11)).is_err());
        assert!(prefs.set("tabs", json!("top")).is_err());
        assert!(prefs.set("custom_engine", json!(3)).is_err());
        assert_eq!(prefs, Prefs::default());
    }

    #[test]
    fn every_engine_and_pace_reads_back() {
        for engine in Engine::ALL {
            assert_eq!(Engine::from_key(engine.key()), Some(engine));
            assert!(!engine.name().is_empty());
        }
        for pace in Sleep::ALL {
            assert_eq!(Sleep::from_key(pace.key()), Some(pace));
        }
        for look in ReaderLook::ALL {
            assert_eq!(ReaderLook::from_key(look.key()), Some(look));
        }
        for with in PeekWith::ALL {
            assert_eq!(PeekWith::from_key(with.key()), Some(with));
        }
    }

    #[test]
    fn floating_away_and_peeking_are_off_until_turned_on() {
        let mut prefs = Prefs::default();
        assert!(!prefs.float_away && !prefs.peek);
        assert_eq!(prefs.peek_with, PeekWith::Shift);
        prefs.set("float_away", json!(true)).unwrap();
        prefs.set("peek", json!(true)).unwrap();
        prefs.set("peek_with", json!("alt")).unwrap();
        assert!(prefs.float_away && prefs.peek);
        assert_eq!(prefs.peek_with, PeekWith::Alt);
        // Only the clicks Settings offers.
        assert!(prefs.set("peek_with", json!("ctrl")).is_err());
        assert!(prefs.set("peek", json!("yes")).is_err());
        assert_eq!(prefs.peek_with, PeekWith::Alt);
    }
}
