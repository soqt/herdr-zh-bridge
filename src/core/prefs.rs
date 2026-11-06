//! Workspace preferences: Herdr's spaces as last seen, each space's pane
//! layout and each tab's binding. Drafts, chats and credentials are never
//! stored. Older files (v7 tab lists, v8 bridge-made spaces) migrate into one
//! space, and the flat v7 tab list is still written so 0.7 keeps its bindings.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

pub const VERSION: u64 = 9;
pub const ZOOM_LEVELS: [f32; 4] = [1.0, 1.15, 1.3, 1.5];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabPrefs {
    pub target: String,
    pub name: String,
    pub model: String,
    pub effort: String,
}

/// How a split lays out its children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Side by side.
    Row,
    /// Stacked.
    Column,
}

/// A pane arrangement: tab stacks split side by side or stacked, each child
/// with its share of the split.
#[derive(Clone, Debug, PartialEq)]
pub enum Tree<T> {
    Pane { tabs: Vec<T>, selected: usize },
    Split { axis: Axis, children: Vec<(f32, Tree<T>)> },
}

impl<T> Tree<T> {
    pub fn pane(tabs: Vec<T>) -> Self {
        Tree::Pane { tabs, selected: 0 }
    }

    /// Every tab, panes depth-first.
    pub fn tabs(&self) -> Vec<&T> {
        let mut out = Vec::new();
        self.visit(&mut |tab| out.push(tab));
        out
    }

    fn visit<'a>(&'a self, f: &mut impl FnMut(&'a T)) {
        match self {
            Tree::Pane { tabs, .. } => tabs.iter().for_each(f),
            Tree::Split { children, .. } => children.iter().for_each(|(_, child)| child.visit(f)),
        }
    }

    /// Maps every tab; a pane or split left empty is dropped.
    pub fn filter_map<U>(self, f: &mut impl FnMut(T) -> Option<U>) -> Option<Tree<U>> {
        match self {
            Tree::Pane { tabs, selected } => {
                let mut kept = Vec::new();
                let mut new_selected = 0;
                for (index, tab) in tabs.into_iter().enumerate() {
                    if let Some(tab) = f(tab) {
                        if index <= selected {
                            new_selected = kept.len();
                        }
                        kept.push(tab);
                    }
                }
                (!kept.is_empty()).then_some(Tree::Pane { tabs: kept, selected: new_selected })
            }
            Tree::Split { axis, children } => {
                let mut kept: Vec<(f32, Tree<U>)> =
                    children.into_iter().filter_map(|(w, child)| Some((w, child.filter_map(f)?))).collect();
                match kept.len() {
                    0 => None,
                    1 => kept.pop().map(|(_, child)| child),
                    _ => Some(Tree::Split { axis, children: kept }),
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpacePrefs {
    /// The Herdr space's id. Empty for the space holding conversations whose
    /// terminal is outside Herdr's window.
    pub key: String,
    pub name: String,
    /// `None` is this computer.
    pub machine: Option<String>,
    /// `None` while the space has no conversations.
    pub layout: Option<Tree<TabPrefs>>,
    /// The focused tab, as an index into `layout.tabs()`.
    pub active: usize,
}

impl SpacePrefs {
    pub fn new(key: impl Into<String>, name: impl Into<String>, layout: Option<Tree<TabPrefs>>) -> Self {
        SpacePrefs { key: key.into(), name: name.into(), machine: None, layout, active: 0 }
    }

    pub fn tabs(&self) -> Vec<&TabPrefs> {
        self.layout.as_ref().map(Tree::tabs).unwrap_or_default()
    }
}

/// The name of the space whose key is empty.
pub const OTHER_SPACE: &str = "其他终端";

#[derive(Clone, Debug, PartialEq)]
pub struct Preferences {
    pub hc: String,
    pub codex: String,
    pub herdr: String,
    /// Herdr's spaces as last seen, in window order.
    pub spaces: Vec<SpacePrefs>,
    /// The key of the space on screen.
    pub space: String,
    /// The space list shows initials only.
    pub sidebar_collapsed: bool,
    /// Machine groups folded in the space list.
    pub folded: Vec<String>,
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
        }
    }

    fn to_json(&self) -> Value {
        json!({"target": self.target, "name": self.name, "model": self.model, "effort": self.effort})
    }
}

/// Any depth serde_json reads (128 levels) is kept, so no saved tab is lost.
fn parse_tree(value: &Value) -> Option<Tree<TabPrefs>> {
    let map = value.as_object()?;
    if let Some(children) = map.get("children").and_then(Value::as_array) {
        let axis = match map.get("axis").and_then(Value::as_str) {
            Some("column") => Axis::Column,
            _ => Axis::Row,
        };
        let children = children
            .iter()
            .filter_map(|child| {
                let weight = child.get("weight").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                Some((weight, parse_tree(child)?))
            })
            .collect();
        return Tree::Split { axis, children }.filter_map(&mut Some);
    }
    let tabs = map.get("tabs").and_then(Value::as_array)?;
    let tabs: Vec<TabPrefs> = tabs.iter().filter_map(|tab| Some(TabPrefs::parse(tab.as_object()?))).collect();
    let selected = map.get("selected").and_then(Value::as_u64).unwrap_or(0) as usize;
    (!tabs.is_empty()).then(|| Tree::Pane { selected: selected.min(tabs.len() - 1), tabs })
}

fn tree_json(tree: &Tree<TabPrefs>, weight: Option<f32>) -> Value {
    let mut value = match tree {
        Tree::Pane { tabs, selected } => {
            json!({"tabs": tabs.iter().map(TabPrefs::to_json).collect::<Vec<_>>(), "selected": selected})
        }
        Tree::Split { axis, children } => json!({
            "axis": if *axis == Axis::Row { "row" } else { "column" },
            "children": children.iter().map(|(w, child)| tree_json(child, Some(*w))).collect::<Vec<_>>(),
        }),
    };
    if let Some(weight) = weight {
        value["weight"] = json!(weight);
    }
    value
}

/// Older files only: a conversation never bound to a terminal holds nothing
/// worth keeping, and no longer marks a space.
fn bound(tab: TabPrefs) -> Option<TabPrefs> {
    (!tab.target.is_empty()).then_some(tab)
}

impl Preferences {
    pub fn defaults(herdr: String) -> Self {
        Preferences {
            hc: "hc".into(),
            codex: "codex".into(),
            herdr,
            spaces: Vec::new(),
            space: String::new(),
            sidebar_collapsed: false,
            folded: Vec::new(),
            zoom: 1.0,
        }
    }

    /// Every tab, spaces in order.
    pub fn tabs(&self) -> Vec<&TabPrefs> {
        self.spaces.iter().flat_map(SpacePrefs::tabs).collect()
    }

    /// Lenient: wrong types fall back to defaults, a space listed twice keeps
    /// its first entry, and a terminal bound twice keeps only its first tab.
    /// Bridge-made spaces (v8), the v7 tab list and pre-0.6 single targets
    /// become one space with an empty key; its conversations move to their
    /// terminals' Herdr spaces once the layout is read.
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
        if let Some(zoom) = map.get("zoom").and_then(Value::as_f64) {
            prefs.zoom = closest_zoom(zoom as f32);
        }
        prefs.sidebar_collapsed = map.get("sidebar_collapsed").and_then(Value::as_bool).unwrap_or(false);
        prefs.folded = map
            .get("folded")
            .and_then(Value::as_array)
            .map(|f| f.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        let mut seen = Vec::new();
        let mut unique = |tab: TabPrefs| {
            if !tab.target.is_empty() {
                if seen.contains(&tab.target) {
                    return None;
                }
                seen.push(tab.target.clone());
            }
            Some(tab)
        };
        // v8 spaces: their tabs, in order, and the one each had focused.
        let mut legacy: Vec<(Vec<TabPrefs>, usize)> = Vec::new();
        let spaces = map.get("spaces").and_then(Value::as_array);
        for entry in spaces.into_iter().flatten() {
            let Some(entry) = entry.as_object() else { continue };
            let layout = entry.get("layout").and_then(parse_tree).and_then(|l| l.filter_map(&mut unique));
            let active = entry.get("active").and_then(Value::as_u64).map_or(0, |a| a as usize);
            let Some(key) = text(entry, "key") else {
                let tabs: Vec<TabPrefs> = layout.map(|l| l.tabs().into_iter().cloned().collect()).unwrap_or_default();
                legacy.push((tabs, active));
                continue;
            };
            if prefs.spaces.iter().any(|s| s.key == key) {
                continue;
            }
            let count = layout.as_ref().map_or(0, |l| l.tabs().len());
            let name = text(entry, "name").filter(|n| !n.trim().is_empty());
            let fallback = if key.is_empty() { OTHER_SPACE } else { "未命名空间" };
            prefs.spaces.push(SpacePrefs {
                name: name.unwrap_or_else(|| fallback.into()),
                machine: text(entry, "machine").filter(|m| !m.is_empty()),
                active: if active < count { active } else { 0 },
                key,
                layout,
            });
        }
        let current = map.get("space");
        prefs.space = current.and_then(Value::as_str).unwrap_or_default().to_string();
        let held = if spaces.is_some() {
            // The v8 space on screen decides which conversation stays focused.
            let shown = current.and_then(Value::as_u64).unwrap_or(0) as usize;
            let (mut tabs, mut active) = (Vec::new(), 0);
            for (index, (list, focused)) in legacy.into_iter().enumerate() {
                for (i, tab) in list.into_iter().enumerate() {
                    let Some(tab) = bound(tab) else { continue };
                    if index == shown && i == focused {
                        active = tabs.len();
                    }
                    tabs.push(tab);
                }
            }
            (!tabs.is_empty())
                .then(|| SpacePrefs { active, ..SpacePrefs::new("", OTHER_SPACE, Some(Tree::pane(tabs))) })
        } else {
            Self::migrate(map, &mut unique)
        };
        if let Some(held) = held
            && !prefs.spaces.iter().any(|s| s.key.is_empty())
        {
            prefs.space = String::new();
            prefs.spaces.push(held);
        }
        prefs
    }

    /// A v7 file: left and right tab groups split side by side.
    fn migrate(map: &Map<String, Value>, unique: &mut impl FnMut(TabPrefs) -> Option<TabPrefs>) -> Option<SpacePrefs> {
        let mut groups: [Vec<(usize, TabPrefs)>; 2] = Default::default();
        let mut index = 0;
        for entry in map.get("tabs").and_then(Value::as_array).into_iter().flatten() {
            let Some(entry) = entry.as_object() else { continue };
            let Some(tab) = unique(TabPrefs::parse(entry)).and_then(bound) else { continue };
            groups[usize::from(entry.get("group") == Some(&json!(1)))].push((index, tab));
            index += 1;
        }
        if index == 0 {
            let tab = bound(TabPrefs { name: String::new(), ..TabPrefs::parse(map) })?;
            groups[0].push((0, tab));
            index = 1;
        }
        let split = map.get("split").and_then(Value::as_bool).unwrap_or(false) && groups.iter().all(|g| !g.is_empty());
        if !split {
            let right = std::mem::take(&mut groups[1]);
            groups[0].extend(right);
            groups[0].sort_by_key(|(i, _)| *i);
        }
        let active = map.get("active").and_then(Value::as_u64).map_or(0, |a| a as usize);
        let active = if active < index { active } else { 0 };
        let pane = |group: &[(usize, TabPrefs)]| Tree::Pane {
            selected: group.iter().position(|(i, _)| *i == active).unwrap_or(0),
            tabs: group.iter().map(|(_, t)| t.clone()).collect(),
        };
        let layout = if split {
            Tree::Split { axis: Axis::Row, children: vec![(0.5, pane(&groups[0])), (0.5, pane(&groups[1]))] }
        } else {
            pane(&groups[0])
        };
        // The saved order was left group first, which the tree keeps.
        let order: Vec<usize> = groups.iter().flatten().map(|(i, _)| *i).collect();
        let active = order.iter().position(|i| *i == active).unwrap_or(0);
        Some(SpacePrefs { active, ..SpacePrefs::new("", OTHER_SPACE, Some(layout)) })
    }

    pub fn to_json(&self) -> Value {
        let spaces: Vec<Value> = self
            .spaces
            .iter()
            .map(|s| {
                json!({"key": s.key, "name": s.name, "machine": s.machine, "active": s.active,
                       "layout": s.layout.as_ref().map(|l| tree_json(l, None))})
            })
            .collect();
        // What v7 readers expect: one strip holding every tab.
        let tabs: Vec<Value> = self
            .tabs()
            .into_iter()
            .map(|t| {
                let mut value = t.to_json();
                value["group"] = json!(0);
                value
            })
            .collect();
        let shown = self.spaces.iter().position(|s| s.key == self.space);
        let before: usize = self.spaces.iter().take(shown.unwrap_or(0)).map(|s| s.tabs().len()).sum();
        let active = before + shown.map_or(0, |i| self.spaces[i].active);
        json!({
            "version": VERSION, "hc": self.hc, "codex": self.codex, "herdr": self.herdr,
            "spaces": spaces, "space": self.space, "sidebar_collapsed": self.sidebar_collapsed,
            "folded": self.folded, "zoom": self.zoom, "tabs": tabs, "split": false, "active": active,
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

    fn targets(tree: &Tree<TabPrefs>) -> Vec<&str> {
        tree.tabs().into_iter().map(|t| t.target.as_str()).collect()
    }

    fn tab(target: &str) -> TabPrefs {
        TabPrefs { target: target.into(), name: target.to_uppercase(), ..Default::default() }
    }

    #[test]
    fn legacy_single_target_migrates_to_one_tab() {
        let prefs =
            Preferences::parse(&json!({"target": "legacy", "model": "m1", "effort": "max"}), "herdr-app".into());
        assert_eq!(prefs.spaces.len(), 1);
        assert_eq!((prefs.spaces[0].key.as_str(), prefs.spaces[0].name.as_str()), ("", OTHER_SPACE));
        assert_eq!(
            prefs.spaces[0].layout,
            Some(Tree::pane(vec![TabPrefs {
                target: "legacy".into(),
                model: "m1".into(),
                effort: "max".into(),
                ..Default::default()
            }]))
        );
        assert_eq!((prefs.hc.as_str(), prefs.codex.as_str(), prefs.herdr.as_str()), ("hc", "codex", "herdr-app"));
        // Nothing bound, nothing to keep.
        assert!(Preferences::parse(&json!({"model": "m1"}), "h".into()).spaces.is_empty());
        assert!(Preferences::parse(&json!("junk"), "h".into()).spaces.is_empty());
    }

    #[test]
    fn v7_split_stays_side_by_side() {
        let file = json!({"version": 7, "hc": "hc", "codex": "codex", "herdr": "/x/herdr-app",
            "tabs": [{"target": "t1", "name": "One", "model": "", "effort": "", "group": 0},
                     {"target": "t2", "name": "Two", "model": "m", "effort": "high", "group": 1},
                     {"target": "t1", "name": "Duplicate", "group": 0}, {"target": "", "group": 0}, "junk"],
            "split": true, "active": 1});
        let prefs = Preferences::parse(&file, "unused".into());
        let space = &prefs.spaces[0];
        assert_eq!((space.key.as_str(), space.active), ("", 1));
        let Some(Tree::Split { axis: Axis::Row, children }) = &space.layout else { panic!("{:?}", space.layout) };
        assert_eq!(children.iter().map(|(_, c)| targets(c)).collect::<Vec<_>>(), [["t1"], ["t2"]]);
        assert_eq!(Preferences::parse(&prefs.to_json(), "unused".into()), prefs);
    }

    #[test]
    fn unsplit_v7_file_keeps_every_tab_in_one_pane_in_order() {
        let file =
            json!({"tabs": [{"target": "a", "group": 1}, {"target": "b", "group": 0}], "split": false, "active": 9});
        let prefs = Preferences::parse(&file, "h".into());
        let layout = prefs.spaces[0].layout.as_ref().unwrap();
        assert_eq!(targets(layout), ["a", "b"]);
        assert!(matches!(layout, Tree::Pane { selected: 0, .. }));
        assert_eq!(prefs.spaces[0].active, 0);
        let only_right = json!({"tabs": [{"target": "a", "group": 1}], "split": true});
        let prefs = Preferences::parse(&only_right, "h".into());
        assert!(matches!(&prefs.spaces[0].layout, Some(Tree::Pane { tabs, .. }) if tabs.len() == 1));
    }

    #[test]
    fn v8_spaces_become_one_held_space_of_bound_tabs() {
        let file = json!({"version": 8, "spaces": [
            {"name": "后端", "named": true, "active": 0, "layout": {"tabs": [{"target": "a"}, {"target": ""}]}},
            {"name": "空间 2", "active": 1, "layout": {"axis": "row", "children": [
                {"weight": 0.5, "tabs": [{"target": "b"}]}, {"weight": 0.5, "tabs": [{"target": "c"}]}]}},
            {"name": "空间 3", "layout": {"tabs": [{"target": ""}]}}
        ], "space": 1});
        let prefs = Preferences::parse(&file, "h".into());
        assert_eq!(prefs.spaces.len(), 1);
        let held = &prefs.spaces[0];
        assert_eq!((held.key.as_str(), held.name.as_str(), prefs.space.as_str()), ("", OTHER_SPACE, ""));
        assert_eq!(targets(held.layout.as_ref().unwrap()), ["a", "b", "c"]);
        // The second space had c focused.
        assert_eq!(held.active, 2);
        let empty = json!({"version": 8, "spaces": [{"name": "空间 1", "layout": {"tabs": [{"target": ""}]}}]});
        assert!(Preferences::parse(&empty, "h".into()).spaces.is_empty());
    }

    #[test]
    fn spaces_round_trip_and_still_write_the_v7_tab_list() {
        let mut prefs = Preferences::defaults("h".into());
        let mut remote = SpacePrefs::new(
            "box/s2",
            "api",
            Some(Tree::Split {
                axis: Axis::Row,
                children: vec![
                    (0.3, Tree::Pane { tabs: vec![tab("b"), tab("c")], selected: 1 }),
                    (
                        0.7,
                        Tree::Split {
                            axis: Axis::Column,
                            children: vec![(0.5, Tree::pane(vec![tab("d")])), (0.5, Tree::pane(vec![tab("")]))],
                        },
                    ),
                ],
            }),
        );
        remote.machine = Some("box".into());
        remote.active = 2;
        prefs.spaces = vec![
            SpacePrefs::new("s1", "herdr-zh-bridge", Some(Tree::pane(vec![tab("a")]))),
            SpacePrefs::new("s3", "空的", None),
            remote,
        ];
        prefs.space = "box/s2".into();
        prefs.sidebar_collapsed = true;
        prefs.folded = vec!["box".into()];
        let value = prefs.to_json();
        assert_eq!(Preferences::parse(&value, "h".into()), prefs);
        let flat: Vec<&str> = value["tabs"].as_array().unwrap().iter().map(|t| t["target"].as_str().unwrap()).collect();
        assert_eq!(flat, ["a", "b", "c", "d", ""]);
        assert_eq!((value["active"].clone(), value["split"].clone()), (json!(3), json!(false)));
    }

    #[test]
    fn broken_spaces_fall_back_and_duplicates_drop() {
        let file = json!({"spaces": [
            {"key": "s1", "name": "A", "layout": {"tabs": [{"target": "x"}, {"target": "y"}], "selected": 7}},
            {"key": "s2", "name": "  ", "machine": "", "layout": {"axis": "column", "children": [
                {"weight": 2, "tabs": [{"target": "x"}]}, {"weight": 1, "tabs": [{"target": "z"}]}, {"tabs": []}]}},
            {"key": "s3", "layout": "junk", "active": 4},
            {"key": "s1", "name": "again"},
            "junk"
        ], "space": 8, "tabs": [{"target": "ignored"}]});
        let prefs = Preferences::parse(&file, "h".into());
        assert_eq!(prefs.spaces.iter().map(|s| s.key.as_str()).collect::<Vec<_>>(), ["s1", "s2", "s3"]);
        assert_eq!(prefs.space, "");
        assert!(matches!(&prefs.spaces[0].layout, Some(Tree::Pane { selected: 1, .. })));
        // The duplicate x leaves a split of one, which becomes that pane.
        assert_eq!(
            prefs.spaces[1].layout,
            Some(Tree::pane(vec![TabPrefs { target: "z".into(), ..Default::default() }]))
        );
        assert_eq!((prefs.spaces[1].name.as_str(), prefs.spaces[1].machine.as_deref()), ("未命名空间", None));
        assert_eq!((prefs.spaces[2].layout.as_ref(), prefs.spaces[2].active), (None, 0));
        assert!(!prefs.tabs().iter().any(|t| t.target == "ignored"));
    }

    #[test]
    fn deep_layouts_keep_every_tab() {
        let tab = |i: usize| TabPrefs { target: format!("t{i}"), ..Default::default() };
        let mut tree = Tree::pane(vec![tab(0)]);
        for i in 1..30 {
            let axis = if i % 2 == 0 { Axis::Row } else { Axis::Column };
            tree = Tree::Split { axis, children: vec![(0.5, Tree::pane(vec![tab(i)])), (0.5, tree)] };
        }
        let mut prefs = Preferences::defaults("h".into());
        prefs.spaces.push(SpacePrefs::new("s", "深", Some(tree)));
        let parsed = Preferences::parse(&prefs.to_json(), "h".into());
        assert_eq!(parsed.tabs().len(), 30);
        assert_eq!(parsed, prefs);
    }

    #[test]
    fn save_is_atomic_and_load_falls_back_to_legacy() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("nested/preferences.json");
        let legacy = folder.path().join(".preferences.json");
        std::fs::write(&legacy, r#"{"target":"old"}"#).unwrap();
        assert_eq!(load(&path, Some(&legacy), "h".into()).tabs()[0].target, "old");
        let mut prefs = Preferences::defaults("h".into());
        prefs.spaces.push(SpacePrefs::new("s1", "空间", Some(Tree::pane(vec![tab("new")]))));
        prefs.zoom = 1.3;
        save(&path, &prefs).unwrap();
        assert!(!path.with_extension("tmp").exists());
        let loaded = load(&path, Some(&legacy), "h".into());
        assert_eq!((loaded.tabs()[0].target.as_str(), loaded.zoom), ("new", 1.3));
        assert!(legacy.exists());
    }
}
