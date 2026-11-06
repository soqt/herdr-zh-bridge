//! Multi-terminal workspace: fixed-size tabs in up to two side-by-side groups.
//! Selection, status and splitting never change the window size.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, DragMoveEvent, Entity, FocusHandle, InteractiveElement,
    IntoElement, ParentElement, Pixels, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, canvas, div, point, prelude::FluentBuilder, px, relative,
};
use gpui_component::{
    IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use serde_json::Value;

use super::conversation::{ConversationPane, PaneEvent, TabStatus};
use super::layout::{DropTarget, Layout, TabId};
use super::theme::{
    ACCENT, BG, BORDER, CARD, CONTROL_HOVER, CONTROL_PRESSED, FOCUS, HOVER, MUTED, SELECTED, SELECTION, TEXT, color,
    to_px, u,
};
use super::{CancelDrag, CloseTab, NewTab, NextTab, OpenSettings, Settings, WORKSPACE_CONTEXT, picker, settings};
use crate::core::catalog::TerminalRow;
use crate::core::prefs::{self, Preferences, TabPrefs};

pub const TAB_WIDTH: f32 = 184.;
const TAB_HEIGHT: f32 = 36.;
const HEADER_HEIGHT: f32 = 44.;
const TAB_GAP: f32 = 3.;
const PLUS_WIDTH: f32 = 32.;
/// The two scroll buttons shown only while the strip overflows; must match
/// their rendered width, or the overflow check flips every frame.
const NAV_WIDTH: f32 = 2. * 28.;
const EDGE: f32 = 28.;
const SCROLL_STEP: f32 = 40.;
const MIN_GROUP: f32 = 300.;
/// The split drop zone starts at this fraction of an unsplit body.
const SPLIT_ZONE: f32 = 0.62;

/// A tab being dragged.
#[derive(Clone)]
pub struct TabDrag {
    pub tab: TabId,
    title: SharedString,
}

struct TabDragPreview(SharedString);

impl Render for TabDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(u(TAB_WIDTH - 2. * TAB_GAP))
            .h(u(TAB_HEIGHT))
            .px(u(12))
            .flex()
            .items_center()
            .rounded(u(6))
            .bg(color(SELECTED).opacity(0.92))
            .border_1()
            .border_color(color(ACCENT))
            .text_size(u(13))
            .text_color(color(TEXT))
            .child(div().truncate().child(self.0.clone()))
    }
}

#[derive(Clone)]
struct SashDrag;

impl Render for SashDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// What the strip needs from a pane, read once per render.
struct TabInfo {
    id: TabId,
    title: SharedString,
    tooltip: SharedString,
    status: TabStatus,
    closable: bool,
    selected: bool,
}

pub struct Workspace {
    panes: HashMap<TabId, Entity<ConversationPane>>,
    subscriptions: HashMap<TabId, [Subscription; 2]>,
    layout: Layout,
    active: Option<TabId>,
    next_id: TabId,
    targets: Vec<TerminalRow>,
    models: Vec<Value>,
    prefs_path: Option<PathBuf>,
    ready: bool,
    focus: FocusHandle,
    strips: [ScrollHandle; 2],
    overflow: [bool; 2],
    split_ratio: f32,
    drag_hover: Option<DropTarget>,
    /// The dragged tab and the pointer x inside a strip viewport.
    strip_pointer: Option<(usize, Pixels, TabId)>,
    edge_scroll: Option<(usize, f32)>,
    edge_task: Option<Task<()>>,
    _tick: Task<()>,
}

impl Workspace {
    pub fn new(
        prefs: Preferences,
        prefs_path: Option<PathBuf>,
        target: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tick = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        });
        let mut this = Workspace {
            panes: HashMap::new(),
            subscriptions: HashMap::new(),
            layout: Layout::default(),
            active: None,
            next_id: 1,
            targets: Vec::new(),
            models: Vec::new(),
            prefs_path,
            ready: false,
            focus: cx.focus_handle(),
            strips: [ScrollHandle::new(), ScrollHandle::new()],
            overflow: [false; 2],
            split_ratio: 0.5,
            drag_hover: None,
            strip_pointer: None,
            edge_scroll: None,
            edge_task: None,
            _tick: tick,
        };
        let mut placed = Vec::new();
        for tab in &prefs.tabs {
            placed.push((this.create_pane(tab, window, cx), tab.group));
        }
        if placed.is_empty() {
            placed.push((this.create_pane(&TabPrefs::default(), window, cx), 0));
        }
        this.layout = Layout::restore(placed, prefs.split);
        let order = this.layout.order();
        let mut selected = order.get(prefs.active).copied().unwrap_or(order[0]);
        if !target.is_empty() {
            selected = match this.find_target(target, cx) {
                Some(existing) => existing,
                None if order.len() == 1 && this.panes[&order[0]].read(cx).target.is_empty() => {
                    let row = TerminalRow {
                        address: target.into(),
                        groups: Vec::new(),
                        name: target.into(),
                        app: String::new(),
                        status: String::new(),
                        cwd: String::new(),
                        raw: Value::Null,
                    };
                    this.panes[&order[0]].update(cx, |pane, cx| pane.apply_target(&row, cx));
                    order[0]
                }
                None => {
                    let prefs = TabPrefs { target: target.into(), ..this.inherited(cx) };
                    let id = this.create_pane(&prefs, window, cx);
                    this.layout.add(id, 0, None);
                    id
                }
            };
        }
        this.ready = true;
        this.focus_tab(selected, true, window, cx);
        this
    }

    // ----- Tabs -----

    fn create_pane(&mut self, prefs: &TabPrefs, window: &mut Window, cx: &mut Context<Self>) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        let pane = cx.new(|cx| ConversationPane::new(id, prefs, window, cx));
        if !self.models.is_empty() {
            let models = self.models.clone();
            pane.update(cx, |pane, cx| pane.apply_models(&models, window, cx));
        }
        let subscriptions =
            [cx.subscribe_in(&pane, window, Self::on_pane_event), cx.observe(&pane, |_, _, cx| cx.notify())];
        self.panes.insert(id, pane);
        self.subscriptions.insert(id, subscriptions);
        id
    }

    /// A new tab keeps the active tab's model and effort.
    fn inherited(&self, cx: &App) -> TabPrefs {
        self.active
            .and_then(|id| self.panes.get(&id))
            .map(|pane| {
                let p = pane.read(cx).preferences(0);
                TabPrefs { model: p.model, effort: p.effort, ..TabPrefs::default() }
            })
            .unwrap_or_default()
    }

    pub fn find_target(&self, address: &str, cx: &App) -> Option<TabId> {
        if address.is_empty() {
            return None;
        }
        self.layout.order().into_iter().find(|id| self.panes[id].read(cx).target == address)
    }

    pub fn pane(&self, id: TabId) -> Option<&Entity<ConversationPane>> {
        self.panes.get(&id)
    }

    pub fn active(&self) -> Option<TabId> {
        self.active
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn targets(&self) -> &[TerminalRow] {
        &self.targets
    }

    pub fn new_tab(
        &mut self,
        target: &str,
        group: Option<usize>,
        choose: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TabId {
        if let Some(existing) = self.find_target(target, cx) {
            self.focus_tab(existing, true, window, cx);
            return existing;
        }
        let group = group
            .or_else(|| self.active.and_then(|a| self.layout.group_of(a)))
            .unwrap_or(0)
            .min(usize::from(self.layout.split()));
        let prefs = TabPrefs { target: target.into(), ..self.inherited(cx) };
        let id = self.create_pane(&prefs, window, cx);
        self.layout.add(id, group, None);
        self.focus_tab(id, true, window, cx);
        if choose {
            self.open_picker(id, window, cx);
        }
        id
    }

    pub fn focus_tab(&mut self, id: TabId, focus_input: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.panes.get(&id).cloned() else { return };
        self.layout.select(id);
        self.active = Some(id);
        pane.update(cx, |pane, cx| {
            pane.unread = false;
            if focus_input {
                pane.focus_input(window, cx);
            }
            cx.notify();
        });
        if !focus_input && window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
        self.scroll_selection_into_view();
        self.sync_visibility(cx);
        self.save_preferences(cx);
        cx.notify();
    }

    fn sync_visibility(&mut self, cx: &mut Context<Self>) {
        for (id, pane) in &self.panes {
            let visible = self.layout.is_visible(*id);
            if pane.read(cx).visible != visible {
                pane.update(cx, |pane, _| pane.visible = visible);
            }
        }
    }

    fn scroll_selection_into_view(&self) {
        for group in 0..2 {
            let tabs = self.layout.tabs(group);
            if let Some(index) = self.layout.selected(group).and_then(|s| tabs.iter().position(|t| *t == s)) {
                // The last tab also reveals the + beside it.
                let item = if index + 1 == tabs.len() { tabs.len() } else { index };
                self.strips[group].scroll_to_item(item);
            }
        }
    }

    /// Closes a tab unless it is translating or waiting for a reply.
    pub fn close_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(pane) = self.panes.get(&id).cloned() else { return false };
        if pane.read(cx).locked() {
            return false;
        }
        let group = self.layout.remove(id).unwrap_or(0);
        self.layout.collapse_empty_group();
        pane.update(cx, |pane, _| pane.dispose());
        self.panes.remove(&id);
        self.subscriptions.remove(&id);
        let was_active = self.active == Some(id) || self.active.is_none();
        if self.active == Some(id) {
            self.active = None;
        }
        if self.layout.is_empty() {
            self.new_tab("", Some(0), false, window, cx);
        } else if was_active {
            let group = group.min(usize::from(self.layout.split()));
            let next = self.layout.selected(group).or_else(|| self.layout.selected(0));
            if let Some(next) = next {
                self.focus_tab(next, true, window, cx);
            }
        } else {
            self.sync_visibility(cx);
            self.save_preferences(cx);
        }
        cx.notify();
        true
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.active.is_some_and(|id| self.close_tab(id, window, cx))
    }

    fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.layout.order();
        if let Some(index) = self.active.and_then(|a| order.iter().position(|t| *t == a)) {
            self.focus_tab(order[(index + 1) % order.len()], true, window, cx);
        }
    }

    /// Binds a chosen terminal. One terminal belongs to one tab; choosing a
    /// different terminal for a bound tab opens a new tab instead of mixing
    /// two histories.
    pub fn bind_target(&mut self, id: TabId, row: TerminalRow, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.panes.get(&id).cloned() else { return };
        if pane.read(cx).locked() {
            return;
        }
        if let Some(existing) = self.find_target(&row.address, cx)
            && existing != id
        {
            self.focus_tab(existing, true, window, cx);
            return;
        }
        let current = pane.read(cx).target.clone();
        let tab = if !current.is_empty() && current != row.address {
            self.new_tab(&row.address, None, false, window, cx)
        } else {
            id
        };
        self.panes[&tab].update(cx, |pane, cx| pane.apply_target(&row, cx));
        self.save_preferences(cx);
        cx.notify();
    }

    pub fn open_picker(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.panes.get(&id).cloned() else { return };
        if pane.read(cx).disposed || pane.read(cx).locked() {
            return;
        }
        if self.targets.is_empty() {
            pane.update(cx, |pane, cx| pane.list_targets(true, cx));
            return;
        }
        let workspace = cx.entity().downgrade();
        let selected = pane.read(cx).target.clone();
        picker::open(self.targets.clone(), selected, window, cx, move |row, window, cx| {
            let _ = workspace.update(cx, |this, cx| this.bind_target(id, row, window, cx));
        });
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        settings::open(cx.entity().downgrade(), window, cx);
    }

    pub fn apply_targets(&mut self, rows: Vec<TerminalRow>, cx: &mut Context<Self>) {
        let by_address: HashMap<String, TerminalRow> = rows.iter().map(|r| (r.address.clone(), r.clone())).collect();
        self.targets = rows;
        for pane in self.panes.values() {
            let target = pane.read(cx).target.clone();
            if let Some(row) = by_address.get(&target) {
                pane.update(cx, |pane, cx| pane.apply_target(row, cx));
            } else if !target.is_empty() {
                pane.update(cx, |pane, cx| pane.set_offline(true, cx));
            }
        }
        self.save_preferences(cx);
    }

    /// Every tab takes the catalog at once; queued jobs keep their model.
    pub fn apply_models(&mut self, records: Vec<Value>, window: &mut Window, cx: &mut Context<Self>) {
        self.models = records;
        for pane in self.panes.values() {
            let models = self.models.clone();
            pane.update(cx, |pane, cx| pane.apply_models(&models, window, cx));
        }
    }

    fn on_pane_event(
        &mut self,
        pane: &Entity<ConversationPane>,
        event: &PaneEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = pane.read(cx).id;
        if !self.panes.contains_key(&id) {
            return;
        }
        match event {
            PaneEvent::Activated => {
                if self.active != Some(id) {
                    self.focus_tab(id, false, window, cx);
                }
            }
            PaneEvent::OpenPicker => self.open_picker(id, window, cx),
            PaneEvent::SavePreferences => self.save_preferences(cx),
            PaneEvent::TargetsLoaded { rows, open_after } => {
                self.apply_targets(rows.clone(), cx);
                if *open_after && !rows.is_empty() {
                    self.open_picker(id, window, cx);
                }
            }
            PaneEvent::ModelsLoaded(records) => self.apply_models(records.clone(), window, cx),
        }
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        for pane in self.panes.values().cloned().collect::<Vec<_>>() {
            pane.update(cx, |pane, cx| pane.tick(cx));
        }
    }

    pub fn save_preferences(&self, cx: &App) {
        let Some(path) = self.prefs_path.as_ref().filter(|_| self.ready) else { return };
        let settings = cx.global::<Settings>();
        let order = self.layout.order();
        let tabs = order
            .iter()
            .map(|id| self.panes[id].read(cx).preferences(self.layout.group_of(*id).unwrap_or(0)))
            .collect();
        let prefs = Preferences {
            hc: settings.hc.clone(),
            codex: settings.codex.clone(),
            herdr: settings.herdr.clone(),
            tabs,
            split: self.layout.split(),
            active: self.active.and_then(|a| order.iter().position(|t| *t == a)).unwrap_or(0),
            zoom: settings.zoom,
        };
        if let Err(error) = prefs::save(path, &prefs) {
            eprintln!("无法保存设置：{error}");
        }
    }

    // ----- Dragging -----

    fn strip_drag_move(
        &mut self,
        group: usize,
        event: &DragMoveEvent<TabDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = event.drag(cx).tab;
        let (position, bounds) = (event.event.position, event.bounds);
        if bounds.contains(&position) {
            let local = position.x - bounds.left();
            self.strip_pointer = Some((group, local, tab));
            self.update_strip_hover(group, window);
            let margin = to_px(EDGE, window);
            self.edge_scroll = if local < margin {
                Some((group, -1.))
            } else if local > bounds.size.width - margin {
                Some((group, 1.))
            } else {
                None
            };
            if self.edge_scroll.is_some() {
                self.ensure_edge_scroll(window, cx);
            }
        } else if self.strip_pointer.is_some_and(|(g, _, _)| g == group) {
            self.strip_pointer = None;
            self.edge_scroll = None;
            if matches!(self.drag_hover, Some(DropTarget::Strip { group: g, .. }) if g == group) {
                self.drag_hover = None;
            }
        }
        cx.notify();
    }

    fn update_strip_hover(&mut self, group: usize, window: &Window) {
        let Some((g, local, tab)) = self.strip_pointer.filter(|(g, _, _)| *g == group) else { return };
        let width = to_px(TAB_WIDTH, window);
        let x = local - self.strips[g].offset().x;
        let count = self.layout.tabs(g).len();
        let index = ((x + width / 2.) / width).floor().clamp(0., count as f32) as usize;
        let target = DropTarget::Strip { group: g, index };
        self.drag_hover = self.layout.accepts(tab, target).then_some(target);
    }

    fn body_drag_move(&mut self, group: usize, event: &DragMoveEvent<TabDrag>, cx: &mut Context<Self>) {
        let tab = event.drag(cx).tab;
        let (position, bounds) = (event.event.position, event.bounds);
        let target = if self.layout.split() { DropTarget::Pane { group } } else { DropTarget::Split };
        let in_zone = bounds.contains(&position)
            && (self.layout.split() || position.x >= bounds.left() + bounds.size.width * SPLIT_ZONE);
        if in_zone && self.layout.accepts(tab, target) {
            self.drag_hover = Some(target);
        } else if self.drag_hover == Some(target) {
            self.drag_hover = None;
        }
        cx.notify();
    }

    /// Keeps scrolling while a dragged tab rests near a strip edge.
    fn ensure_edge_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.edge_task.is_some() {
            return;
        }
        self.edge_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(80)).await;
                let more = this.update_in(cx, |this, window, cx| this.step_edge_scroll(window, cx));
                if !more.unwrap_or(false) {
                    break;
                }
            }
        }));
    }

    fn step_edge_scroll(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !cx.has_active_drag() {
            self.edge_scroll = None;
        }
        let Some((group, direction)) = self.edge_scroll else {
            self.edge_task = None;
            return false;
        };
        self.scroll_strip(group, direction * to_px(SCROLL_STEP, window));
        self.update_strip_hover(group, window);
        cx.notify();
        true
    }

    fn scroll_strip(&self, group: usize, by: Pixels) {
        let strip = &self.strips[group];
        let offset = strip.offset();
        let x = (offset.x - by).min(px(0.)).max(-strip.max_offset().x);
        strip.set_offset(point(x, offset.y));
    }

    fn finish_drop(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.edge_scroll = None;
        self.strip_pointer = None;
        let Some(target) = self.drag_hover.take() else { return };
        if self.layout.drop_tab(tab, target) {
            if target == DropTarget::Split {
                self.split_ratio = 0.5;
            }
            self.focus_tab(tab, true, window, cx);
        }
        cx.notify();
    }

    fn cancel_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.has_active_drag() {
            cx.stop_active_drag(window);
        }
        self.drag_hover = None;
        self.strip_pointer = None;
        self.edge_scroll = None;
        cx.notify();
    }

    // ----- Rendering -----

    fn tabs_menu(
        entity: Entity<Workspace>,
        group: usize,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        move |menu, _, cx| {
            let workspace = entity.read(cx);
            let tabs: Vec<(TabId, String)> = workspace
                .layout
                .order()
                .into_iter()
                .map(|id| (id, workspace.panes[&id].read(cx).full_title()))
                .collect();
            let new_tab = {
                let entity = entity.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| {
                        this.new_tab("", Some(group), true, window, cx);
                    })
                }
            };
            let open_settings = {
                let entity = entity.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| this.open_settings(window, cx))
                }
            };
            let mut menu = menu
                .item(PopupMenuItem::new("新建会话").on_click(new_tab))
                .item(PopupMenuItem::new("设置").on_click(open_settings))
                .separator();
            for (id, title) in tabs {
                let entity = entity.clone();
                menu = menu.item(PopupMenuItem::new(title).on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| this.focus_tab(id, true, window, cx));
                }));
            }
            menu
        }
    }

    fn render_tab(&self, info: TabInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let TabInfo { id, title, tooltip, status, closable, selected } = info;
        let close = div()
            .id(("close", id))
            .flex_shrink_0()
            .size(u(20))
            .rounded(u(4))
            .flex()
            .items_center()
            .justify_center()
            .text_size(u(15))
            .text_color(color(if closable { MUTED } else { BORDER }))
            .child("×")
            .when(closable, |this| {
                this.hover(|s| s.bg(color(CONTROL_HOVER).opacity(0.5)).text_color(color(TEXT)))
                    .active(|s| s.bg(color(CONTROL_PRESSED)))
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab(id, window, cx);
                    }))
            });
        div().id(("tab-slot", id)).flex_shrink_0().w(u(TAB_WIDTH)).h(u(HEADER_HEIGHT)).px(u(TAB_GAP)).py(u(4)).child(
            h_flex()
                .id(("tab", id))
                .debug_selector(move || format!("tab-{id}"))
                .size_full()
                .pl(u(12))
                .pr(u(6))
                .gap(u(8))
                .rounded(u(6))
                .border_1()
                .border_color(color(if selected { BORDER } else { BG }))
                .bg(color(if selected { SELECTED } else { BG }))
                .when(!selected, |this| {
                    this.hover(|s| s.bg(color(HOVER)).border_color(color(HOVER)))
                        .active(|s| s.bg(color(SELECTED)).border_color(color(BORDER)))
                })
                .cursor_pointer()
                .child(div().flex_shrink_0().size(u(6)).rounded_full().bg(color(status.color())))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(u(13))
                        .text_color(color(if selected { TEXT } else { MUTED }))
                        .child(title.clone()),
                )
                .child(close)
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| this.focus_tab(id, true, window, cx)))
                .on_drag(TabDrag { tab: id, title: title.clone() }, |drag, _, _, cx| {
                    cx.new(|_| TabDragPreview(drag.title.clone()))
                }),
        )
    }

    fn render_header(&mut self, group: usize, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.layout.selected(group);
        let infos: Vec<TabInfo> = self
            .layout
            .tabs(group)
            .iter()
            .map(|id| {
                let pane = self.panes[id].read(cx);
                TabInfo {
                    id: *id,
                    title: pane.title().into(),
                    tooltip: pane.full_title().into(),
                    status: pane.status(),
                    closable: pane.closable(),
                    selected: selected == Some(*id),
                }
            })
            .collect();
        let count = infos.len();
        let content = to_px(TAB_WIDTH * count as f32 + 2. * TAB_GAP + PLUS_WIDTH + TAB_GAP, window);
        let nav = to_px(NAV_WIDTH, window);
        let strip = self.strips[group].clone();
        let viewport = strip.bounds().size.width;
        let available = viewport + if self.overflow[group] { nav } else { px(0.) };
        let overflow = viewport > px(0.) && content > available;
        self.overflow[group] = overflow;
        let marker = match self.drag_hover {
            Some(DropTarget::Strip { group: g, index }) if g == group && cx.has_active_drag() => Some(index),
            _ => None,
        };
        let entity = cx.entity();
        let tabs: Vec<AnyElement> =
            infos.into_iter().map(|info| self.render_tab(info, cx).into_any_element()).collect();

        h_flex()
            .w_full()
            .h(u(HEADER_HEIGHT))
            .flex_shrink_0()
            .bg(color(BG))
            .child(
                div()
                    .id(("strip", group))
                    .debug_selector(move || format!("strip-{group}"))
                    .relative()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&strip)
                    .children(tabs)
                    .child(
                        div()
                            .id(("new-tab", group))
                            .flex_shrink_0()
                            .mt(u(4))
                            .ml(u(TAB_GAP))
                            .mr(u(TAB_GAP))
                            .w(u(PLUS_WIDTH))
                            .h(u(TAB_HEIGHT))
                            .rounded(u(6))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(u(17))
                            .text_color(color(MUTED))
                            .hover(|s| s.bg(color(CONTROL_HOVER).opacity(0.5)).text_color(color(TEXT)))
                            .active(|s| s.bg(color(CONTROL_PRESSED)))
                            .child("+")
                            .tooltip(|window, cx| Tooltip::new("新建终端会话").build(window, cx))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.new_tab("", Some(group), true, window, cx);
                            })),
                    )
                    .when_some(marker, |this, index| {
                        this.child(
                            div()
                                .absolute()
                                .top(u(5))
                                .left(u((TAB_WIDTH * index as f32 - 1.).max(1.)))
                                .w(u(2))
                                .h(u(HEADER_HEIGHT - 10.))
                                .rounded(u(1))
                                .bg(color(ACCENT)),
                        )
                    })
                    .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<TabDrag>, window, cx| {
                        this.strip_drag_move(group, event, window, cx)
                    }))
                    .on_drop(cx.listener(|this, drag: &TabDrag, window, cx| this.finish_drop(drag.tab, window, cx)))
                    .context_menu(Self::tabs_menu(entity.clone(), group)),
            )
            .when(overflow, |this| {
                this.child(
                    h_flex()
                        .flex_shrink_0()
                        .h_full()
                        .child(
                            Button::new(("back", group))
                                .ghost()
                                .small()
                                .w(u(28))
                                .h(u(30))
                                .icon(IconName::ChevronLeft)
                                .on_click(cx.listener(move |this, _, window, _| {
                                    this.scroll_strip(group, -3. * to_px(SCROLL_STEP, window))
                                })),
                        )
                        .child(
                            Button::new(("forward", group))
                                .ghost()
                                .small()
                                .w(u(28))
                                .h(u(30))
                                .icon(IconName::ChevronRight)
                                .on_click(cx.listener(move |this, _, window, _| {
                                    this.scroll_strip(group, 3. * to_px(SCROLL_STEP, window))
                                })),
                        ),
                )
            })
            .child(
                div().flex_shrink_0().px(u(4)).child(
                    Button::new(("tabs-menu", group))
                        .ghost()
                        .small()
                        .w(u(30))
                        .h(u(30))
                        .icon(IconName::Ellipsis)
                        .tooltip("设置与会话")
                        .dropdown_menu(Self::tabs_menu(entity, group)),
                ),
            )
            // Overflow depends on the strip's laid-out width: re-render once
            // if this frame's layout disagrees with what was drawn.
            .child(
                canvas(
                    move |_, window, _| {
                        let viewport = strip.bounds().size.width;
                        let available = viewport + if overflow { nav } else { px(0.) };
                        if viewport > px(0.) && (content > available) != overflow {
                            window.on_next_frame(|window, _| window.refresh());
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_0(),
            )
    }

    fn render_body(&self, group: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = self.layout.selected(group).and_then(|id| self.panes.get(&id)).cloned();
        let hint = match self.drag_hover.filter(|_| cx.has_active_drag()) {
            Some(DropTarget::Pane { group: g }) if g == group => Some(("移入此分栏", false)),
            Some(DropTarget::Split) if group == 0 => Some(("右侧分栏", true)),
            _ => None,
        };
        div()
            .id(("body", group))
            .debug_selector(move || format!("body-{group}"))
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(color(CARD))
            .when_some(pane, |this, pane| this.child(pane))
            .when_some(hint, |this, (label, right_half)| {
                this.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right_0()
                        .when(right_half, |d| d.w_1_2())
                        .when(!right_half, |d| d.left_0())
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(color(SELECTION).opacity(0.85))
                        .border_2()
                        .border_color(color(ACCENT))
                        .text_size(u(13))
                        .text_color(color(TEXT))
                        .child(label),
                )
            })
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<TabDrag>, _, cx| this.body_drag_move(group, event, cx)),
            )
            .on_drop(cx.listener(|this, drag: &TabDrag, window, cx| this.finish_drop(drag.tab, window, cx)))
    }

    fn render_group(&mut self, group: usize, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.active.and_then(|a| self.layout.group_of(a)) == Some(group);
        v_flex()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .rounded(u(6))
            .border_1()
            .border_color(color(if active && self.layout.split() { FOCUS } else { BORDER }))
            .bg(color(CARD))
            .child(self.render_header(group, window, cx))
            .child(div().w_full().h(px(1.)).flex_shrink_0().bg(color(BORDER)))
            .child(self.render_body(group, cx))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.drag_hover = None;
        }
        let split = self.layout.split();
        let ratio = self.split_ratio;
        let left = self.render_group(0, window, cx).into_any_element();
        let right = split.then(|| self.render_group(1, window, cx).into_any_element());
        div()
            .id("workspace")
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .p(u(8))
            .bg(color(BG))
            .text_color(color(TEXT))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.new_tab("", None, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.close_active(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.next_tab(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)))
            .on_action(cx.listener(|this, _: &CancelDrag, window, cx| this.cancel_drag(window, cx)))
            .child(
                h_flex()
                    .id("groups")
                    .size_full()
                    .on_drag_move(cx.listener(|this, event: &DragMoveEvent<SashDrag>, window, cx| {
                        let width = event.bounds.size.width;
                        if width <= px(0.) {
                            return;
                        }
                        let min = (to_px(MIN_GROUP, window) / width).min(0.5);
                        let x = (event.event.position.x - event.bounds.left()) / width;
                        this.split_ratio = x.clamp(min, 1. - min);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .h_full()
                            .min_w_0()
                            .when(split, |d| d.w(relative(ratio)).flex_shrink_0())
                            .when(!split, |d| d.flex_1())
                            .child(left),
                    )
                    .when_some(right, |this, right| {
                        this.child(
                            div()
                                .id("sash")
                                .h_full()
                                .w(u(6))
                                .flex_shrink_0()
                                .cursor_col_resize()
                                .hover(|s| s.bg(color(BORDER)))
                                .on_drag(SashDrag, |_, _, _, cx| cx.new(|_| SashDrag))
                                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                                    if event.click_count() >= 2 {
                                        this.split_ratio = 0.5;
                                        cx.notify();
                                    }
                                })),
                        )
                        .child(div().h_full().flex_1().min_w_0().child(right))
                    }),
            )
    }
}
