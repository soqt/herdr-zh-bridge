//! Terminal picker grouped by machine / workspace / tab. Headings cannot be
//! chosen, and addresses always come from records, never from labels.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, Disableable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use indexmap::IndexMap;

use super::theme::{BORDER, FIELD, MUTED, SELECTION, TEXT, color, to_px, u};
use crate::core::catalog::TerminalRow;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Group { depth: usize, label: String },
    Terminal { depth: usize, row: usize },
}

enum Child {
    Group(Vec<String>),
    Row(usize),
}

/// The visible tree, flattened in source order. Rows sharing a group path
/// appear under one heading even when they are not adjacent.
pub fn tree_lines(rows: &[TerminalRow], query: &str) -> Vec<Line> {
    let mut nodes: IndexMap<Vec<String>, (String, Vec<Child>)> = IndexMap::new();
    let mut root = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if !row.matches(query) {
            continue;
        }
        let mut path: Vec<String> = Vec::new();
        for (key, label) in &row.groups {
            let parent = path.clone();
            path.push(key.clone());
            if !nodes.contains_key(&path) {
                nodes.insert(path.clone(), (label.clone(), Vec::new()));
                let child = Child::Group(path.clone());
                match nodes.get_mut(&parent) {
                    Some((_, children)) => children.push(child),
                    None => root.push(child),
                }
            }
        }
        match nodes.get_mut(&path) {
            Some((_, children)) => children.push(Child::Row(index)),
            None => root.push(Child::Row(index)),
        }
    }
    fn flatten(
        children: &[Child],
        depth: usize,
        nodes: &IndexMap<Vec<String>, (String, Vec<Child>)>,
        out: &mut Vec<Line>,
    ) {
        for child in children {
            match child {
                Child::Row(row) => out.push(Line::Terminal { depth, row: *row }),
                Child::Group(path) => {
                    let (label, children) = &nodes[path];
                    out.push(Line::Group { depth, label: label.clone() });
                    flatten(children, depth + 1, nodes, out);
                }
            }
        }
    }
    let mut lines = Vec::new();
    flatten(&root, 0, &nodes, &mut lines);
    lines
}

type Choose = Rc<dyn Fn(TerminalRow, &mut Window, &mut App)>;

pub struct TerminalPicker {
    rows: Vec<TerminalRow>,
    pub query: Entity<InputState>,
    lines: Vec<Line>,
    selected: Option<usize>,
    choose: Choose,
    _subscription: Subscription,
}

impl TerminalPicker {
    pub fn new(
        rows: Vec<TerminalRow>,
        selected: &str,
        choose: Choose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("搜索名称、目录、ID 或任意详情字段"));
        let subscription = cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::Change => this.filter(cx),
            InputEvent::PressEnter { .. } => this.accept(window, cx),
            _ => {}
        });
        let selected = rows.iter().position(|r| !selected.is_empty() && r.address == selected);
        let lines = tree_lines(&rows, "");
        TerminalPicker { rows, query, lines, selected, choose, _subscription: subscription }
    }

    fn filter(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        self.lines = tree_lines(&self.rows, &query);
        // A selection hidden by the search cannot be accepted.
        if let Some(row) = self.selected
            && !self.lines.iter().any(|l| matches!(l, Line::Terminal { row: r, .. } if *r == row))
        {
            self.selected = None;
        }
        cx.notify();
    }

    pub fn select(&mut self, row: usize, cx: &mut Context<Self>) {
        self.selected = Some(row);
        cx.notify();
    }

    pub fn accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected.map(|i| self.rows[i].clone()) else { return };
        window.close_dialog(cx);
        (self.choose)(row, window, cx);
    }

    pub fn visible_terminals(&self) -> Vec<&TerminalRow> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                Line::Terminal { row, .. } => Some(&self.rows[*row]),
                Line::Group { .. } => None,
            })
            .collect()
    }

    pub fn details(&self) -> String {
        self.selected.map(|i| serde_json::to_string_pretty(&self.rows[i].raw).unwrap_or_default()).unwrap_or_default()
    }

    fn cell(text: impl Into<SharedString>, width: f32) -> impl IntoElement {
        div().flex_shrink_0().w(u(width)).pr(u(10)).truncate().child(text.into())
    }
}

impl Render for TerminalPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.visible_terminals().len();
        let lines: Vec<_> = self
            .lines
            .iter()
            .enumerate()
            .map(|(index, line)| match line {
                Line::Group { depth, label } => h_flex()
                    .id(("group", index))
                    .h(u(28))
                    .pl(u(10. + 16. * *depth as f32))
                    .text_size(u(12.5))
                    .text_color(color(MUTED))
                    .child(div().truncate().child(label.clone()))
                    .into_any_element(),
                Line::Terminal { depth, row } => {
                    let record = &self.rows[*row];
                    let row = *row;
                    let selected = self.selected == Some(row);
                    h_flex()
                        .id(("terminal", index))
                        .h(u(32))
                        .pl(u(10. + 16. * *depth as f32))
                        .rounded(u(4))
                        .text_size(u(13))
                        .when(selected, |d| d.bg(color(SELECTION)))
                        .when(!selected, |d| d.hover(|s| s.bg(cx.theme().list_hover)))
                        .child(Self::cell(record.name.clone(), 240. - 16. * *depth as f32))
                        .child(Self::cell(record.app.clone(), 110.))
                        .child(Self::cell(record.status.clone(), 80.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pr(u(10))
                                .truncate()
                                .text_color(color(MUTED))
                                .child(record.cwd.clone()),
                        )
                        .child(Self::cell(record.address.clone(), 150.))
                        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            this.select(row, cx);
                            if event.click_count() >= 2 {
                                this.accept(window, cx);
                            }
                        }))
                        .into_any_element()
                }
            })
            .collect();
        let header = h_flex()
            .h(u(26))
            .pl(u(14))
            .pr(u(4))
            .text_size(u(12))
            .text_color(color(MUTED))
            .child(Self::cell("Herdr 层级 / 终端名称", 240.))
            .child(Self::cell("程序", 110.))
            .child(Self::cell("状态", 80.))
            .child(div().flex_1().child("完整工作目录"))
            .child(Self::cell("终端 ID", 150.));
        let details = self.details();
        v_flex()
            .w_full()
            .gap(u(8))
            .text_color(color(TEXT))
            .child(
                div()
                    .text_size(u(12.5))
                    .text_color(color(MUTED))
                    .child("按机器 / 工作区 / 标签页分组 · 搜索名称、目录、ID 或任意详情字段"),
            )
            .child(Input::new(&self.query).cleanable(true))
            .child(
                v_flex()
                    .w_full()
                    .rounded(u(6))
                    .border_1()
                    .border_color(color(BORDER))
                    .bg(color(FIELD))
                    .child(header)
                    .child(
                        div().id("picker-list").h(u(300)).overflow_y_scroll().px(u(4)).pb(u(4)).children(lines).when(
                            count == 0,
                            |d| {
                                d.child(
                                    div().p(u(16)).text_size(u(13)).text_color(color(MUTED)).child("没有匹配的终端。"),
                                )
                            },
                        ),
                    ),
            )
            .child(div().text_size(u(12)).text_color(color(MUTED)).child("完整记录 · 可选择、复制；没有删除字段"))
            .child(
                div()
                    .id("picker-details")
                    .h(u(130))
                    .overflow_y_scroll()
                    .px(u(10))
                    .py(u(8))
                    .rounded(u(6))
                    .border_1()
                    .border_color(color(BORDER))
                    .bg(color(FIELD))
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(u(12))
                    .text_color(color(MUTED))
                    .when(details.is_empty(), |d| d.child("选择终端后显示它的完整记录。"))
                    .when(!details.is_empty(), |d| {
                        d.child(gpui_base::SelectableText::new("picker-details-text", SharedString::from(details)))
                    }),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap(u(8))
                    .child(
                        div()
                            .flex_1()
                            .text_size(u(12.5))
                            .text_color(color(MUTED))
                            .child(format!("{count} / {} 个终端", self.rows.len())),
                    )
                    .child(
                        Button::new("picker-cancel")
                            .ghost()
                            .label("取消")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("picker-accept")
                            .primary()
                            .label("使用此终端")
                            .disabled(self.selected.is_none())
                            .on_click(cx.listener(|this, _, window, cx| this.accept(window, cx))),
                    ),
            )
            .text_size(u(13))
    }
}

pub fn open(
    rows: Vec<TerminalRow>,
    selected: String,
    title: SharedString,
    window: &mut Window,
    cx: &mut App,
    choose: impl Fn(TerminalRow, &mut Window, &mut App) + 'static,
) {
    let picker = cx.new(|cx| TerminalPicker::new(rows, &selected, Rc::new(choose), window, cx));
    let first = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            picker.read(cx).query.focus_handle(cx).focus(window, cx);
        }
        let width = to_px(900., window).min(window.viewport_size().width - px(48.));
        dialog.title(title.clone()).w(width).keyboard(true).overlay_closable(true).child(picker.clone())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::catalog::terminal_catalog;
    use crate::core::catalog::tests::{records, world};

    #[test]
    fn hierarchy_keeps_every_terminal_under_its_groups() {
        let rows = terminal_catalog(&records(), &world());
        let lines = tree_lines(&rows, "");
        let terminals = lines.iter().filter(|l| matches!(l, Line::Terminal { .. })).count();
        assert_eq!(terminals, 5);
        assert_eq!(lines[0], Line::Group { depth: 0, label: "本机".into() });
        // Two tabs share the name 开发 but stay separate headings.
        let tabs = lines.iter().filter(|l| matches!(l, Line::Group { label, .. } if label == "开发")).count();
        assert_eq!(tabs, 3);
        let sub_agent =
            lines.iter().position(|l| matches!(l, Line::Group { label, .. } if label.starts_with("子 agent"))).unwrap();
        assert_eq!(lines[sub_agent + 1], Line::Terminal { depth: 4, row: 1 });
    }

    #[test]
    fn search_covers_raw_details() {
        let rows = terminal_catalog(&records(), &world());
        let only = |query: &str| -> Vec<String> {
            tree_lines(&rows, query)
                .into_iter()
                .filter_map(|l| match l {
                    Line::Terminal { row, .. } => Some(rows[row].address.clone()),
                    Line::Group { .. } => None,
                })
                .collect()
        };
        assert_eq!(only("remote/t1"), ["remote/t1"]);
        assert_eq!(only("KEEP"), ["t1"]);
        assert!(only("nothing-matches").is_empty());
    }
}
