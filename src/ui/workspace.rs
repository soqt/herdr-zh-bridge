//! Herdr's spaces and the conversations with their terminals. The space list
//! on the left mirrors Herdr's window, machine by machine, and follows it as
//! spaces open and close; the current space's conversations fill the right
//! in tab groups split side by side or stacked. A conversation always lives
//! in the space its terminal is in. Selection, status and splitting never
//! change the window size.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, Bounds, ClickEvent, Context, DragMoveEvent, Entity, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, canvas, div, point,
    prelude::FluentBuilder, px, relative,
};
use gpui_component::{
    Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit_assets::IconName as Lucide;
use serde_json::Value;

use super::conversation::{ConversationPane, PaneEvent, TabStatus};
use super::layout::{self, DropTarget, EMPTY, Edge, Layout, Node, PaneId, Space, SpaceId, TabId};
use super::theme::{
    ACCENT, ACCENT_TEXT, BG, BORDER, CARD, CONTROL_HOVER, CONTROL_PRESSED, FAINT, FOCUS, HOVER, MUTED, SELECTED,
    SELECTION, TEXT, WARNING, color, to_px, u,
};
use super::{
    Backend, CancelDrag, CloseTab, NewTab, NextTab, OWN_TITLEBAR, OpenSettings, SelectSpace, Settings, ToggleSidebar,
    WORKSPACE_CONTEXT, picker, settings,
};
use crate::core::catalog::{Catalog, HerdrSpace, TerminalRow, load_terminals};
use crate::core::prefs::{self, Axis, OTHER_SPACE, Preferences, SpacePrefs, TabPrefs};
use crate::core::text::strip;

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
/// The smallest pane a split or a divider drag may leave.
const MIN_PANE_WIDTH: f32 = 300.;
const MIN_PANE_HEIGHT: f32 = 260.;
const SASH: f32 = 6.;
/// A drop this close to a pane's edge, as a fraction of its body, splits it.
const EDGE_ZONE: f32 = 0.3;
pub const SIDEBAR_WIDTH: f32 = 236.;
/// The collapsed space list: initials only.
const RAIL_WIDTH: f32 = 48.;
const GROUP_ROW: f32 = 30.;
const SPACE_ROW: f32 = 32.;
/// The sidebar's top row, in pixels: on macOS it holds the window buttons,
/// which do not scale with the font size.
pub const TITLE_HEIGHT: f32 = 56.;
/// Where macOS draws the window buttons, centred in the title row.
pub const WINDOW_BUTTONS_AT: (f32, f32) = (16., 21.);
/// The room the window buttons take at the start of the title row, in pixels.
const WINDOW_BUTTONS_WIDTH: f32 = 84.;
/// Above the panes, in pixels, so the top tabs line up with the window buttons.
const TOP_GAP: f32 = 6.;
/// Ticks (2 s each) between reloads of Herdr's layout.
const CATALOG_EVERY: u32 = 2;

/// A tab being dragged.
#[derive(Clone)]
pub struct TabDrag {
    pub tab: TabId,
    title: SharedString,
}

struct DragPreview(SharedString);

impl Render for DragPreview {
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

/// A divider between children `index` and `index + 1` of the split at `path`.
#[derive(Clone)]
struct SashDrag {
    path: Vec<usize>,
    index: usize,
}

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

struct SpaceInfo {
    id: SpaceId,
    name: SharedString,
    count: usize,
    /// The most urgent conversation's; `None` without conversations.
    status: Option<TabStatus>,
    selected: bool,
    live: bool,
}

/// What to open once the catalog being read arrives.
#[derive(Clone, Copy, Debug)]
enum Pick {
    /// Choose the terminal of this conversation.
    Tab(TabId),
    /// Start a conversation with one of this space's terminals.
    Space(SpaceId),
}

type PaneBounds = HashMap<(SpaceId, PaneId), Bounds<Pixels>>;

#[derive(Default)]
struct Strip {
    scroll: ScrollHandle,
    overflow: bool,
}

/// The status a space shows: its most urgent tab's.
fn rank(status: TabStatus) -> u8 {
    match status {
        TabStatus::Attention => 5,
        TabStatus::Working => 4,
        TabStatus::Waiting => 3,
        TabStatus::Unread => 2,
        TabStatus::Offline => 1,
        TabStatus::Idle => 0,
    }
}

fn status_label(status: TabStatus) -> &'static str {
    match status {
        TabStatus::Attention => "需处理",
        TabStatus::Working => "处理中",
        TabStatus::Waiting => "等待回复",
        TabStatus::Unread => "新回复",
        TabStatus::Offline => "有离线终端",
        TabStatus::Idle => "",
    }
}

/// A space-list group's icon and heading.
fn group_heading(group: &str) -> (Icon, String) {
    match group.strip_prefix("machine:") {
        Some(machine) => (Icon::new(Lucide::Server), machine.to_string()),
        None if group == "local" => (Icon::new(Lucide::Monitor), "本机".into()),
        None => (Icon::new(IconName::SquareTerminal), "其他".into()),
    }
}

/// What the collapsed rail shows for a space.
fn initial(name: &str) -> String {
    name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

pub struct Workspace {
    panes: HashMap<TabId, Entity<ConversationPane>>,
    subscriptions: HashMap<TabId, [Subscription; 2]>,
    /// In list order: Herdr's window order, grouped by machine.
    spaces: Vec<Space>,
    current: SpaceId,
    active: Option<TabId>,
    next_id: TabId,
    next_space: SpaceId,
    targets: Vec<TerminalRow>,
    /// Herdr's window has answered at least once, so `targets` say which
    /// space each terminal is in.
    layout_known: bool,
    catalog_loading: bool,
    /// Why the space list may be out of date.
    catalog_note: Option<String>,
    pick_after: Option<Pick>,
    ticks: u32,
    models: Vec<Value>,
    prefs_path: Option<PathBuf>,
    /// What was last written, so an unchanged state is not written again.
    saved: RefCell<Option<Preferences>>,
    ready: bool,
    focus: FocusHandle,
    strips: HashMap<(SpaceId, PaneId), Strip>,
    /// Each pane's last drawn bounds, for split room checks.
    pane_bounds: Rc<RefCell<PaneBounds>>,
    space_list: ScrollHandle,
    drag_hover: Option<DropTarget>,
    /// The dragged tab and the pointer x inside a strip viewport.
    strip_pointer: Option<(PaneId, Pixels, TabId)>,
    edge_scroll: Option<(PaneId, f32)>,
    edge_task: Option<Task<()>>,
    /// The space list shows initials only.
    collapsed: bool,
    /// Machine groups whose spaces are hidden.
    folded: BTreeSet<String>,
    /// A divider moved during the current drag; saved once it ends.
    resized: bool,
    /// Pressed on an empty part of the top row; the next move drags the window.
    moving_window: bool,
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
                if this.update_in(cx, |this, window, cx| this.tick(window, cx)).is_err() {
                    break;
                }
            }
        });
        let mut this = Workspace {
            panes: HashMap::new(),
            subscriptions: HashMap::new(),
            spaces: Vec::new(),
            current: 0,
            active: None,
            next_id: 1,
            next_space: 1,
            targets: Vec::new(),
            layout_known: false,
            catalog_loading: false,
            catalog_note: None,
            pick_after: None,
            ticks: 0,
            models: Vec::new(),
            prefs_path,
            saved: RefCell::new(None),
            ready: false,
            focus: cx.focus_handle(),
            strips: HashMap::new(),
            pane_bounds: Rc::default(),
            space_list: ScrollHandle::new(),
            drag_hover: None,
            strip_pointer: None,
            edge_scroll: None,
            edge_task: None,
            collapsed: prefs.sidebar_collapsed,
            folded: prefs.folded.iter().cloned().collect(),
            resized: false,
            moving_window: false,
            _tick: tick,
        };
        // Herdr's spaces as last seen, until its window answers.
        for saved in &prefs.spaces {
            let tree = saved
                .layout
                .clone()
                .and_then(|tree| tree.filter_map(&mut |tab: TabPrefs| Some(this.create_pane(&tab, window, cx))));
            let layout = tree.map(Layout::restore).unwrap_or_default();
            let active = layout.order().get(saved.active).copied();
            let id = this.alloc_space();
            this.spaces.push(Space {
                id,
                key: saved.key.clone(),
                name: saved.name.clone(),
                machine: saved.machine.clone(),
                live: true,
                layout,
                active,
            });
        }
        this.current = this.spaces.iter().find(|s| s.key == prefs.space).or(this.spaces.first()).map_or(0, |s| s.id);
        if !target.is_empty() {
            this.new_tab(target, None, false, window, cx);
        }
        this.ready = true;
        this.select_space(this.current, window, cx);
        this.refresh_catalog(window, cx);
        this
    }

    // ----- Spaces -----

    fn alloc_space(&mut self) -> SpaceId {
        self.next_space += 1;
        self.next_space - 1
    }

    fn space_index(&self, id: SpaceId) -> Option<usize> {
        self.spaces.iter().position(|s| s.id == id)
    }

    fn space_ref(&self, id: SpaceId) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    fn space_mut(&mut self, id: SpaceId) -> Option<&mut Space> {
        self.spaces.iter_mut().find(|s| s.id == id)
    }

    fn current_space(&self) -> Option<&Space> {
        self.space_ref(self.current)
    }

    /// The current space's arrangement. Without any space yet, conversations
    /// go to the one for terminals outside Herdr's window.
    fn layout_mut(&mut self) -> &mut Layout {
        if self.space_index(self.current).is_none() {
            self.current = self.ensure_other();
        }
        let index = self.space_index(self.current).unwrap();
        &mut self.spaces[index].layout
    }

    /// The current space's arrangement.
    pub fn layout(&self) -> &Layout {
        self.current_space().map_or(&EMPTY, |s| &s.layout)
    }

    pub fn spaces(&self) -> &[Space] {
        &self.spaces
    }

    pub fn current(&self) -> SpaceId {
        self.current
    }

    pub fn space_of(&self, tab: TabId) -> Option<SpaceId> {
        self.spaces.iter().find(|s| s.layout.contains(tab)).map(|s| s.id)
    }

    /// Every tab, spaces in list order.
    fn all_tabs(&self) -> Vec<TabId> {
        self.spaces.iter().flat_map(|s| s.layout.order()).collect()
    }

    /// The space for conversations whose terminal is outside Herdr's window.
    fn ensure_other(&mut self) -> SpaceId {
        if let Some(space) = self.spaces.iter().find(|s| s.key.is_empty()) {
            return space.id;
        }
        let id = self.alloc_space();
        self.spaces.push(Space {
            id,
            key: String::new(),
            name: OTHER_SPACE.into(),
            machine: None,
            live: false,
            layout: Layout::default(),
            active: None,
        });
        id
    }

    /// Shows a space and focuses the tab it last had focused.
    pub fn select_space(&mut self, id: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.space_ref(id) else { return };
        let tab = space.active.filter(|t| space.layout.contains(*t)).or_else(|| space.layout.order().first().copied());
        if let Some(tab) = tab {
            self.focus_tab(tab, true, window, cx);
            return;
        }
        // An empty space lists its terminals; keys still reach the workspace.
        self.current = id;
        self.active = None;
        self.focus.focus(window, cx);
        self.sync_visibility(cx);
        self.save_preferences(cx);
        cx.notify();
    }

    fn select_space_at(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.spaces.get(index).map(|s| s.id) {
            self.select_space(id, window, cx);
        }
    }

    /// Closes every conversation of a space unless one is translating or
    /// waiting for a reply. The space stays while Herdr has it.
    pub fn close_sessions(&mut self, id: SpaceId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(tabs) = self.space_ref(id).map(|s| s.layout.order()) else { return false };
        if tabs.is_empty() || tabs.iter().any(|t| self.panes[t].read(cx).locked()) {
            return false;
        }
        for tab in &tabs {
            if let Some(pane) = self.panes.remove(tab) {
                pane.update(cx, |pane, _| pane.dispose());
            }
            self.subscriptions.remove(tab);
        }
        if let Some(space) = self.space_mut(id) {
            space.layout = Layout::default();
            space.active = None;
        }
        if self.active.is_some_and(|a| tabs.contains(&a)) {
            self.active = None;
        }
        self.prune(window, cx);
        if self.current == id {
            self.select_space(id, window, cx);
        }
        self.sync_visibility(cx);
        self.save_preferences(cx);
        cx.notify();
        true
    }

    /// Drops spaces Herdr no longer has once their last conversation is gone.
    fn prune(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.space_index(self.current);
        self.spaces.retain(|s| s.live || !s.layout.is_empty());
        if self.space_ref(self.current).is_none() {
            self.active = None;
            let next = index.and_then(|i| self.spaces.get(i.min(self.spaces.len().saturating_sub(1)))).map(|s| s.id);
            match next {
                Some(next) => self.select_space(next, window, cx),
                None => self.current = 0,
            }
        }
        self.forget_stale_panes();
    }

    pub fn toggle_sidebar(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.collapsed = !self.collapsed;
        self.save_preferences(cx);
        cx.notify();
    }

    pub fn sidebar_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Folds or unfolds a machine's spaces in the list.
    pub fn toggle_group(&mut self, group: &str, cx: &mut Context<Self>) {
        if !self.folded.remove(group) {
            self.folded.insert(group.to_string());
        }
        self.save_preferences(cx);
        cx.notify();
    }

    /// The + of a space: a new conversation with one of its terminals.
    pub fn add_session(&mut self, id: SpaceId, window: &mut Window, cx: &mut Context<Self>) {
        if cx.has_active_drag() || self.space_ref(id).is_none() {
            return;
        }
        self.select_space(id, window, cx);
        self.pick(Pick::Space(id), window, cx);
    }

    /// The space a terminal is in, once Herdr's layout is known.
    fn home_of(&self, address: &str) -> Option<SpaceId> {
        if !self.layout_known || address.is_empty() {
            return None;
        }
        let row = self.targets.iter().find(|r| r.address == address)?;
        let key = row.space().unwrap_or_default();
        self.spaces.iter().find(|s| s.key == key).map(|s| s.id)
    }

    /// Moves a conversation into the space its terminal is in. The caller
    /// updates focus and visibility.
    fn place(&mut self, tab: TabId, cx: &App) -> bool {
        let Some(home) = self.panes.get(&tab).and_then(|p| self.home_of(&p.read(cx).target)) else { return false };
        let Some(from) = self.space_of(tab).filter(|from| *from != home) else { return false };
        let source = self.space_mut(from).unwrap();
        let focus_pane = source.layout.remove(tab);
        if source.active == Some(tab) {
            source.active = focus_pane.and_then(|p| source.layout.selected(p));
        }
        let space = self.space_mut(home).unwrap();
        let pane = space.active.and_then(|a| space.layout.pane_of(a));
        space.layout.add(tab, pane, None);
        space.active.get_or_insert(tab);
        true
    }

    /// A space's terminals, as its + offers them; every terminal while
    /// Herdr's layout is unknown or the space has none.
    pub fn space_rows(&self, space: Option<SpaceId>) -> Vec<TerminalRow> {
        let key = space.and_then(|s| self.space_ref(s)).map(|s| s.key.as_str());
        let rows: Vec<TerminalRow> = match key {
            Some(key) if self.layout_known => {
                self.targets.iter().filter(|r| r.space().unwrap_or_default() == key).cloned().collect()
            }
            _ => Vec::new(),
        };
        if rows.is_empty() { self.targets.clone() } else { rows }
    }

    /// Follows Herdr's window: its spaces, their names and order. Spaces it
    /// closed stay while they have conversations.
    fn mirror(&mut self, herdr: &[HerdrSpace]) {
        for space in self.spaces.iter_mut().filter(|s| !s.key.is_empty()) {
            space.live = false;
        }
        for found in herdr {
            match self.spaces.iter_mut().find(|s| s.key == found.id) {
                Some(space) => {
                    space.name = found.name.clone();
                    space.machine = found.machine.clone();
                    space.live = true;
                }
                None => {
                    let id = self.alloc_space();
                    self.spaces.push(Space {
                        id,
                        key: found.id.clone(),
                        name: found.name.clone(),
                        machine: found.machine.clone(),
                        live: true,
                        layout: Layout::default(),
                        active: None,
                    });
                }
            }
        }
        let outside = self.targets.iter().any(|r| r.space().is_none());
        if outside {
            self.ensure_other();
        }
        if let Some(other) = self.spaces.iter_mut().find(|s| s.key.is_empty()) {
            other.live = outside;
        }
        // Machines in the order Herdr first lists them; closed spaces after
        // their machine's open ones; terminals outside Herdr last.
        let groups: Vec<String> = herdr.iter().map(|h| layout::group_of(&h.id, h.machine.as_deref())).collect();
        self.spaces.sort_by_key(|space| {
            let group = match space.key.is_empty() {
                true => usize::MAX,
                false => groups.iter().position(|g| *g == space.group()).unwrap_or(groups.len()),
            };
            (group, herdr.iter().position(|h| h.id == space.key).unwrap_or(herdr.len()))
        });
    }

    /// Takes a fresh catalog: terminal names and status always, and Herdr's
    /// spaces when its window answered. Conversations follow their terminals.
    pub fn apply_catalog(&mut self, catalog: Catalog, window: &mut Window, cx: &mut Context<Self>) {
        self.catalog_note = Some(catalog.warning).filter(|w| !w.is_empty());
        self.apply_targets(catalog.rows, cx);
        if let Some(herdr) = catalog.spaces {
            self.layout_known = true;
            self.mirror(&herdr);
            for tab in self.all_tabs() {
                self.place(tab, cx);
            }
            // The focused conversation is followed to its space.
            if let Some(active) = self.active
                && self.space_of(active) != Some(self.current)
            {
                self.focus_tab(active, false, window, cx);
            }
            self.prune(window, cx);
            if self.space_ref(self.current).is_none()
                && let Some(first) = self.spaces.first().map(|s| s.id)
            {
                self.select_space(first, window, cx);
            }
        }
        self.sync_visibility(cx);
        self.save_preferences(cx);
        cx.notify();
    }

    /// Reloads terminals and Herdr's layout beside everything else.
    pub fn refresh_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = cx.global::<Settings>();
        if self.catalog_loading || settings.preview {
            return;
        }
        self.catalog_loading = true;
        let herdr = strip(&settings.herdr).to_string();
        let hc = strip(&settings.hc).to_string();
        let mut session = (cx.global::<Backend>().0)();
        session.core.set_binary(&hc);
        let load = cx.background_spawn(async move { load_terminals(session.core.as_ref(), &herdr) });
        cx.spawn_in(window, async move |this, cx| {
            let result = load.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.catalog_loading = false;
                match result {
                    Ok(catalog) => this.apply_catalog(catalog, window, cx),
                    Err(error) => {
                        this.catalog_note = Some(error.message().to_string());
                        cx.notify();
                    }
                }
                if let Some(pick) = this.pick_after.take() {
                    this.open_pick(pick, window, cx);
                }
            });
        })
        .detach();
    }

    /// Reloads the catalog, so new Herdr terminals appear, then picks.
    fn pick(&mut self, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<Settings>().preview {
            self.open_pick(pick, window, cx);
            return;
        }
        self.pick_after = Some(pick);
        self.refresh_catalog(window, cx);
    }

    fn open_pick(&mut self, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        match pick {
            Pick::Tab(tab) => self.open_picker(tab, window, cx),
            Pick::Space(space) => {
                let Some(name) = self.space_ref(space).map(|s| s.name.clone()) else { return };
                let rows = self.space_rows(Some(space));
                if rows.is_empty() {
                    return;
                }
                let workspace = cx.entity().downgrade();
                let title = format!("在「{name}」新建会话").into();
                picker::open(rows, String::new(), title, window, cx, move |row, window, cx| {
                    let _ = workspace.update(cx, |this, cx| this.open_terminal(space, row, window, cx));
                });
            }
        }
    }

    /// A conversation with a terminal, in `space`; the one it already has
    /// comes to the front instead.
    pub fn open_terminal(
        &mut self,
        space: SpaceId,
        row: TerminalRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        if let Some(existing) = self.find_target(&row.address, cx) {
            self.focus_tab(existing, true, window, cx);
            return Some(existing);
        }
        let space = if self.space_ref(space).is_some() { space } else { self.ensure_other() };
        let prefs = TabPrefs { target: row.address.clone(), ..self.inherited(cx) };
        let id = self.create_pane(&prefs, window, cx);
        self.panes[&id].update(cx, |pane, cx| pane.apply_target(&row, cx));
        let space = self.space_mut(space)?;
        let pane = space.active.and_then(|a| space.layout.pane_of(a));
        space.layout.add(id, pane, None);
        self.place(id, cx);
        self.focus_tab(id, true, window, cx);
        self.prune(window, cx);
        Some(id)
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
                let p = pane.read(cx).preferences();
                TabPrefs { model: p.model, effort: p.effort, ..TabPrefs::default() }
            })
            .unwrap_or_default()
    }

    /// One terminal belongs to one tab across every space.
    pub fn find_target(&self, address: &str, cx: &App) -> Option<TabId> {
        if address.is_empty() {
            return None;
        }
        self.all_tabs().into_iter().find(|id| self.panes[id].read(cx).target == address)
    }

    pub fn pane(&self, id: TabId) -> Option<&Entity<ConversationPane>> {
        self.panes.get(&id)
    }

    pub fn active(&self) -> Option<TabId> {
        self.active
    }

    pub fn targets(&self) -> &[TerminalRow] {
        &self.targets
    }

    /// Opens a tab in `pane` of the current space, or beside the active tab;
    /// a terminal in another Herdr space opens there.
    pub fn new_tab(
        &mut self,
        target: &str,
        pane: Option<PaneId>,
        choose: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TabId {
        if let Some(existing) = self.find_target(target, cx) {
            self.focus_tab(existing, true, window, cx);
            return existing;
        }
        let pane = pane.or_else(|| self.active.and_then(|a| self.layout().pane_of(a)));
        let prefs = TabPrefs { target: target.into(), ..self.inherited(cx) };
        let id = self.create_pane(&prefs, window, cx);
        if let Some(row) = self.targets.iter().find(|r| !target.is_empty() && r.address == target).cloned() {
            self.panes[&id].update(cx, |pane, cx| pane.apply_target(&row, cx));
        }
        self.layout_mut().add(id, pane, None);
        self.place(id, cx);
        self.focus_tab(id, true, window, cx);
        if choose {
            self.pick(Pick::Tab(id), window, cx);
        }
        id
    }

    /// Opens a new conversation in a new pane at `edge` of `pane`.
    pub fn new_tab_beside(
        &mut self,
        pane: PaneId,
        edge: Edge,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TabId> {
        if self.layout().group(pane).is_none() || !self.room_to_split(pane, edge.axis(), window) {
            return None;
        }
        let prefs = self.inherited(cx);
        let id = self.create_pane(&prefs, window, cx);
        let layout = self.layout_mut();
        layout.add(id, Some(pane), None);
        layout.drop_tab(id, DropTarget::Edge { pane, edge });
        self.focus_tab(id, true, window, cx);
        self.pick(Pick::Tab(id), window, cx);
        Some(id)
    }

    /// Moves a tab into a new pane at `edge` of its own pane.
    pub fn split_tab(&mut self, tab: TabId, edge: Edge, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(pane) = self.layout().pane_of(tab) else { return false };
        if !self.room_to_split(pane, edge.axis(), window)
            || !self.layout_mut().drop_tab(tab, DropTarget::Edge { pane, edge })
        {
            return false;
        }
        self.focus_tab(tab, true, window, cx);
        true
    }

    /// Whether `pane` is big enough for two panes along `axis`. A pane not
    /// drawn yet is assumed to be.
    fn room_to_split(&self, pane: PaneId, axis: Axis, window: &Window) -> bool {
        let Some(bounds) = self.pane_bounds.borrow().get(&(self.current, pane)).copied() else { return true };
        match axis {
            Axis::Row => bounds.size.width >= to_px(2. * MIN_PANE_WIDTH + SASH, window),
            Axis::Column => bounds.size.height >= to_px(2. * MIN_PANE_HEIGHT + SASH, window),
        }
    }

    pub fn focus_tab(&mut self, id: TabId, focus_input: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.panes.get(&id).cloned() else { return };
        let Some(space) = self.space_of(id) else { return };
        self.current = space;
        if let Some(space) = self.space_mut(space) {
            space.layout.select(id);
            space.active = Some(id);
        }
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

    /// Only the selected tab of each pane in the current space is visible.
    fn sync_visibility(&mut self, cx: &mut Context<Self>) {
        let layout = self.layout();
        let shown: Vec<TabId> = layout.panes().into_iter().filter_map(|p| layout.selected(p)).collect();
        for (id, pane) in &self.panes {
            let visible = shown.contains(id);
            let state = pane.read(cx);
            // A reply on screen is read.
            if state.visible != visible || (visible && state.unread) {
                pane.update(cx, |pane, _| {
                    pane.visible = visible;
                    pane.unread &= !visible;
                });
            }
        }
    }

    fn strip(&mut self, pane: PaneId) -> &mut Strip {
        self.strips.entry((self.current, pane)).or_default()
    }

    fn scroll_selection_into_view(&mut self) {
        let layout = self.layout();
        let items: Vec<(PaneId, usize)> = layout
            .panes()
            .into_iter()
            .filter_map(|pane| {
                let tabs = layout.tabs(pane);
                let index = tabs.iter().position(|t| Some(*t) == layout.selected(pane))?;
                // The last tab also reveals the + beside it.
                Some((pane, if index + 1 == tabs.len() { tabs.len() } else { index }))
            })
            .collect();
        for (pane, item) in items {
            self.strip(pane).scroll.scroll_to_item(item);
        }
    }

    /// Drops strip and bounds state for panes that no longer exist.
    fn forget_stale_panes(&mut self) {
        let live: Vec<(SpaceId, PaneId)> =
            self.spaces.iter().flat_map(|s| s.layout.panes().into_iter().map(move |p| (s.id, p))).collect();
        self.strips.retain(|key, _| live.contains(key));
        self.pane_bounds.borrow_mut().retain(|key, _| live.contains(key));
    }

    /// Closes a tab unless it is translating or waiting for a reply. A space
    /// left empty lists its terminals.
    pub fn close_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(pane) = self.panes.get(&id).cloned() else { return false };
        if pane.read(cx).locked() {
            return false;
        }
        let Some(space_id) = self.space_of(id) else { return false };
        let was_active = self.active == Some(id) || self.active.is_none();
        if self.active == Some(id) {
            self.active = None;
        }
        let space = self.space_mut(space_id).unwrap();
        let focus_pane = space.layout.remove(id);
        if space.active == Some(id) {
            space.active = focus_pane.and_then(|p| space.layout.selected(p));
        }
        pane.update(cx, |pane, _| pane.dispose());
        self.panes.remove(&id);
        self.subscriptions.remove(&id);
        // A space Herdr closed goes with its last conversation.
        self.prune(window, cx);
        if was_active && space_id == self.current {
            self.select_space(space_id, window, cx);
        }
        self.sync_visibility(cx);
        self.save_preferences(cx);
        cx.notify();
        true
    }

    pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.active.is_some_and(|id| self.close_tab(id, window, cx))
    }

    fn next_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let order = self.layout().order();
        if let Some(index) = self.active.and_then(|a| order.iter().position(|t| *t == a)) {
            self.focus_tab(order[(index + 1) % order.len()], true, window, cx);
        }
    }

    /// Binds a chosen terminal. One terminal belongs to one tab; choosing a
    /// different terminal for a bound tab opens a new tab beside it instead
    /// of mixing two histories. The tab moves to the terminal's space.
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
            let group = self.layout().pane_of(id);
            self.new_tab(&row.address, group, false, window, cx)
        } else {
            id
        };
        self.panes[&tab].update(cx, |pane, cx| pane.apply_target(&row, cx));
        if self.place(tab, cx) && self.active == Some(tab) {
            self.focus_tab(tab, true, window, cx);
        }
        self.prune(window, cx);
        self.sync_visibility(cx);
        self.save_preferences(cx);
        cx.notify();
    }

    pub fn open_picker(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.panes.get(&id).cloned() else { return };
        if pane.read(cx).disposed || pane.read(cx).locked() {
            return;
        }
        let rows = self.space_rows(self.space_of(id));
        if rows.is_empty() {
            return;
        }
        let workspace = cx.entity().downgrade();
        let selected = pane.read(cx).target.clone();
        picker::open(rows, selected, "选择目标终端".into(), window, cx, move |row, window, cx| {
            let _ = workspace.update(cx, |this, cx| this.bind_target(id, row, window, cx));
        });
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        settings::open(cx.entity().downgrade(), window, cx);
    }

    /// Terminal names and status only; Herdr's spaces come with [`Self::apply_catalog`].
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
        cx.notify();
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
            PaneEvent::OpenPicker => self.pick(Pick::Tab(id), window, cx),
            PaneEvent::SavePreferences => self.save_preferences(cx),
            PaneEvent::ModelsLoaded(records) => self.apply_models(records.clone(), window, cx),
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for pane in self.panes.values().cloned().collect::<Vec<_>>() {
            pane.update(cx, |pane, cx| pane.tick(cx));
        }
        self.ticks += 1;
        if self.ticks.is_multiple_of(CATALOG_EVERY) {
            self.refresh_catalog(window, cx);
        }
    }

    pub fn save_preferences(&self, cx: &App) {
        let Some(path) = self.prefs_path.as_ref().filter(|_| self.ready) else { return };
        let settings = cx.global::<Settings>();
        let spaces =
            self.spaces
                .iter()
                .map(|space| {
                    let order = space.layout.order();
                    SpacePrefs {
                        key: space.key.clone(),
                        name: space.name.clone(),
                        machine: space.machine.clone(),
                        layout: space.layout.to_tree().and_then(|tree| {
                            tree.filter_map(&mut |id| Some(self.panes.get(&id)?.read(cx).preferences()))
                        }),
                        active: space.active.and_then(|a| order.iter().position(|t| *t == a)).unwrap_or(0),
                    }
                })
                .collect();
        let prefs = Preferences {
            hc: settings.hc.clone(),
            codex: settings.codex.clone(),
            herdr: settings.herdr.clone(),
            spaces,
            space: self.current_space().map(|s| s.key.clone()).unwrap_or_default(),
            sidebar_collapsed: self.collapsed,
            folded: self.folded.iter().cloned().collect(),
            zoom: settings.zoom,
        };
        if self.saved.borrow().as_ref() == Some(&prefs) {
            return;
        }
        match prefs::save(path, &prefs) {
            Ok(()) => *self.saved.borrow_mut() = Some(prefs),
            Err(error) => eprintln!("无法保存设置：{error}"),
        }
    }

    // ----- Dragging -----

    fn strip_drag_move(
        &mut self,
        pane: PaneId,
        event: &DragMoveEvent<TabDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tab = event.drag(cx).tab;
        let (position, bounds) = (event.event.position, event.bounds);
        if bounds.contains(&position) {
            let local = position.x - bounds.left();
            self.strip_pointer = Some((pane, local, tab));
            self.update_strip_hover(pane, window);
            let margin = to_px(EDGE, window);
            self.edge_scroll = if local < margin {
                Some((pane, -1.))
            } else if local > bounds.size.width - margin {
                Some((pane, 1.))
            } else {
                None
            };
            if self.edge_scroll.is_some() {
                self.ensure_edge_scroll(window, cx);
            }
        } else if self.strip_pointer.is_some_and(|(p, _, _)| p == pane) {
            self.strip_pointer = None;
            self.edge_scroll = None;
            if matches!(self.drag_hover, Some(DropTarget::Strip { pane: p, .. }) if p == pane) {
                self.drag_hover = None;
            }
        }
        cx.notify();
    }

    fn update_strip_hover(&mut self, pane: PaneId, window: &Window) {
        let Some((_, local, tab)) = self.strip_pointer.filter(|(p, _, _)| *p == pane) else { return };
        let width = to_px(TAB_WIDTH, window);
        let x = local - self.strips.get(&(self.current, pane)).map_or(px(0.), |s| s.scroll.offset().x);
        let count = self.layout().tabs(pane).len();
        let index = ((x + width / 2.) / width).floor().clamp(0., count as f32) as usize;
        let target = DropTarget::Strip { pane, index };
        self.drag_hover = self.layout().accepts(tab, target).then_some(target);
    }

    /// Near an edge a drop splits the pane there; in the middle it merges.
    fn body_target(
        &self,
        pane: PaneId,
        tab: TabId,
        position: gpui::Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: &Window,
    ) -> Option<DropTarget> {
        let (width, height) = (bounds.size.width, bounds.size.height);
        if !bounds.contains(&position) || width <= px(0.) || height <= px(0.) {
            return None;
        }
        let fx = (position.x - bounds.left()) / width;
        let fy = (position.y - bounds.top()) / height;
        let edges = [(Edge::Left, fx), (Edge::Right, 1. - fx), (Edge::Top, fy), (Edge::Bottom, 1. - fy)];
        let (edge, distance) = edges.into_iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        let layout = self.layout();
        if distance < EDGE_ZONE {
            let target = DropTarget::Edge { pane, edge };
            if layout.accepts(tab, target) && self.room_to_split(pane, edge.axis(), window) {
                return Some(target);
            }
        }
        let target = DropTarget::Pane { pane };
        layout.accepts(tab, target).then_some(target)
    }

    fn body_drag_move(
        &mut self,
        pane: PaneId,
        event: &DragMoveEvent<TabDrag>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let tab = event.drag(cx).tab;
        match self.body_target(pane, tab, event.event.position, event.bounds, window) {
            Some(target) => self.drag_hover = Some(target),
            None => {
                if let Some(DropTarget::Pane { pane: p } | DropTarget::Edge { pane: p, .. }) = self.drag_hover
                    && p == pane
                {
                    self.drag_hover = None;
                }
            }
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
        let Some((pane, direction)) = self.edge_scroll else {
            self.edge_task = None;
            return false;
        };
        self.scroll_strip(pane, direction * to_px(SCROLL_STEP, window));
        self.update_strip_hover(pane, window);
        cx.notify();
        true
    }

    fn scroll_strip(&mut self, pane: PaneId, by: Pixels) {
        let strip = &self.strip(pane).scroll;
        let offset = strip.offset();
        let x = (offset.x - by).min(px(0.)).max(-strip.max_offset().x);
        strip.set_offset(point(x, offset.y));
    }

    fn finish_drop(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.edge_scroll = None;
        self.strip_pointer = None;
        if let Some(target) = self.drag_hover.take()
            && self.layout_mut().drop_tab(tab, target)
        {
            self.forget_stale_panes();
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

    // ----- Menus -----

    fn tabs_menu(
        entity: Entity<Workspace>,
        pane: PaneId,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        move |menu, window, cx| {
            let workspace = entity.read(cx);
            let active = workspace.active;
            let tabs: Vec<(TabId, String)> = workspace
                .layout()
                .order()
                .into_iter()
                .map(|id| (id, workspace.panes[&id].read(cx).full_title()))
                .collect();
            let (right, down) =
                (workspace.room_to_split(pane, Axis::Row, window), workspace.room_to_split(pane, Axis::Column, window));
            let on = |f: fn(&mut Workspace, &mut Window, &mut Context<Workspace>)| {
                let entity = entity.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| f(this, window, cx))
                }
            };
            let beside = |edge: Edge| {
                let entity = entity.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| {
                        this.new_tab_beside(pane, edge, window, cx);
                    })
                }
            };
            let new_tab = {
                let entity = entity.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| {
                        this.new_tab("", Some(pane), true, window, cx);
                    })
                }
            };
            let mut menu = menu
                .item(PopupMenuItem::new("新建会话").on_click(new_tab))
                .item(PopupMenuItem::new("向右分栏新建").disabled(!right).on_click(beside(Edge::Right)))
                .item(PopupMenuItem::new("向下分栏新建").disabled(!down).on_click(beside(Edge::Bottom)))
                .separator()
                .item(PopupMenuItem::new("设置").on_click(on(|this, window, cx| this.open_settings(window, cx))))
                .separator();
            for (id, title) in tabs {
                let entity = entity.clone();
                menu =
                    menu.item(PopupMenuItem::new(title).checked(active == Some(id)).on_click(move |_, window, cx| {
                        entity.update(cx, |this, cx| this.focus_tab(id, true, window, cx));
                    }));
            }
            menu
        }
    }

    fn tab_menu(
        entity: Entity<Workspace>,
        tab: TabId,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        move |menu, window, cx| {
            let workspace = entity.read(cx);
            let layout = workspace.layout();
            let can_split = |edge: Edge| {
                layout.pane_of(tab).is_some_and(|pane| {
                    layout.accepts(tab, DropTarget::Edge { pane, edge })
                        && workspace.room_to_split(pane, edge.axis(), window)
                })
            };
            let (right, down) = (can_split(Edge::Right), can_split(Edge::Bottom));
            let closable = workspace.panes.get(&tab).is_some_and(|p| p.read(cx).closable());
            let split = |edge: Edge| {
                let entity = entity.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| {
                        this.split_tab(tab, edge, window, cx);
                    })
                }
            };
            let close = {
                let entity = entity.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| {
                        this.close_tab(tab, window, cx);
                    })
                }
            };
            menu.item(PopupMenuItem::new("向右拆分").disabled(!right).on_click(split(Edge::Right)))
                .item(PopupMenuItem::new("向下拆分").disabled(!down).on_click(split(Edge::Bottom)))
                .separator()
                .item(PopupMenuItem::new("关闭会话").disabled(!closable).on_click(close))
        }
    }

    fn space_menu(
        entity: Entity<Workspace>,
        space: SpaceId,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        move |menu, _, cx| {
            let workspace = entity.read(cx);
            let Some(found) = workspace.space_ref(space) else { return menu };
            let tabs = found.layout.order();
            let closable = !tabs.is_empty() && tabs.iter().all(|t| workspace.panes[t].read(cx).closable());
            let live = found.live;
            let on = |f: fn(&mut Workspace, SpaceId, &mut Window, &mut Context<Workspace>)| {
                let entity = entity.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    entity.update(cx, |this, cx| f(this, space, window, cx))
                }
            };
            menu.item(PopupMenuItem::new("新建会话").disabled(!live).on_click(on(|this, space, window, cx| {
                this.add_session(space, window, cx);
            })))
            .separator()
            .item(PopupMenuItem::new("关闭全部会话").disabled(!closable).on_click(on(
                |this, space, window, cx| {
                    this.close_sessions(space, window, cx);
                },
            )))
        }
    }

    // ----- Rendering: space list -----

    /// Lets an empty part of the top of the window move it, as a title bar
    /// would: drag to move, double-click to zoom.
    fn window_drag<E: InteractiveElement>(element: E, cx: &mut Context<Self>) -> E {
        if !OWN_TITLEBAR {
            return element;
        }
        element
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, _| {
                    this.moving_window = event.click_count == 1;
                    if event.click_count == 2 {
                        window.titlebar_double_click();
                    }
                }),
            )
            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.moving_window = false))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, _| {
                if std::mem::take(&mut this.moving_window) && event.pressed_button == Some(MouseButton::Left) {
                    window.start_window_move();
                }
            }))
    }

    fn icon_button(id: impl Into<SharedString>, tip: &'static str) -> gpui::Stateful<gpui::Div> {
        let id: SharedString = id.into();
        let selector = id.to_string();
        div()
            .id(id)
            .debug_selector(move || selector.clone())
            .flex_shrink_0()
            .size(u(28))
            .rounded(u(6))
            .flex()
            .items_center()
            .justify_center()
            .text_color(color(MUTED))
            .cursor_pointer()
            .hover(|s| s.bg(color(CONTROL_HOVER).opacity(0.5)).text_color(color(TEXT)))
            .active(|s| s.bg(color(CONTROL_PRESSED)))
            // Not a window drag.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
    }

    fn toggle_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tip = match (self.collapsed, cfg!(target_os = "macos")) {
            (true, true) => "展开空间栏 · ⌘B",
            (true, false) => "展开空间栏 · Ctrl+B",
            (false, true) => "收起空间栏 · ⌘B",
            (false, false) => "收起空间栏 · Ctrl+B",
        };
        let icon = if self.collapsed { IconName::PanelLeftOpen } else { IconName::PanelLeftClose };
        Self::icon_button("toggle-sidebar", tip)
            .child(Icon::new(icon).small())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_sidebar(window, cx)))
    }

    /// The top row: the window buttons on macOS, then the app's name.
    fn render_title(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let buttons = OWN_TITLEBAR && !window.is_fullscreen();
        let row = h_flex()
            .id("title-row")
            .debug_selector(|| "title-row".into())
            .flex_shrink_0()
            .w_full()
            .when(OWN_TITLEBAR, |r| r.h(px(TITLE_HEIGHT)))
            .when(!OWN_TITLEBAR, |r| r.h(u(48)))
            .pr(u(10))
            .gap(u(8))
            .when(buttons, |r| r.pl(px(WINDOW_BUTTONS_WIDTH)))
            .when(!buttons, |r| r.pl(u(14)));
        let row = Self::window_drag(row, cx);
        if self.collapsed {
            return row;
        }
        let mark = div()
            .flex_shrink_0()
            .size(u(22))
            .rounded(u(6))
            .flex()
            .items_center()
            .justify_center()
            .bg(color(ACCENT))
            .text_size(u(13))
            .font_weight(FontWeight::BOLD)
            .text_color(color(ACCENT_TEXT))
            .child("中");
        row.child(mark)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(u(15))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color(TEXT))
                    .child("中文桥"),
            )
            .child(self.toggle_button(cx))
    }

    fn render_heading(&self, group: &str, cx: &mut Context<Self>) -> AnyElement {
        if self.collapsed {
            return div().flex_shrink_0().mx(u(10)).my(u(5)).h(px(1.)).bg(color(BORDER)).into_any_element();
        }
        let (icon, label) = group_heading(group);
        let folded = self.folded.contains(group);
        let key = group.to_string();
        h_flex()
            .id(SharedString::from(format!("group-{group}")))
            .debug_selector({
                let selector = format!("group-{group}");
                move || selector.clone()
            })
            .flex_shrink_0()
            .w_full()
            .h(u(GROUP_ROW))
            .mt(u(2))
            .pl(u(10))
            .pr(u(10))
            .gap(u(8))
            .rounded(u(6))
            .text_color(color(MUTED))
            .cursor_pointer()
            .hover(|s| s.bg(color(CONTROL_HOVER).opacity(0.5)).text_color(color(TEXT)))
            .active(|s| s.bg(color(CONTROL_PRESSED)))
            .child(icon.small())
            .child(div().flex_1().min_w_0().truncate().text_size(u(13)).child(label))
            .child(Icon::new(if folded { IconName::ChevronRight } else { IconName::ChevronDown }).xsmall())
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(&key, cx)))
            .into_any_element()
    }

    fn render_space_row(&self, info: SpaceInfo, cx: &mut Context<Self>) -> AnyElement {
        let SpaceInfo { id, name, count, status, selected, live } = info;
        let text = color(if !live {
            FAINT
        } else if selected {
            TEXT
        } else {
            MUTED
        });
        let dot = |size: f32| {
            div().flex_shrink_0().size(u(size)).rounded_full().when_some(status, |d, s| d.bg(color(s.color())))
        };
        let row = if self.collapsed {
            // The rail shows each space's initial with its status dot.
            div()
                .id(("space", id))
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .h(u(36))
                .text_size(u(14))
                .text_color(text)
                .child(initial(&name))
                .child(dot(6.).absolute().top(u(5)).right(u(5)))
        } else {
            h_flex().id(("space", id)).h(u(SPACE_ROW)).pl(u(32)).pr(u(5)).gap(u(4))
        };
        let label = status.map(status_label).unwrap_or_default();
        let mut tooltip = format!("{name} · {count} 个会话");
        if !label.is_empty() {
            tooltip += &format!(" · {label}");
        }
        if !live {
            tooltip += " · Herdr 中已关闭，会话关闭后移除";
        }
        let tooltip: SharedString = tooltip.into();
        let add = div()
            .id(("add", id))
            .debug_selector(move || format!("add-{id}"))
            .flex_shrink_0()
            .size(u(22))
            .rounded(u(5))
            .flex()
            .items_center()
            .justify_center()
            .text_color(color(if selected { MUTED } else { FAINT }))
            .cursor_pointer()
            .hover(|s| s.bg(color(CONTROL_HOVER)).text_color(color(TEXT)))
            .active(|s| s.bg(color(CONTROL_PRESSED)))
            .child(Icon::new(IconName::Plus).small())
            .tooltip(|window, cx| Tooltip::new("在此空间新建会话").build(window, cx))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.add_session(id, window, cx);
            }));
        row.debug_selector(move || format!("space-{id}"))
            .flex_shrink_0()
            .w_full()
            .rounded(u(6))
            .border_1()
            .border_color(color(if selected { BORDER } else { BG }))
            .bg(color(if selected { SELECTED } else { BG }))
            .when(!selected, |this| {
                this.hover(|s| s.bg(color(CONTROL_HOVER).opacity(0.5)).border_color(color(CONTROL_HOVER).opacity(0.5)))
                    .active(|s| s.bg(color(CONTROL_PRESSED)).border_color(color(CONTROL_PRESSED)))
            })
            .cursor_pointer()
            .when(!self.collapsed, |this| {
                this.child(div().flex_1().min_w_0().truncate().text_size(u(13)).text_color(text).child(name.clone()))
                    .when(live, |this| this.child(add))
                    .child(div().flex_shrink_0().w(u(14)).flex().justify_center().child(dot(7.)))
            })
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, window, cx| this.select_space(id, window, cx)))
            .context_menu(Self::space_menu(cx.entity(), id))
            .into_any_element()
    }

    fn render_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut group: Option<String> = None;
        for space in &self.spaces {
            let own = space.group();
            if group.as_ref() != Some(&own) {
                // The rail separates groups but cannot fold them.
                if !self.collapsed || group.is_some() {
                    rows.push(self.render_heading(&own, cx));
                }
                group = Some(own.clone());
            }
            if !self.collapsed && self.folded.contains(&own) {
                continue;
            }
            let order = space.layout.order();
            let info = SpaceInfo {
                id: space.id,
                name: space.name.clone().into(),
                count: order.len(),
                status: order.iter().map(|t| self.panes[t].read(cx).status()).max_by_key(|s| rank(*s)),
                selected: space.id == self.current,
                live: space.live,
            };
            rows.push(self.render_space_row(info, cx));
        }
        let collapsed = self.collapsed;
        let empty_note = match (self.spaces.is_empty(), self.layout_known) {
            (false, _) => None,
            (true, false) => Some("正在读取 Herdr 空间…"),
            (true, true) => Some("Herdr 中还没有空间"),
        };
        let settings_tip =
            if cfg!(target_os = "macos") { "路径与字号 · ⌘," } else { "路径与字号 · Ctrl+," };
        let settings = h_flex()
            .id("sidebar-settings")
            .flex_shrink_0()
            .h(u(32))
            .gap(u(8))
            .rounded(u(6))
            .text_size(u(13))
            .text_color(color(MUTED))
            .cursor_pointer()
            .hover(|s| s.bg(color(CONTROL_HOVER).opacity(0.5)).text_color(color(TEXT)))
            .active(|s| s.bg(color(CONTROL_PRESSED)))
            .child(Icon::new(IconName::Settings).small())
            .when(collapsed, |this| this.w(u(28)).justify_center())
            .when(!collapsed, |this| this.flex_1().px(u(10)).child("设置"))
            .tooltip(move |window, cx| Tooltip::new(settings_tip).build(window, cx))
            .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx)));
        let note = self.catalog_note.clone().filter(|_| !collapsed).map(|note| {
            let tip: SharedString = note.into();
            h_flex()
                .id("catalog-note")
                .flex_shrink_0()
                .h(u(26))
                .px(u(10))
                .gap(u(6))
                .text_size(u(12))
                .text_color(color(WARNING))
                .child(Icon::new(IconName::TriangleAlert).xsmall())
                .child(div().flex_1().min_w_0().truncate().child("Herdr 布局暂不可用"))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        });
        // The rail must fit the window buttons.
        let width = if collapsed {
            let rail = to_px(RAIL_WIDTH, window);
            if OWN_TITLEBAR { rail.max(px(WINDOW_BUTTONS_WIDTH)) } else { rail }
        } else {
            to_px(SIDEBAR_WIDTH, window)
        };
        v_flex()
            .id("sidebar")
            .debug_selector(|| "sidebar".into())
            .w(width)
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(color(BORDER))
            .bg(color(BG))
            .child(self.render_title(window, cx))
            .when(!collapsed, |this| {
                this.child(
                    div()
                        .flex_shrink_0()
                        .h(u(28))
                        .px(u(14))
                        .flex()
                        .items_center()
                        .text_size(u(12.5))
                        .text_color(color(FAINT))
                        .child("空间"),
                )
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        v_flex()
                            .id("space-list")
                            .debug_selector(|| "space-list".into())
                            .size_full()
                            .px(u(6))
                            .gap(u(1))
                            .overflow_y_scroll()
                            .track_scroll(&self.space_list)
                            .children(rows)
                            .when_some(empty_note.filter(|_| !collapsed), |this, note| {
                                this.child(
                                    div().px(u(10)).py(u(6)).text_size(u(12.5)).text_color(color(FAINT)).child(note),
                                )
                            }),
                    )
                    .vertical_scrollbar(&self.space_list),
            )
            .children(note)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .px(u(6))
                    .pb(u(8))
                    .pt(u(4))
                    .gap(u(2))
                    .when(collapsed, |this| this.flex_col().items_center())
                    .child(settings)
                    .when(collapsed, |this| this.child(self.toggle_button(cx))),
            )
    }

    /// A space without conversations: its terminals, one click each.
    fn render_empty_space(&self, cx: &mut Context<Self>) -> AnyElement {
        let space = self.current_space();
        let id = space.map(|s| s.id);
        let rows: Vec<TerminalRow> = match space {
            Some(space) if self.layout_known => {
                self.targets.iter().filter(|r| r.space().unwrap_or_default() == space.key).cloned().collect()
            }
            Some(_) => Vec::new(),
            // No space at all: whatever terminals there are.
            None => self.targets.clone(),
        };
        let (title, hint) = match space {
            None if !self.layout_known => ("正在读取 Herdr 空间…".to_string(), ""),
            None => ("Herdr 中还没有空间".to_string(), "在 Herdr 里新建空间后，这里会自动显示。"),
            Some(space) if !space.live => (space.name.clone(), "Herdr 中已关闭这个空间。"),
            Some(space) if rows.is_empty() && self.layout_known => (space.name.clone(), "这个空间里还没有可用的终端。"),
            Some(space) if rows.is_empty() => (space.name.clone(), "正在读取这个空间的终端…"),
            Some(space) => (space.name.clone(), "选择一个终端，开始中文会话。"),
        };
        let buttons: Vec<AnyElement> = rows
            .into_iter()
            .map(|row| {
                let address = row.address.clone();
                let detail = [row.app.as_str(), row.cwd.as_str()]
                    .into_iter()
                    .filter(|s| !s.is_empty() && *s != "未上报")
                    .collect::<Vec<_>>()
                    .join(" · ");
                h_flex()
                    .id(SharedString::from(format!("open-{address}")))
                    .debug_selector({
                        let selector = format!("open-{address}");
                        move || selector.clone()
                    })
                    .w_full()
                    .h(u(44))
                    .px(u(14))
                    .gap(u(10))
                    .rounded(u(8))
                    .border_1()
                    .border_color(color(BORDER))
                    .bg(color(BG))
                    .cursor_pointer()
                    .hover(|s| s.bg(color(CONTROL_HOVER).opacity(0.5)).border_color(color(CONTROL_HOVER)))
                    .active(|s| s.bg(color(CONTROL_PRESSED)))
                    .child(Icon::new(IconName::SquareTerminal).small().text_color(color(MUTED)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().text_size(u(13)).text_color(color(TEXT)).child(row.name.clone()))
                            .when(!detail.is_empty(), |this| {
                                this.child(div().truncate().text_size(u(11.5)).text_color(color(FAINT)).child(detail))
                            }),
                    )
                    .child(div().flex_shrink_0().text_size(u(12)).text_color(color(FAINT)).child(address))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let space = id.unwrap_or_else(|| this.ensure_other());
                        this.open_terminal(space, row.clone(), window, cx);
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .id("empty-space")
            .debug_selector(|| "empty-space".into())
            .size_full()
            .rounded(u(6))
            .border_1()
            .border_color(color(BORDER))
            .bg(color(CARD))
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .w(u(420))
                    .max_w(relative(0.9))
                    .gap(u(8))
                    .child(div().text_size(u(16)).font_weight(FontWeight::SEMIBOLD).child(title))
                    .when(!hint.is_empty(), |this| {
                        this.child(div().text_size(u(13)).text_color(color(MUTED)).child(hint))
                    })
                    .when(!buttons.is_empty(), |this| this.child(v_flex().mt(u(8)).gap(u(6)).children(buttons))),
            )
            .into_any_element()
    }

    // ----- Rendering: panes -----

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
                    cx.new(|_| DragPreview(drag.title.clone()))
                })
                .context_menu(Self::tab_menu(cx.entity(), id)),
        )
    }

    /// `top`: the pane touches the top of the window, so its strip's empty
    /// part moves the window.
    fn render_header(
        &mut self,
        pane: PaneId,
        top: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let layout = self.layout();
        let selected = layout.selected(pane);
        let infos: Vec<TabInfo> = layout
            .tabs(pane)
            .iter()
            .map(|id| {
                let conversation = self.panes[id].read(cx);
                TabInfo {
                    id: *id,
                    title: conversation.title().into(),
                    tooltip: conversation.full_title().into(),
                    status: conversation.status(),
                    closable: conversation.closable(),
                    selected: selected == Some(*id),
                }
            })
            .collect();
        let count = infos.len();
        let content = to_px(TAB_WIDTH * count as f32 + 2. * TAB_GAP + PLUS_WIDTH + TAB_GAP, window);
        let nav = to_px(NAV_WIDTH, window);
        let state = self.strip(pane);
        let strip = state.scroll.clone();
        let viewport = strip.bounds().size.width;
        let available = viewport + if state.overflow { nav } else { px(0.) };
        let overflow = viewport > px(0.) && content > available;
        state.overflow = overflow;
        let marker = match self.drag_hover {
            Some(DropTarget::Strip { pane: p, index }) if p == pane && cx.has_active_drag() => Some(index),
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
                    .id(("strip", pane))
                    .debug_selector(move || format!("strip-{pane}"))
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
                            .id(("new-tab", pane))
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
                                this.new_tab("", Some(pane), true, window, cx);
                            })),
                    )
                    .when(top && OWN_TITLEBAR, |this| {
                        this.child(Self::window_drag(div().id(("drag", pane)).flex_1().min_w_0().h_full(), cx))
                    })
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
                        this.strip_drag_move(pane, event, window, cx)
                    }))
                    .on_drop(cx.listener(|this, drag: &TabDrag, window, cx| this.finish_drop(drag.tab, window, cx)))
                    .context_menu(Self::tabs_menu(entity.clone(), pane)),
            )
            .when(overflow, |this| {
                this.child(
                    h_flex()
                        .flex_shrink_0()
                        .h_full()
                        .child(
                            Button::new(("back", pane))
                                .ghost()
                                .small()
                                .w(u(28))
                                .h(u(30))
                                .icon(IconName::ChevronLeft)
                                .on_click(cx.listener(move |this, _, window, _| {
                                    this.scroll_strip(pane, -3. * to_px(SCROLL_STEP, window))
                                })),
                        )
                        .child(
                            Button::new(("forward", pane))
                                .ghost()
                                .small()
                                .w(u(28))
                                .h(u(30))
                                .icon(IconName::ChevronRight)
                                .on_click(cx.listener(move |this, _, window, _| {
                                    this.scroll_strip(pane, 3. * to_px(SCROLL_STEP, window))
                                })),
                        ),
                )
            })
            .child(
                div().flex_shrink_0().px(u(4)).child(
                    Button::new(("tabs-menu", pane))
                        .ghost()
                        .small()
                        .w(u(30))
                        .h(u(30))
                        .icon(IconName::Ellipsis)
                        .tooltip("会话、分栏与设置")
                        .dropdown_menu(Self::tabs_menu(entity, pane)),
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

    fn render_body(&self, pane: PaneId, cx: &mut Context<Self>) -> impl IntoElement {
        let conversation = self.layout().selected(pane).and_then(|id| self.panes.get(&id)).cloned();
        let hint = match self.drag_hover.filter(|_| cx.has_active_drag()) {
            Some(DropTarget::Pane { pane: p }) if p == pane => Some(("移入此分栏", None)),
            Some(DropTarget::Edge { pane: p, edge }) if p == pane => {
                let label = match edge {
                    Edge::Left => "左侧分栏",
                    Edge::Right => "右侧分栏",
                    Edge::Top => "上方分栏",
                    Edge::Bottom => "下方分栏",
                };
                Some((label, Some(edge)))
            }
            _ => None,
        };
        div()
            .id(("body", pane))
            .debug_selector(move || format!("body-{pane}"))
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(color(CARD))
            .when_some(conversation, |this, conversation| this.child(conversation))
            .when_some(hint, |this, (label, edge)| {
                let overlay = div()
                    .absolute()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(color(SELECTION).opacity(0.85))
                    .border_2()
                    .border_color(color(ACCENT))
                    .text_size(u(13))
                    .text_color(color(TEXT))
                    .child(label);
                this.child(match edge {
                    None => overlay.inset_0(),
                    Some(Edge::Left) => overlay.top_0().bottom_0().left_0().w_1_2(),
                    Some(Edge::Right) => overlay.top_0().bottom_0().right_0().w_1_2(),
                    Some(Edge::Top) => overlay.left_0().right_0().top_0().h_1_2(),
                    Some(Edge::Bottom) => overlay.left_0().right_0().bottom_0().h_1_2(),
                })
            })
            .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<TabDrag>, window, cx| {
                this.body_drag_move(pane, event, window, cx)
            }))
            .on_drop(cx.listener(|this, drag: &TabDrag, window, cx| this.finish_drop(drag.tab, window, cx)))
    }

    fn render_group(
        &mut self,
        pane: PaneId,
        top: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let layout = self.layout();
        let focused = layout.panes().len() > 1 && self.active.and_then(|a| layout.pane_of(a)) == Some(pane);
        let bounds = self.pane_bounds.clone();
        let key = (self.current, pane);
        v_flex()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .rounded(u(6))
            .border_1()
            .border_color(color(if focused { FOCUS } else { BORDER }))
            .bg(color(CARD))
            .child(self.render_header(pane, top, window, cx))
            .child(div().w_full().h(px(1.)).flex_shrink_0().bg(color(BORDER)))
            .child(self.render_body(pane, cx))
            .child(
                canvas(
                    move |b, _, _| {
                        bounds.borrow_mut().insert(key, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
    }

    fn render_sash(&self, path: &[usize], index: usize, axis: Axis, cx: &mut Context<Self>) -> impl IntoElement {
        let name: String = path.iter().map(|i| format!("-{i}")).collect::<String>() + &format!("-{index}");
        let selector = format!("sash{name}");
        let reset = path.to_vec();
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .flex_shrink_0()
            .rounded(u(2))
            .when(axis == Axis::Row, |d| d.w(u(SASH)).h_full().cursor_col_resize())
            .when(axis == Axis::Column, |d| d.h(u(SASH)).w_full().cursor_row_resize())
            .hover(|s| s.bg(color(BORDER)))
            .active(|s| s.bg(color(FOCUS)))
            .on_drag(SashDrag { path: path.to_vec(), index }, |drag, _, _, cx| cx.new(|_| drag.clone()))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.click_count() >= 2 {
                    this.layout_mut().equalize(&reset);
                    this.save_preferences(cx);
                    cx.notify();
                }
            }))
    }

    fn render_node(
        &mut self,
        node: &Node,
        path: Vec<usize>,
        top: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (axis, children) = match node {
            Node::Pane(group) => return self.render_group(group.id, top, window, cx).into_any_element(),
            Node::Split { axis, children } => (*axis, children),
        };
        let mut container = div()
            .id(SharedString::from(format!("split{path:?}")))
            .flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .when(axis == Axis::Row, |d| d.flex_row())
            .when(axis == Axis::Column, |d| d.flex_col());
        for (i, (weight, child)) in children.iter().enumerate() {
            if i > 0 {
                container = container.child(self.render_sash(&path, i - 1, axis, cx));
            }
            let mut child_path = path.clone();
            child_path.push(i);
            // Stacked panes below the first are not at the top.
            let element = self.render_node(child, child_path, top && (axis == Axis::Row || i == 0), window, cx);
            container = container.child(
                div()
                    .flex_basis(relative(0.))
                    .flex_grow(*weight)
                    .flex_shrink_1()
                    .min_w_0()
                    .min_h_0()
                    .when(axis == Axis::Row, |d| d.h_full())
                    .when(axis == Axis::Column, |d| d.w_full())
                    .child(element),
            );
        }
        container
            .on_drag_move(cx.listener(move |this, event: &DragMoveEvent<SashDrag>, window, cx| {
                let drag = event.drag(cx);
                if drag.path != path {
                    return;
                }
                let index = drag.index;
                let bounds = event.bounds;
                let (extent, offset, min) = match axis {
                    Axis::Row => (bounds.size.width, event.event.position.x - bounds.left(), MIN_PANE_WIDTH),
                    Axis::Column => (bounds.size.height, event.event.position.y - bounds.top(), MIN_PANE_HEIGHT),
                };
                if extent <= px(0.) {
                    return;
                }
                this.layout_mut().resize(&path, index, offset / extent, to_px(min, window) / extent);
                this.resized = true;
                cx.notify();
            }))
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.drag_hover = None;
            // Wherever the divider was released, its shares are saved once.
            if std::mem::take(&mut self.resized) {
                self.save_preferences(cx);
            }
        }
        let sidebar = self.render_sidebar(window, cx).into_any_element();
        let root = self.layout().root().cloned();
        let body = match root {
            Some(root) => self.render_node(&root, Vec::new(), true, window, cx),
            None => self.render_empty_space(cx),
        };
        h_flex()
            .id("workspace")
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .bg(color(BG))
            .text_color(color(TEXT))
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.new_tab("", None, true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                this.close_active(window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.next_tab(window, cx)))
            // A drag belongs to the space it started in, so the view stays put until it ends.
            .on_action(cx.listener(|this, action: &SelectSpace, window, cx| {
                if !cx.has_active_drag() {
                    this.select_space_at(action.0, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, window, cx| this.toggle_sidebar(window, cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)))
            .on_action(cx.listener(|this, _: &CancelDrag, window, cx| this.cancel_drag(window, cx)))
            .child(sidebar)
            .child(
                div()
                    .id("space-body")
                    .relative()
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .pt(px(TOP_GAP))
                    .px(u(8))
                    .pb(u(8))
                    .child(body)
                    .when(OWN_TITLEBAR, |this| {
                        this.child(Self::window_drag(
                            div().id("top-gap").absolute().top_0().left_0().right_0().h(px(TOP_GAP)),
                            cx,
                        ))
                    }),
            )
    }
}
