//! Workspace preferences. The v7 layout is shared with the Python 0.7 release;
//! drafts, chats and credentials are never stored.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

pub const VERSION: u64 = 7;
pub const ZOOM_LEVELS: [f32; 4] = [1.0, 1.15, 1.3, 1.5];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabPrefs {
    pub target: String,
    pub name: String,
    pub model: String,
    pub effort: String,
    pub group: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub hc: String,
    pub codex: String,
    pub herdr: String,
    pub tabs: Vec<TabPrefs>,
    pub split: bool,
    pub active: usize,
    pub zoom: f32,
}

fn text(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(String::from)
}

impl TabPrefs {
    fn parse(map: &Map<String, Value>) -> Self {
        TabPrefs {
            target: text(map, "target").unwrap_or_default(),
            name: text(map, "name").unwrap_or_default(),
            model: text(map, "model").unwrap_or_default(),
            effort: text(map, "effort").unwrap_or_default(),
            group: usize::from(map.get("group") == Some(&json!(1))),
        }
    }
}

impl Preferences {
    pub fn defaults(herdr: String) -> Self {
        Preferences {
            hc: "hc".into(),
            codex: "codex".into(),
            herdr,
            tabs: Vec::new(),
            split: false,
            active: 0,
            zoom: 1.0,
        }
    }

    /// Lenient: wrong types fall back to defaults. A pre-0.6 file (top-level
    /// target/model/effort) becomes a single tab.
    pub fn parse(value: &Value, default_herdr: String) -> Self {
        let mut prefs = Self::defaults(default_herdr);
        let Some(map) = value.as_object() else { return prefs };
        if let Some(hc) = text(map, "hc") {
            prefs.hc = hc;
        }
        if let Some(codex) = text(map, "codex") {
            prefs.codex = codex;
        }
        if let Some(herdr) = text(map, "herdr") {
            prefs.herdr = herdr;
        }
        let mut seen = Vec::new();
        for entry in map.get("tabs").and_then(Value::as_array).into_iter().flatten() {
            let Some(entry) = entry.as_object() else { continue };
            let tab = TabPrefs::parse(entry);
            if !tab.target.is_empty() {
                if seen.contains(&tab.target) {
                    continue;
                }
                seen.push(tab.target.clone());
            }
            prefs.tabs.push(tab);
        }
        if prefs.tabs.is_empty() {
            let legacy = TabPrefs::parse(map);
            prefs.tabs.push(TabPrefs { name: String::new(), group: 0, ..legacy });
        }
        let used = |group| prefs.tabs.iter().any(|t| t.group == group);
        prefs.split = map.get("split").and_then(Value::as_bool).unwrap_or(false) && used(0) && used(1);
        if !prefs.split {
            prefs.tabs.iter_mut().for_each(|t| t.group = 0);
        }
        prefs.active = map
            .get("active")
            .and_then(Value::as_u64)
            .map(|a| a as usize)
            .filter(|a| *a < prefs.tabs.len())
            .unwrap_or(0);
        if let Some(zoom) = map.get("zoom").and_then(Value::as_f64) {
            prefs.zoom = closest_zoom(zoom as f32);
        }
        prefs
    }

    pub fn to_json(&self) -> Value {
        let tabs: Vec<Value> = self
            .tabs
            .iter()
            .map(|t| {
                json!({"target": t.target, "name": t.name, "model": t.model,
                       "effort": t.effort, "group": t.group})
            })
            .collect();
        json!({
            "version": VERSION, "hc": self.hc, "codex": self.codex, "herdr": self.herdr,
            "tabs": tabs, "split": self.split, "active": self.active, "zoom": self.zoom,
        })
    }
}

pub fn closest_zoom(zoom: f32) -> f32 {
    ZOOM_LEVELS.into_iter().min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs())).unwrap()
}

fn read(path: &Path) -> Option<Value> {
    let value: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    value.is_object().then_some(value)
}

/// Loads `path`; when it is missing or unreadable, reads the legacy
/// `.preferences.json` kept beside an older installation, leaving it in place.
pub fn load(path: &Path, legacy: Option<&Path>, default_herdr: String) -> Preferences {
    let value = read(path).or_else(|| legacy.and_then(read)).unwrap_or(Value::Null);
    Preferences::parse(&value, default_herdr)
}

pub fn legacy_path() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(".preferences.json"))
}

/// Atomic replace, so a crash never leaves a truncated file.
pub fn save(path: &Path, prefs: &Preferences) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, prefs.to_json().to_string())?;
    std::fs::rename(&temp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_single_target_migrates_to_one_tab() {
        let prefs =
            Preferences::parse(&json!({"target": "legacy", "model": "m1", "effort": "max"}), "herdr-app".into());
        assert_eq!(
            prefs.tabs,
            [TabPrefs { target: "legacy".into(), model: "m1".into(), effort: "max".into(), ..Default::default() }]
        );
        assert_eq!((prefs.hc.as_str(), prefs.codex.as_str(), prefs.herdr.as_str()), ("hc", "codex", "herdr-app"));
    }

    #[test]
    fn python_v7_file_round_trips_without_private_data() {
        let file = json!({"version": 7, "hc": "hc", "codex": "codex", "herdr": "/x/herdr-app",
            "tabs": [{"target": "t1", "name": "One", "model": "", "effort": "", "group": 0},
                     {"target": "t2", "name": "Two", "model": "m", "effort": "high", "group": 1},
                     {"target": "t1", "name": "Duplicate", "group": 0}, "junk"],
            "split": true, "active": 1});
        let prefs = Preferences::parse(&file, "unused".into());
        assert!(prefs.split);
        assert_eq!(prefs.tabs.len(), 2);
        assert_eq!((prefs.tabs[1].group, prefs.active), (1, 1));
        assert_eq!(Preferences::parse(&prefs.to_json(), "unused".into()), prefs);
    }

    #[test]
    fn unsplit_file_keeps_every_tab_in_the_first_group() {
        let file =
            json!({"tabs": [{"target": "a", "group": 1}, {"target": "b", "group": 0}], "split": false, "active": 9});
        let prefs = Preferences::parse(&file, "h".into());
        assert!(!prefs.split && prefs.tabs.iter().all(|t| t.group == 0));
        assert_eq!(prefs.active, 0);
        let only_right = json!({"tabs": [{"target": "a", "group": 1}], "split": true});
        let prefs = Preferences::parse(&only_right, "h".into());
        assert!(!prefs.split && prefs.tabs[0].group == 0);
    }

    #[test]
    fn save_is_atomic_and_load_falls_back_to_legacy() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("nested/preferences.json");
        let legacy = folder.path().join(".preferences.json");
        std::fs::write(&legacy, r#"{"target":"old"}"#).unwrap();
        assert_eq!(load(&path, Some(&legacy), "h".into()).tabs[0].target, "old");
        let mut prefs = Preferences::defaults("h".into());
        prefs.tabs.push(TabPrefs { target: "new".into(), ..Default::default() });
        prefs.zoom = 1.3;
        save(&path, &prefs).unwrap();
        assert!(!path.with_extension("tmp").exists());
        let loaded = load(&path, Some(&legacy), "h".into());
        assert_eq!((loaded.tabs[0].target.as_str(), loaded.zoom), ("new", 1.3));
        assert!(legacy.exists());
    }
}
