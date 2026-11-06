//! `--preview`: demo spaces and conversations. No CLI calls, nothing saved.

use gpui::{Context, Window};
use serde_json::json;

use super::workspace::Workspace;
use crate::core::catalog::{Catalog, herdr_spaces, terminal_catalog};
use crate::core::prefs::{Axis, Preferences, SpacePrefs, TabPrefs, Tree};

fn tab(target: &str, name: &str) -> TabPrefs {
    TabPrefs { target: target.into(), name: name.into(), model: "GPT-6.1 Sol".into(), effort: "high".into() }
}

/// Herdr's window in the demo: two machines, four spaces.
pub fn catalog() -> Catalog {
    let terminal = |address: &str| json!({"terminal": address});
    let world = json!({"spaces": [
        {"id": "s1", "name": "herdr-zh-bridge", "machine": null, "folder": "/Users/demo/herdr-zh-bridge", "tabs": [
            {"id": "tab1", "name": "1", "terminals": [terminal("preview-one"), terminal("preview-two")]},
            {"id": "tab2", "name": "2", "terminals": [terminal("preview-three")]}]},
        {"id": "s2", "name": "文档", "machine": null, "folder": "/Users/demo/docs", "tabs": [
            {"id": "tab3", "name": "1", "terminals": [terminal("preview-five"), terminal("preview-six")]}]},
        {"id": "s3", "name": "playground", "machine": null, "folder": "/Users/demo/playground", "tabs": [
            {"id": "tab4", "name": "1", "terminals": [terminal("preview-seven"), terminal("preview-eight")]}]},
        {"id": "box/s1", "name": "api-server", "machine": "build-box", "folder": "/srv/api", "tabs": [
            {"id": "box/tab1", "name": "1", "terminals": [terminal("build-box/preview-four")]}]}
    ]});
    let record = |address: &str, name: &str, app: &str, cwd: &str| json!({"address": address, "name": name, "agent": app, "cwd": cwd, "state": "live"});
    let records = vec![
        record("preview-one", "Codex · 界面设计", "codex", "/Users/demo/herdr-zh-bridge"),
        record("preview-two", "Claude · 代码审查", "claude", "/Users/demo/herdr-zh-bridge"),
        record("preview-three", "Grok · API 文档", "grok", "/Users/demo/herdr-zh-bridge"),
        record("build-box/preview-four", "Codex · 部署脚本", "codex", "/srv/api"),
        record("preview-five", "Claude · README", "claude", "/Users/demo/docs"),
        record("preview-six", "Codex · 翻译校对", "codex", "/Users/demo/docs"),
        record("preview-seven", "Claude Code", "claude", "/Users/demo/playground"),
        record("preview-eight", "zsh", "zsh", "/Users/demo/playground"),
    ];
    Catalog { rows: terminal_catalog(&records, &world), spaces: Some(herdr_spaces(&world)), warning: String::new() }
}

/// The demo's saved state: three of its spaces have conversations, the
/// first two panes side by side.
pub fn preferences() -> Preferences {
    let mut prefs = Preferences::defaults("herdr-app".into());
    let space = |key: &str, name: &str, layout: Option<Tree<TabPrefs>>| SpacePrefs::new(key, name, layout);
    let mut remote =
        space("box/s1", "api-server", Some(Tree::pane(vec![tab("build-box/preview-four", "Codex · 部署脚本")])));
    remote.machine = Some("build-box".into());
    prefs.spaces = vec![
        space(
            "s1",
            "herdr-zh-bridge",
            Some(Tree::Split {
                axis: Axis::Row,
                children: vec![
                    (
                        0.5,
                        Tree::pane(vec![
                            tab("preview-one", "Codex · 界面设计"),
                            tab("preview-two", "Claude · 代码审查"),
                        ]),
                    ),
                    (0.5, Tree::pane(vec![tab("preview-three", "Grok · API 文档")])),
                ],
            }),
        ),
        space(
            "s2",
            "文档",
            Some(Tree::Split {
                axis: Axis::Column,
                children: vec![
                    (0.5, Tree::pane(vec![tab("preview-five", "Claude · README")])),
                    (0.5, Tree::pane(vec![tab("preview-six", "Codex · 翻译校对")])),
                ],
            }),
        ),
        space("s3", "playground", None),
        remote,
    ];
    prefs.space = "s1".into();
    prefs
}

pub fn populate(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    workspace.apply_catalog(catalog(), window, cx);
    let tabs: Vec<_> = workspace.spaces().iter().flat_map(|s| s.layout.order()).collect();
    let pane = |i: usize| tabs.get(i).and_then(|id| workspace.pane(*id)).cloned();
    if let Some(first) = pane(0) {
        first.update(cx, |pane, cx| {
            pane.add_demo(
                "把标签页做成固定大小，切换时不要改变布局。",
                &[
                    "英文译文：\nMake the tabs fixed-size and keep the layout stable when switching.",
                    "已发送到 preview-one，等待回复…",
                    "提取的回答原文：\nUpdated the tab bar: every tab now shares one width and height.",
                ],
                Some(
                    "已调整标签栏：\n\n所有标签使用相同的宽度和高度。选中状态只改变颜色，任务状态用指示点显示。\n\n较长名称会自动省略，悬停可以查看完整信息。",
                ),
            );
            pane.set_draft("继续优化输入框的间距", window, cx);
        });
    }
    if let Some(second) = pane(1) {
        second.update(cx, |pane, _| pane.unread = true);
    }
    if let Some(third) = pane(2) {
        third.update(cx, |pane, _| {
            pane.add_demo(
                "检查 API 的错误处理。",
                &["英文译文：\nReview the API error handling.", "已发送到 preview-three，等待回复…"],
                Some("建议统一错误响应结构，并为超时、权限不足和请求限流分别提供明确的处理方式。"),
            );
        });
    }
    if let Some(last) = tabs.last().and_then(|id| workspace.pane(*id)).cloned() {
        last.update(cx, |pane, _| pane.unread = true);
    }
    if let Some(first) = tabs.first() {
        workspace.focus_tab(*first, true, window, cx);
    }
}
