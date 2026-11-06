//! Headless UI tests on GPUI's test platform: real entities, real workers,
//! fake terminals and translators.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::{
    AppContext, Bounds, Entity, Modifiers, MouseButton, Pixels, ScrollDelta, ScrollEvent, TestAppContext,
    VisualTestContext, point, px, size,
};
use gpui_component::Root;
use serde_json::{Value, json};

use super::conversation::{ChatItem, ConversationPane, Role};
use super::layout::TabId;
use super::workspace::Workspace;
use super::{Backend, Settings};
use crate::core::catalog::TerminalRow;
use crate::core::error::{BridgeError, Result};
use crate::core::herdr::Terminals;
use crate::core::prefs::{self, Preferences, TabPrefs};
use crate::core::session::Session;
use crate::core::translator::{Language, Translate};

#[derive(Default)]
struct Terminal {
    missing: bool,
    status: String,
    turn: i64,
    screen: String,
    sent: Vec<String>,
}

type World = Arc<Mutex<HashMap<String, Terminal>>>;

struct FakeCore(World);
struct FakeTranslator;

impl Terminals for FakeCore {
    fn terminals(&self) -> Result<Vec<Value>> {
        Ok(self.0.lock().unwrap().keys().map(|a| json!({"address": a, "name": a})).collect())
    }
    fn get(&self, address: &str) -> Result<Value> {
        let world = self.0.lock().unwrap();
        let t = world.get(address).filter(|t| !t.missing).ok_or_else(|| BridgeError::new("no terminal"))?;
        Ok(json!({"pid": 1, "foreground": {"pid": 2, "name": "codex"}, "status": {"state": t.status}, "turn": t.turn}))
    }
    fn read(&self, address: &str) -> Result<String> {
        Ok(self.0.lock().unwrap()[address].screen.clone())
    }
    fn send(&self, address: &str, text: &str) -> Result<()> {
        let mut world = self.0.lock().unwrap();
        let t = world.get_mut(address).unwrap();
        t.sent.push(text.into());
        t.screen = format!("Answer {address}");
        Ok(())
    }
}

impl Translate for FakeTranslator {
    fn translate(&self, text: &str, language: Language) -> Result<String> {
        Ok(match language {
            Language::English => format!("English {text}"),
            Language::SimplifiedChinese => format!("中文 {text}"),
        })
    }
    fn extract_reply(&self, screen: &str, _: Option<&str>, _: &str) -> Result<String> {
        Ok(screen.into())
    }
}

fn world(addresses: &[&str]) -> World {
    let terminals = addresses
        .iter()
        .map(|a| (a.to_string(), Terminal { status: "idle".into(), screen: "before".into(), ..Default::default() }))
        .collect();
    Arc::new(Mutex::new(terminals))
}

fn tabs(targets: &[&str]) -> Preferences {
    let mut prefs = Preferences::defaults("herdr-app".into());
    prefs.tabs = targets.iter().map(|t| TabPrefs { target: t.to_string(), ..Default::default() }).collect();
    prefs
}

struct Harness<'a> {
    cx: &'a mut VisualTestContext,
    workspace: Entity<Workspace>,
    world: World,
}

fn open(cx: &mut TestAppContext, prefs: Preferences, path: Option<PathBuf>, world: World) -> Harness<'_> {
    let backend_world = world.clone();
    let backend =
        Backend(Arc::new(move || Session::new(Box::new(FakeCore(backend_world.clone())), Box::new(FakeTranslator))));
    let settings =
        Settings { hc: "hc".into(), codex: "codex".into(), herdr: "herdr-app".into(), zoom: 1.0, preview: false };
    cx.update(|cx| super::init(settings, backend, cx));
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| Workspace::new(prefs, path, "", window, cx));
        Root::new(view, window, cx)
    });
    let root = window.root(cx).unwrap();
    let workspace = cx.read(|cx| root.read(cx).view().clone().downcast::<Workspace>().unwrap());
    let cx = VisualTestContext::from_window(window.into(), cx).into_mut();
    cx.simulate_resize(size(px(1000.), px(700.)));
    cx.run_until_parked();
    Harness { cx, workspace, world }
}

impl Harness<'_> {
    fn pane(&mut self, index: usize) -> Entity<ConversationPane> {
        let workspace = self.workspace.clone();
        self.cx.read(|cx| {
            let w = workspace.read(cx);
            w.pane(w.layout().order()[index]).unwrap().clone()
        })
    }

    fn id(&mut self, index: usize) -> TabId {
        self.pane(index).read_with(self.cx, |p, _| p.id)
    }

    fn on_pane<R>(
        &mut self,
        index: usize,
        f: impl FnOnce(&mut ConversationPane, &mut gpui::Window, &mut gpui::Context<ConversationPane>) -> R,
    ) -> R {
        let pane = self.pane(index);
        let result = self.cx.update(|window, cx| pane.update(cx, |p, cx| f(p, window, cx)));
        self.cx.run_until_parked();
        result
    }

    fn on_workspace<R>(
        &mut self,
        f: impl FnOnce(&mut Workspace, &mut gpui::Window, &mut gpui::Context<Workspace>) -> R,
    ) -> R {
        let workspace = self.workspace.clone();
        let result = self.cx.update(|window, cx| workspace.update(cx, |w, cx| f(w, window, cx)));
        self.cx.run_until_parked();
        result
    }

    fn draft(&mut self, index: usize) -> String {
        let pane = self.pane(index);
        self.cx.read(|cx| pane.read(cx).draft(cx))
    }

    fn chat(&mut self, index: usize) -> Vec<(Role, String)> {
        self.pane(index).read_with(self.cx, |p, _| {
            p.items
                .iter()
                .filter_map(|item| match item {
                    ChatItem::Message { role, text } => Some((*role, text.to_string())),
                    ChatItem::Log { .. } => None,
                })
                .collect()
        })
    }

    fn logs(&mut self, index: usize) -> Vec<(Vec<String>, bool)> {
        self.pane(index).read_with(self.cx, |p, _| {
            p.items
                .iter()
                .filter_map(|item| match item {
                    ChatItem::Log { entries, expanded, .. } => Some((entries.clone(), *expanded)),
                    ChatItem::Message { .. } => None,
                })
                .collect()
        })
    }

    fn set(&mut self, address: &str, f: impl FnOnce(&mut Terminal)) {
        f(self.world.lock().unwrap().get_mut(address).unwrap());
    }

    fn sent(&self, address: &str) -> Vec<String> {
        self.world.lock().unwrap()[address].sent.clone()
    }

    fn draw(&mut self) {
        self.cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
    }

    fn bounds(&mut self, selector: &str) -> Bounds<Pixels> {
        self.draw();
        // Test-only: debug selectors are looked up by 'static name.
        let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
        self.cx.debug_bounds(selector).unwrap_or_else(|| panic!("no element {selector}"))
    }

    fn drag(&mut self, tab: TabId, to: gpui::Point<Pixels>, release: bool) {
        let start = self.bounds(&format!("tab-{tab}")).center();
        self.cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        self.cx.simulate_mouse_move(start + point(px(20.), px(0.)), MouseButton::Left, Modifiers::default());
        self.cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
        self.cx.simulate_mouse_move(to + point(px(1.), px(0.)), MouseButton::Left, Modifiers::default());
        if release {
            self.cx.simulate_mouse_up(to + point(px(1.), px(0.)), MouseButton::Left, Modifiers::default());
        }
        self.cx.run_until_parked();
    }

    fn right_of_body(&mut self) -> gpui::Point<Pixels> {
        let body = self.bounds("body-0");
        point(body.left() + body.size.width * 0.85, body.center().y)
    }
}

#[gpui::test]
fn input_clears_at_once_and_the_reply_is_tracked(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1"]), None, world(&["t1"]));
    let pane = h.pane(0);
    h.cx.update(|window, cx| {
        pane.update(cx, |p, cx| {
            p.set_draft("  你好\n", window, cx);
            p.send(window, cx);
            // Cleared before the worker has even started.
            assert_eq!(p.draft(cx), "");
            assert_eq!(p.sending, 1);
        })
    });
    h.cx.run_until_parked();
    assert_eq!(h.sent("t1"), ["English 你好"]);
    assert_eq!(h.chat(0), [(Role::User, "你好".to_string())]);
    assert!(h.pane(0).read_with(h.cx, |p, _| p.has_pending() && p.sending == 0));
}

#[gpui::test]
fn typing_during_a_send_is_kept(cx: &mut TestAppContext) {
    for next in ["下一条", "原文"] {
        let mut h = open(cx, tabs(&["t1"]), None, world(&["t1"]));
        let pane = h.pane(0);
        h.cx.update(|window, cx| {
            pane.update(cx, |p, cx| {
                p.set_draft("原文", window, cx);
                p.send(window, cx);
                // Typed before the worker reports the send.
                p.set_draft(next, window, cx);
            })
        });
        h.cx.run_until_parked();
        assert_eq!(h.draft(0), next);
    }
}

#[gpui::test]
fn enter_sends_and_shift_enter_breaks_the_line(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1"]), None, world(&["t1"]));
    h.on_pane(0, |p, window, cx| p.focus_input(window, cx));
    h.cx.simulate_input("你好");
    h.cx.simulate_keystrokes("shift-enter");
    h.cx.simulate_input("世界");
    assert_eq!(h.draft(0), "你好\n世界");
    assert!(h.sent("t1").is_empty());
    h.cx.simulate_keystrokes("enter");
    h.cx.run_until_parked();
    assert_eq!(h.sent("t1"), ["English 你好\n世界"]);
    assert_eq!(h.draft(0), "");
}

#[gpui::test]
fn enter_in_the_model_field_does_not_send(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1"]), None, world(&["t1"]));
    h.on_pane(0, |p, window, cx| p.set_draft("草稿", window, cx));
    let field = h.bounds("model-field");
    h.cx.simulate_click(field.center(), Modifiers::default());
    h.cx.simulate_input("x");
    assert!(h.pane(0).read_with(h.cx, |p, _| p.models.model.ends_with('x')));
    h.cx.simulate_keystrokes("enter");
    h.cx.run_until_parked();
    assert!(h.sent("t1").is_empty());
    assert_eq!(h.draft(0), "草稿");
}

#[gpui::test]
fn failure_returns_the_text_and_other_tabs_stay_usable(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1", "t2"]), None, world(&["t1", "t2"]));
    h.set("t1", |t| t.missing = true);
    h.on_pane(0, |p, window, cx| {
        p.set_draft("保留原文", window, cx);
        p.send(window, cx);
    });
    assert_eq!(h.draft(0), "保留原文");
    assert!(h.pane(0).read_with(h.cx, |p, _| !p.has_pending() && p.sending == 0));
    assert_eq!(h.chat(0).last().unwrap().0, Role::Attention);
    assert!(h.sent("t1").is_empty());
    assert!(h.pane(1).read_with(h.cx, |p, _| !p.locked()));
    assert!(h.chat(1).is_empty());
}

#[gpui::test]
fn concurrent_round_trips_stay_with_their_tab(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1", "t2"]), None, world(&["t1", "t2"]));
    let b = h.id(1);
    for index in 0..2 {
        h.on_pane(index, |p, window, cx| {
            let draft = format!("问题 {}", p.target);
            p.set_draft(&draft, window, cx);
            p.send(window, cx);
        });
    }
    assert_eq!(
        (h.sent("t1"), h.sent("t2")),
        (vec!["English 问题 t1".to_string()], vec!["English 问题 t2".to_string()])
    );
    h.on_workspace(|w, window, cx| w.focus_tab(b, true, window, cx));

    h.set("t2", |t| t.turn = 1);
    h.on_pane(1, |p, _, cx| p.tick(cx));
    assert!(h.pane(1).read_with(h.cx, |p, _| !p.has_pending() && !p.unread));
    assert!(h.pane(0).read_with(h.cx, |p, _| p.has_pending()));

    h.set("t1", |t| t.turn = 1);
    h.on_pane(0, |p, _, cx| p.tick(cx));
    let reply = |chat: Vec<(Role, String)>| {
        chat.into_iter().filter(|(r, _)| *r == Role::Reply).map(|(_, t)| t).collect::<Vec<_>>()
    };
    assert_eq!(reply(h.chat(0)), ["中文 Answer t1"]);
    assert_eq!(reply(h.chat(1)), ["中文 Answer t2"]);
    // A reply that lands in a hidden tab is marked unread.
    assert!(h.pane(0).read_with(h.cx, |p, _| p.unread));
    assert_eq!(h.workspace.read_with(h.cx, |w, _| w.active()), Some(b));
}

#[gpui::test]
fn logs_are_open_and_skip_success_notices(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1"]), None, world(&["t1"]));
    h.on_pane(0, |p, window, cx| {
        p.set_draft("你好", window, cx);
        p.send(window, cx);
    });
    h.set("t1", |t| t.turn = 1);
    h.on_pane(0, |p, _, cx| p.tick(cx));
    let logs = h.logs(0);
    assert_eq!(logs.len(), 1);
    let (entries, expanded) = &logs[0];
    assert!(expanded);
    assert!(entries.iter().any(|e| e.contains("English 你好")));
    assert!(!entries.iter().any(|e| e.contains("本轮已完成")));
    assert!(!h.chat(0).iter().any(|(_, t)| t.contains("English")));
}

#[gpui::test]
fn duplicate_terminal_focuses_existing_and_rebinding_opens_new_tab(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1", "t2"]), None, world(&["t1", "t2", "t3"]));
    let (first, second) = (h.id(0), h.id(1));
    let row = |address: &str| TerminalRow {
        address: address.into(),
        groups: vec![],
        name: address.to_uppercase(),
        app: String::new(),
        status: String::new(),
        cwd: String::new(),
        raw: Value::Null,
    };
    assert_eq!(h.on_workspace(|w, window, cx| w.new_tab("t1", None, false, window, cx)), first);
    h.on_workspace(|w, window, cx| w.bind_target(second, row("t1"), window, cx));
    assert_eq!(h.workspace.read_with(h.cx, |w, _| w.active()), Some(first));
    assert_eq!(h.pane(1).read_with(h.cx, |p, _| p.target.clone()), "t2");
    h.on_workspace(|w, window, cx| w.bind_target(first, row("t3"), window, cx));
    assert_eq!(h.workspace.read_with(h.cx, |w, _| w.layout().len()), 3);
    assert_eq!(h.pane(0).read_with(h.cx, |p, _| p.target.clone()), "t1");
    let active = h.workspace.read_with(h.cx, |w, _| w.active()).unwrap();
    let pane = h.workspace.read_with(h.cx, |w, _| w.pane(active).unwrap().clone());
    assert_eq!(pane.read_with(h.cx, |p, _| (p.target.clone(), p.target_name.clone())), ("t3".into(), "T3".into()));
}

#[gpui::test]
fn missing_terminal_disables_only_its_tab(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1", "t2"]), None, world(&["t1", "t2"]));
    h.on_pane(0, |p, _, cx| p.list_targets(false, cx));
    h.world.lock().unwrap().remove("t1");
    h.on_pane(1, |p, _, cx| p.list_targets(false, cx));
    assert!(h.pane(0).read_with(h.cx, |p, _| p.offline && p.target == "t1"));
    assert!(h.pane(1).read_with(h.cx, |p, _| !p.offline));
}

#[gpui::test]
fn messages_send_while_replies_are_pending_and_resolve_in_order(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1", "t2"]), None, world(&["t1", "t2"]));
    let first = h.id(0);
    h.on_pane(0, |p, window, cx| {
        p.set_draft("一", window, cx);
        p.send(window, cx);
        p.set_draft("二", window, cx);
        p.send(window, cx);
    });
    assert_eq!(h.sent("t1"), ["English 一", "English 二"]);
    assert_eq!(h.pane(0).read_with(h.cx, |p, _| p.pending()), 2);
    // A third message goes out while the tracker is polling.
    let pane = h.pane(0);
    h.cx.update(|window, cx| {
        pane.update(cx, |p, cx| {
            p.tick(cx);
            p.set_draft("三", window, cx);
            p.send(window, cx);
            assert_eq!((p.tracking, p.sending), (1, 1));
        })
    });
    h.cx.run_until_parked();
    assert_eq!(h.sent("t1").len(), 3);
    assert!(!h.on_workspace(|w, window, cx| w.close_tab(first, window, cx)));

    h.set("t1", |t| t.turn = 1);
    h.on_pane(0, |p, _, cx| p.tick(cx));
    let replies: Vec<String> = h.chat(0).into_iter().filter(|(r, _)| *r == Role::Reply).map(|(_, t)| t).collect();
    assert_eq!(replies, ["中文 Answer t1"]);
    assert_eq!(h.pane(0).read_with(h.cx, |p, _| p.pending()), 2);
    // The reply is logged with its own round, not the newest one.
    let logs = h.logs(0);
    assert!(logs[0].0.iter().any(|e| e.contains("中文译文")));
    assert!(!logs[2].0.iter().any(|e| e.contains("中文译文")));

    h.on_pane(0, |p, _, cx| p.abandon(cx));
    assert!(h.on_workspace(|w, window, cx| w.close_tab(first, window, cx)));
    assert_eq!(h.workspace.read_with(h.cx, |w, _| w.layout().len()), 1);
}

fn log_scroll(h: &mut Harness, index: usize) -> (Pixels, Pixels) {
    h.pane(0).read_with(h.cx, |p, _| match &p.items[index] {
        ChatItem::Log { scroll, .. } => (scroll.offset().y, scroll.max_offset().y),
        _ => unreachable!(),
    })
}

fn log_indices(h: &mut Harness) -> Vec<usize> {
    h.pane(0).read_with(h.cx, |p, _| {
        p.items.iter().enumerate().filter(|(_, i)| matches!(i, ChatItem::Log { .. })).map(|(i, _)| i).collect()
    })
}

#[gpui::test]
fn logs_keep_a_fixed_height_and_follow_new_entries(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1"]), None, world(&["t1"]));
    let entries: Vec<String> = (0..60).map(|i| format!("entry {i}")).collect();
    let entries: Vec<&str> = entries.iter().map(String::as_str).collect();
    h.on_pane(0, |p, _, _| {
        p.add_demo("短", &["one line"], None);
        p.add_demo("长", &entries, None);
    });
    let [short, long] = log_indices(&mut h)[..] else { panic!() };
    let height = |h: &mut Harness, index: usize| h.bounds(&format!("log-{index}")).size.height;
    assert_eq!(height(&mut h, short), height(&mut h, long));
    let (offset, max) = log_scroll(&mut h, long);
    assert!(max > px(0.) && offset == -max);

    let before = height(&mut h, long);
    h.on_pane(0, |p, _, _| (0..20).for_each(|i| p.log_to(long, format!("more {i}"))));
    h.draw();
    let (offset, grown) = log_scroll(&mut h, long);
    assert_eq!(height(&mut h, long), before);
    assert!(grown > max && offset == -grown);

    // Scrolled back, the log stays put; reopened, it shows the end again.
    h.on_pane(0, |p, _, _| {
        if let ChatItem::Log { scroll, .. } = &p.items[long] {
            scroll.set_offset(point(px(0.), px(0.)));
        }
        p.log_to(long, "later".into());
    });
    h.draw();
    assert_eq!(log_scroll(&mut h, long).0, px(0.));
    h.on_pane(0, |p, _, cx| {
        p.toggle_log(long, cx);
        p.toggle_log(long, cx);
    });
    h.draw();
    let (offset, max) = log_scroll(&mut h, long);
    assert_eq!(offset, -max);
}

#[gpui::test]
fn the_wheel_scrolls_a_log_first_then_the_chat(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1"]), None, world(&["t1"]));
    let entries: Vec<String> = (0..60).map(|i| format!("entry {i}")).collect();
    let entries: Vec<&str> = entries.iter().map(String::as_str).collect();
    h.on_pane(0, |p, _, _| {
        for i in 0..3 {
            p.add_demo(&format!("问题 {i}"), &entries, Some("回答"));
        }
    });
    let log = *log_indices(&mut h).last().unwrap();
    let wheel = |h: &mut Harness, dy: f32| {
        let at = h.bounds(&format!("log-{log}")).center();
        h.cx.simulate_event(ScrollEvent {
            position: at,
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            ..Default::default()
        });
        h.draw();
    };
    let top = |h: &mut Harness| h.bounds(&format!("log-{log}")).top();

    let (chat_before, (log_before, _)) = (top(&mut h), log_scroll(&mut h, log));
    wheel(&mut h, 60.);
    assert!(log_scroll(&mut h, log).0 > log_before);
    assert_eq!(top(&mut h), chat_before);

    h.on_pane(0, |p, _, _| {
        if let ChatItem::Log { scroll, .. } = &p.items[log] {
            scroll.set_offset(point(px(0.), px(0.)));
        }
    });
    h.draw();
    wheel(&mut h, 60.);
    assert!(top(&mut h) > chat_before);
}

#[gpui::test]
fn preferences_restore_bindings_but_never_chat_or_drafts(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("preferences.json");
    std::fs::write(&path, r#"{"target":"legacy","model":"m1","effort":"max"}"#).unwrap();
    let prefs = prefs::load(&path, None, "herdr-app".into());
    let mut h = open(cx, prefs, Some(path.clone()), world(&["legacy", "second"]));
    assert_eq!(
        h.pane(0).read_with(h.cx, |p, _| (p.target.clone(), p.models.effort.clone())),
        ("legacy".into(), "max".into())
    );
    h.on_pane(0, |p, window, cx| {
        p.set_draft("private draft", window, cx);
        p.add_demo("private question", &["private log"], Some("private answer"));
    });
    let second = h.on_workspace(|w, window, cx| w.new_tab("second", None, false, window, cx));
    let target = h.right_of_body();
    h.drag(second, target, true);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("private"));
    let saved = prefs::load(&path, None, "x".into());
    assert!(saved.split);
    assert_eq!(saved.tabs.iter().map(|t| t.target.as_str()).collect::<Vec<_>>(), ["legacy", "second"]);
    assert_eq!(saved.tabs[0].model, "m1");
}

#[gpui::test]
fn dragging_right_splits_and_back_onto_the_strip_merges(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1", "t2"]), None, world(&["t1", "t2"]));
    let (first, second) = (h.id(0), h.id(1));
    h.on_pane(0, |p, window, cx| p.set_draft("草稿 A", window, cx));
    let target = h.right_of_body();
    h.drag(first, target, true);
    assert!(h.workspace.read_with(h.cx, |w, _| w.layout().split() && w.layout().group_of(first) == Some(1)));
    assert_eq!(h.workspace.read_with(h.cx, |w, _| w.active()), Some(first));

    let strip = h.bounds("strip-0");
    h.drag(first, point(strip.left() + px(8.), strip.center().y), true);
    let layout = h.workspace.read_with(h.cx, |w, _| w.layout().clone());
    assert!(!layout.split());
    assert_eq!(layout.tabs(0), [first, second]);
    assert_eq!(h.draft(0), "草稿 A");
}

#[gpui::test]
fn escape_or_release_outside_cancels_a_drag(cx: &mut TestAppContext) {
    let mut h = open(cx, tabs(&["t1", "t2"]), None, world(&["t1", "t2"]));
    let second = h.id(1);
    let target = h.right_of_body();
    h.drag(second, target, false);
    assert!(h.cx.update(|_, cx| cx.has_active_drag()));
    h.cx.simulate_keystrokes("escape");
    h.cx.run_until_parked();
    assert!(!h.cx.update(|_, cx| cx.has_active_drag()));
    h.cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
    h.cx.run_until_parked();
    assert!(!h.workspace.read_with(h.cx, |w, _| w.layout().split()));

    h.drag(second, point(px(-40.), px(-40.)), true);
    let layout = h.workspace.read_with(h.cx, |w, _| w.layout().clone());
    assert!(!layout.split());
    assert_eq!(layout.group_of(second), Some(0));
}

#[gpui::test]
fn reordering_within_the_strip_persists(cx: &mut TestAppContext) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("preferences.json");
    let mut h = open(cx, tabs(&["t1", "t2", "t3"]), Some(path.clone()), world(&["t1", "t2", "t3"]));
    let third = h.id(2);
    let strip = h.bounds("strip-0");
    h.drag(third, point(strip.left() + px(8.), strip.center().y), true);
    let order: Vec<String> = (0..3).map(|i| h.pane(i).read_with(h.cx, |p, _| p.target.clone())).collect();
    assert_eq!(order, ["t3", "t1", "t2"]);
    let saved = prefs::load(&path, None, "x".into());
    assert_eq!(saved.tabs.iter().map(|t| t.target.as_str()).collect::<Vec<_>>(), ["t3", "t1", "t2"]);
    assert_eq!(saved.active, 0);
}
