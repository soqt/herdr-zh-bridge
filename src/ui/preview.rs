//! `--preview`: demo tabs and conversations. No CLI calls, nothing saved.

use gpui::{Context, Window};

use super::workspace::Workspace;
use crate::core::prefs::{Preferences, TabPrefs};

pub fn preferences() -> Preferences {
    let tab = |target: &str, name: &str, group| TabPrefs {
        target: target.into(),
        name: name.into(),
        model: "GPT-6.1 Sol".into(),
        effort: "high".into(),
        group,
    };
    let mut prefs = Preferences::defaults("herdr-app".into());
    prefs.tabs = vec![
        tab("preview-one", "Codex · 界面设计", 0),
        tab("preview-two", "Claude · 代码审查", 0),
        tab("preview-three", "Grok · API 文档", 1),
    ];
    prefs.split = true;
    prefs
}

pub fn populate(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let order = workspace.layout().order();
    let pane = |i: usize| workspace.pane(order[i]).cloned();
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
    if let Some(first) = order.first() {
        workspace.focus_tab(*first, true, window, cx);
    }
}
