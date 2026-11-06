//! One terminal-bound conversation. Sending and tracking each have their own
//! worker, so a new message never waits for an earlier reply and polling
//! never delays a send. Workers and messages never follow UI focus.

use std::borrow::Cow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, Focusable, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, ScrollEvent, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Task, Window, div, prelude::FluentBuilder, px, relative,
};
use gpui_base::SelectableText;
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::{ScrollableElement, Scrollbar, ScrollbarMode},
    v_flex,
};
use serde_json::Value;

use super::layout::TabId;
use super::models::ModelChoices;
use super::theme::{ACCENT, BORDER, ERROR, FAINT, FIELD, FOCUS, MUTED, TEXT, WARNING, color, hover_frame, u};
use super::{Backend, COMPOSER_CONTEXT, SendMessage, Settings, send_hint};
use crate::core::catalog::{TerminalRow, list_models, load_terminals};
use crate::core::error::{BridgeError, Result};
use crate::core::prefs::TabPrefs;
use crate::core::session::{CHINESE_PREFIX, LATEST_CHINESE_PREFIX, Poll, Session, TURN_CONTINUES, TURN_DONE, Turns};
use crate::core::text::{is_blank, strip};
use crate::core::translator::Language;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Reply,
    Attention,
}

#[derive(Clone, Debug)]
pub enum ChatItem {
    Message {
        role: Role,
        text: SharedString,
    },
    /// One round's activity: sent English, raw screen, errors. Open by
    /// default, at a fixed height that scrolls inside.
    Log {
        entries: Vec<String>,
        /// What the log shows: the entries as short selectable runs.
        runs: Vec<SharedString>,
        expanded: bool,
        scroll: ScrollHandle,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Send,
    Track,
}

/// Worker → UI messages. `log` is the index of the round's activity log.
enum Message {
    /// An activity line; a finished translation also appears in the chat.
    Log { log: usize, text: String },
    /// A tracking failure pauses automatic collection; a send failure does not.
    Error { log: Option<usize>, text: String, pause: bool },
    /// A message that was not sent, for an empty composer.
    Unsent(String),
    /// A worker finished one job.
    Finished { lane: Lane, poll: bool },
}

/// Requests for the workspace. Panes never update the workspace directly.
#[derive(Clone, Debug)]
pub enum PaneEvent {
    Activated,
    OpenPicker,
    SavePreferences,
    TargetsLoaded { rows: Vec<TerminalRow>, open_after: bool },
    ModelsLoaded(Vec<Value>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStatus {
    Idle,
    Working,
    Waiting,
    Offline,
    Unread,
    Attention,
}

impl TabStatus {
    pub fn color(self) -> u32 {
        match self {
            TabStatus::Working | TabStatus::Waiting => WARNING,
            TabStatus::Attention => ERROR,
            TabStatus::Unread => ACCENT,
            TabStatus::Offline => MUTED,
            TabStatus::Idle => super::theme::GREEN,
        }
    }
}

/// A worker's channel back to its pane.
struct Out(async_channel::Sender<Message>);

impl Out {
    fn send(&self, message: Message) {
        let _ = self.0.send_blocking(message);
    }

    fn log(&self, log: usize, text: String) {
        self.send(Message::Log { log, text });
    }

    fn error(&self, log: usize, error: &BridgeError, pause: bool) {
        self.send(Message::Error { log: Some(log), text: error.message().to_string(), pause });
    }
}

type Job = Box<dyn FnOnce(&mut Session, &Out) + Send>;

/// Runs its jobs one at a time, in order, on a background thread.
struct Worker {
    jobs: async_channel::Sender<(Job, bool)>,
    _task: Task<()>,
}

impl Worker {
    fn spawn(lane: Lane, mut session: Session, out: async_channel::Sender<Message>, cx: &App) -> Self {
        let (jobs, queue) = async_channel::unbounded::<(Job, bool)>();
        let task = cx.background_spawn(async move {
            let out = Out(out);
            while let Ok((job, poll)) = queue.recv().await {
                if catch_unwind(AssertUnwindSafe(|| job(&mut session, &out))).is_err() {
                    let text = "内部错误：本次操作意外中止，请重试。".to_string();
                    out.send(Message::Error { log: None, text, pause: lane == Lane::Track });
                }
                out.send(Message::Finished { lane, poll });
            }
        });
        Worker { jobs, _task: task }
    }
}

/// Paths and this tab's model, captured when a job is queued.
#[derive(Clone)]
struct Config {
    hc: String,
    codex: String,
    model: String,
    effort: String,
}

impl Config {
    fn apply(&self, session: &mut Session) {
        session.core.set_binary(&self.hc);
        session.translator.configure(&self.codex, &self.model, &self.effort);
    }
}

/// One height for every selector control; the stock single-line input is 2rem.
const CONTROL: f32 = 28.;

/// Lines per selectable run. Selecting costs characters × lines of each run
/// the selection touches, every frame, so long text is split at line breaks.
const RUN_LINES: usize = 12;

/// Splits `text` at line breaks into runs of about `RUN_LINES` lines. No run
/// is blank, so a copy across runs, joined by newlines, keeps every line.
pub fn selectable_runs(text: &str) -> Vec<&str> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let (mut start, mut lines) = (0, 0);
    for (index, _) in text.match_indices('\n') {
        lines += 1;
        if lines >= RUN_LINES && !is_blank(&text[start..index]) {
            runs.push((start, index));
            start = index + 1;
            lines = 0;
        }
    }
    match runs.last_mut() {
        Some(last) if is_blank(&text[start..]) => last.1 = text.len(),
        _ => runs.push((start, text.len())),
    }
    runs.into_iter().map(|(start, end)| &text[start..end]).collect()
}

/// The activity log's fixed height; new entries scroll inside it.
const LOG_HEIGHT: f32 = 200.;

/// Lines a long log entry, such as a terminal snapshot, shows from its end.
/// The terminal itself keeps the full history.
const LOG_ENTRY_LINES: usize = 80;

/// An entry as the log shows it: the first line (its label), then at most the
/// last `LOG_ENTRY_LINES` lines.
pub fn log_display(entry: &str) -> Cow<'_, str> {
    let Some((label, body)) = entry.split_once('\n') else { return entry.into() };
    let lines = body.matches('\n').count() + 1;
    if lines <= LOG_ENTRY_LINES {
        return entry.into();
    }
    let start = body.rmatch_indices('\n').nth(LOG_ENTRY_LINES - 1).map_or(0, |(index, _)| index + 1);
    format!("{label}\n…（已省略前 {} 行）\n{}", lines - LOG_ENTRY_LINES, &body[start..]).into()
}

/// Entries are separated by a blank line, carried by each later entry's first run.
fn log_runs(entry: &str, first: bool) -> Vec<SharedString> {
    let text = log_display(entry);
    let gap = if first { "" } else { "\n" };
    selectable_runs(&text)
        .into_iter()
        .enumerate()
        .map(|(i, run)| if i == 0 { format!("{gap}{run}") } else { run.to_string() }.into())
        .collect()
}

fn selectable(text: &str) -> impl Iterator<Item = SelectableText> + '_ {
    selectable_runs(text).into_iter().enumerate().map(|(i, run)| SelectableText::new(("run", i), run.to_string()))
}

pub struct ConversationPane {
    pub id: TabId,
    /// Turns awaiting replies, shared by both workers.
    turns: Turns,
    sender: Option<Worker>,
    tracker: Option<Worker>,
    /// Send jobs queued or running.
    pub sending: usize,
    /// Tracking jobs queued or running; `polling` when one is automatic.
    pub tracking: usize,
    pub polling: bool,
    loading_targets: bool,
    loading_models: bool,
    pub auto_paused: bool,
    pub unread: bool,
    pub offline: bool,
    pub visible: bool,
    pub disposed: bool,
    pub target: String,
    pub target_name: String,
    pub models: ModelChoices,
    pub items: Vec<ChatItem>,
    /// The newest activity log, for notices outside a round.
    last_log: Option<usize>,
    pub input: Entity<TextareaState>,
    model_input: Entity<InputState>,
    chat_scroll: ScrollHandle,
    _inbox: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PaneEvent> for ConversationPane {}

impl ConversationPane {
    pub fn new(id: TabId, prefs: &TabPrefs, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let models = ModelChoices::new(&prefs.model, &prefs.effort);
        let input = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder(format!("输入中文，{}", send_hint())).auto_grow(3, 10)
        });
        let model_text = models.model.clone();
        let model_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("模型 ID");
            state.set_value(model_text, window, cx);
            state
        });
        let subscriptions = vec![cx.subscribe(&model_input, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                this.set_model(&text, cx);
            }
        })];
        let backend = cx.global::<Backend>().0.clone();
        let sending = backend();
        let mut tracking = backend();
        let turns = sending.turns.clone();
        tracking.turns = turns.clone();
        let (out, inbox) = async_channel::unbounded();
        let sender = Worker::spawn(Lane::Send, sending, out.clone(), cx);
        let tracker = Worker::spawn(Lane::Track, tracking, out, cx);
        let inbox = cx.spawn_in(window, async move |this, cx| {
            while let Ok(message) = inbox.recv().await {
                if this.update_in(cx, |pane, window, cx| pane.receive(message, window, cx)).is_err() {
                    break;
                }
            }
        });
        ConversationPane {
            id,
            turns,
            sender: Some(sender),
            tracker: Some(tracker),
            sending: 0,
            tracking: 0,
            polling: false,
            loading_targets: false,
            loading_models: false,
            auto_paused: false,
            unread: false,
            offline: false,
            visible: false,
            disposed: false,
            target: prefs.target.clone(),
            target_name: if prefs.name.is_empty() { prefs.target.clone() } else { prefs.name.clone() },
            models,
            items: Vec::new(),
            last_log: None,
            input,
            model_input,
            chat_scroll: ScrollHandle::new(),
            _inbox: inbox,
            _subscriptions: subscriptions,
        }
    }

    // ----- State shown by the workspace -----

    pub fn has_pending(&self) -> bool {
        !self.turns.is_empty()
    }

    pub fn pending(&self) -> usize {
        self.turns.len()
    }

    /// A tracking job the user asked for, as opposed to an automatic poll.
    pub fn translating(&self) -> bool {
        self.tracking > usize::from(self.polling)
    }

    /// The oldest turn's send result is unknown; only the user can resolve it.
    pub fn stuck(&self) -> bool {
        self.sending == 0 && self.turns.with_front(|p| !p.sent).unwrap_or(false)
    }

    pub fn needs_attention(&self) -> bool {
        self.has_pending() && (self.auto_paused || self.stuck())
    }

    /// Rebinding or closing would orphan a message in flight or an awaited reply.
    pub fn locked(&self) -> bool {
        self.sending > 0 || self.translating() || self.has_pending()
    }

    pub fn closable(&self) -> bool {
        !self.locked()
    }

    pub fn status(&self) -> TabStatus {
        if self.needs_attention() {
            TabStatus::Attention
        } else if self.sending > 0 || self.translating() {
            TabStatus::Working
        } else if self.has_pending() {
            TabStatus::Waiting
        } else if self.offline {
            TabStatus::Offline
        } else if self.unread {
            TabStatus::Unread
        } else {
            TabStatus::Idle
        }
    }

    pub fn title(&self) -> String {
        [&self.target_name, &self.target]
            .into_iter()
            .find(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| "新会话".into())
    }

    /// Full name, address and status, for tooltips and the session menu.
    pub fn full_title(&self) -> String {
        let mut title = self.title();
        if !self.target.is_empty() && self.target_name != self.target {
            title = format!("{title} · {}", self.target);
        }
        let status = if self.needs_attention() {
            "需处理"
        } else if self.sending > 0 {
            "发送中"
        } else if self.translating() {
            "处理中"
        } else if self.has_pending() {
            "等待"
        } else if self.offline {
            "离线"
        } else if self.unread {
            "新回复"
        } else {
            ""
        };
        if status.is_empty() { title } else { format!("{title} · {status}") }
    }

    pub fn preferences(&self, group: usize) -> TabPrefs {
        TabPrefs {
            target: self.target.clone(),
            name: self.target_name.clone(),
            model: self.models.slug(),
            effort: self.models.effort_value(),
            group,
        }
    }

    pub fn draft(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    pub fn set_draft(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.set_value(text.to_string(), window, cx));
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.focus(window, cx));
    }

    // ----- Workspace → pane -----

    pub fn apply_target(&mut self, row: &TerminalRow, cx: &mut Context<Self>) {
        self.target = row.address.clone();
        self.target_name = row.name.clone();
        self.offline = false;
        cx.notify();
    }

    pub fn set_offline(&mut self, offline: bool, cx: &mut Context<Self>) {
        self.offline = offline;
        cx.notify();
    }

    /// Queued jobs keep the model they were queued with.
    pub fn apply_models(&mut self, records: &[Value], window: &mut Window, cx: &mut Context<Self>) {
        let notice = self.models.apply(records);
        let text = self.models.model.clone();
        self.model_input.update(cx, |state, cx| state.set_value(text, window, cx));
        if let Some(notice) = notice {
            self.log(notice);
        }
        cx.notify();
    }

    pub fn dispose(&mut self) {
        self.disposed = true;
        self.sender = None;
        self.tracker = None;
    }

    fn set_model(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(notice) = self.models.set_model(text) {
            self.log(notice);
        }
        cx.emit(PaneEvent::SavePreferences);
        cx.notify();
    }

    fn pick_model(&mut self, label: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.model_input.update(cx, |state, cx| state.set_value(label.to_string(), window, cx));
        self.set_model(label, cx);
    }

    fn pick_effort(&mut self, effort: &str, cx: &mut Context<Self>) {
        self.models.effort = effort.to_string();
        cx.emit(PaneEvent::SavePreferences);
        cx.notify();
    }

    // ----- Chat and logs -----

    fn add_chat(&mut self, role: Role, text: &str) {
        self.items.push(ChatItem::Message { role, text: text.to_string().into() });
        self.chat_scroll.scroll_to_bottom();
    }

    fn begin_log(&mut self) -> usize {
        self.items.push(ChatItem::Log {
            entries: Vec::new(),
            runs: Vec::new(),
            expanded: true,
            scroll: ScrollHandle::new(),
        });
        let index = self.items.len() - 1;
        self.last_log = Some(index);
        self.chat_scroll.scroll_to_bottom();
        index
    }

    /// Completion is conveyed by the reply itself, not repetitive success notices.
    pub(crate) fn log_to(&mut self, index: usize, text: String) {
        if text == TURN_DONE || text == TURN_CONTINUES {
            return;
        }
        if let Some(ChatItem::Log { entries, runs, expanded, scroll }) = self.items.get_mut(index) {
            // An open log that shows its end keeps following new entries.
            let follow = *expanded && scroll.offset().y <= px(1.) - scroll.max_offset().y;
            runs.extend(log_runs(&text, entries.is_empty()));
            entries.push(text);
            if follow {
                scroll.scroll_to_bottom();
            }
        }
    }

    fn log(&mut self, text: String) {
        let index = match self.last_log {
            Some(index) => index,
            None => self.begin_log(),
        };
        self.log_to(index, text);
    }

    /// Opening a log shows its newest entries.
    pub fn toggle_log(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(ChatItem::Log { expanded, scroll, .. }) = self.items.get_mut(index) {
            *expanded = !*expanded;
            if *expanded {
                scroll.scroll_to_bottom();
                self.chat_scroll.scroll_to_item(index);
            }
            cx.notify();
        }
    }

    /// Appends a finished exchange without a worker (preview and tests).
    pub fn add_demo(&mut self, question: &str, log: &[&str], answer: Option<&str>) {
        self.add_chat(Role::User, question);
        let index = self.begin_log();
        for entry in log {
            self.log_to(index, entry.to_string());
        }
        if let Some(answer) = answer {
            self.add_chat(Role::Reply, answer);
        }
    }

    fn show_error(&mut self, log: Option<usize>, text: String) {
        self.add_chat(Role::Attention, &text);
        self.unread = !self.visible;
        match log {
            Some(log) => self.log_to(log, text),
            None => self.log(text),
        }
    }

    // ----- Jobs -----

    fn config(&self, cx: &App) -> Config {
        let settings = cx.global::<Settings>();
        Config {
            hc: strip(&settings.hc).to_string(),
            codex: strip(&settings.codex).to_string(),
            model: self.models.slug(),
            effort: self.models.effort_value(),
        }
    }

    /// Queues a job on one worker. Preview mode never starts a CLI.
    fn run(&mut self, lane: Lane, poll: bool, cx: &App, job: impl FnOnce(&mut Session, &Out) + Send + 'static) {
        if self.disposed || cx.global::<Settings>().preview {
            return;
        }
        let worker = match lane {
            Lane::Send => self.sender.as_ref(),
            Lane::Track => self.tracker.as_ref(),
        };
        if worker.is_none_or(|w| w.jobs.try_send((Box::new(job), poll)).is_err()) {
            return;
        }
        match lane {
            Lane::Send => self.sending += 1,
            Lane::Track => {
                self.tracking += 1;
                self.polling |= poll;
            }
        }
    }

    fn receive(&mut self, message: Message, window: &mut Window, cx: &mut Context<Self>) {
        if self.disposed {
            return;
        }
        match message {
            Message::Log { log, text } => {
                let chinese = text.strip_prefix(CHINESE_PREFIX).or_else(|| text.strip_prefix(LATEST_CHINESE_PREFIX));
                if let Some(chinese) = chinese {
                    self.auto_paused = false;
                    self.add_chat(Role::Reply, chinese);
                    self.unread = !self.visible;
                }
                self.log_to(log, text);
            }
            Message::Error { log, text, pause } => {
                // With nothing left to collect there is nothing to pause.
                self.auto_paused |= pause && self.has_pending();
                self.show_error(log, text);
            }
            Message::Unsent(chinese) => {
                if is_blank(&self.draft(cx)) {
                    self.set_draft(&chinese, window, cx);
                }
            }
            Message::Finished { lane: Lane::Send, .. } => self.sending = self.sending.saturating_sub(1),
            Message::Finished { lane: Lane::Track, poll } => {
                self.tracking = self.tracking.saturating_sub(1);
                if poll {
                    self.polling = false;
                }
            }
        }
        cx.notify();
    }

    /// Sends at once, even while earlier replies are pending: the composer
    /// clears immediately and gets the text back only if sending fails.
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.disposed {
            return;
        }
        let target = strip(&self.target).to_string();
        if target.is_empty() {
            cx.emit(PaneEvent::OpenPicker);
            return;
        }
        let draft = self.draft(cx);
        if is_blank(&draft) || self.offline || cx.global::<Settings>().preview {
            return;
        }
        let chinese = strip(&draft).to_string();
        let config = self.config(cx);
        self.add_chat(Role::User, &chinese);
        let log = self.begin_log();
        self.set_draft("", window, cx);
        self.run(Lane::Send, false, cx, move |session, out| {
            config.apply(session);
            if let Err(error) = session.start(&target, &chinese, log, &|text| out.log(log, text)) {
                out.error(log, &error, false);
                if !session.turns.has_label(log) {
                    out.send(Message::Unsent(chinese));
                }
            }
        });
        cx.notify();
    }

    /// Retries the oldest pending turn.
    pub fn collect(&mut self, cx: &mut Context<Self>) {
        if self.translating() || !self.has_pending() {
            return;
        }
        self.auto_paused = false;
        let config = self.config(cx);
        self.run(Lane::Track, false, cx, move |session, out| {
            config.apply(session);
            let Some(log) = session.turns.with_front(|p| p.label) else { return };
            if let Err(error) = session.collect(&|text| out.log(log, text)) {
                out.error(log, &error, true);
            }
        });
        cx.notify();
    }

    pub fn current_screen(&mut self, cx: &mut Context<Self>) {
        if self.translating() {
            return;
        }
        let target = strip(&self.target).to_string();
        if target.is_empty() {
            cx.emit(PaneEvent::OpenPicker);
            return;
        }
        // The oldest pending turn on this terminal keeps its own log.
        let log = match self.turns.with_front(|p| (p.target == target).then_some(p.label)).flatten() {
            Some(label) => label,
            None => self.begin_log(),
        };
        self.auto_paused = false;
        let config = self.config(cx);
        self.run(Lane::Track, false, cx, move |session, out| {
            config.apply(session);
            if let Err(error) = session.translate_screen(&target, &|text| out.log(log, text)) {
                out.error(log, &error, true);
            }
        });
        cx.notify();
    }

    /// Translates the composer text itself, for answers copied by hand.
    pub fn manual(&mut self, cx: &mut Context<Self>) {
        if self.translating() {
            return;
        }
        let text = strip(&self.draft(cx)).to_string();
        if text.is_empty() {
            return;
        }
        let log = self.begin_log();
        let config = self.config(cx);
        self.run(Lane::Track, false, cx, move |session, out| {
            config.apply(session);
            match session.translator.translate(&text, Language::SimplifiedChinese) {
                Ok(chinese) => out.log(log, format!("{CHINESE_PREFIX}{chinese}")),
                Err(error) => out.error(log, &error, false),
            }
        });
        cx.notify();
    }

    /// Stops tracking; the task in the original terminal is not affected.
    pub fn abandon(&mut self, cx: &mut Context<Self>) {
        self.turns.clear();
        self.auto_paused = false;
        cx.notify();
    }

    /// Polls the oldest pending turn and collects it once complete.
    pub fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.has_pending() {
            self.auto_paused = false;
        }
        if self.tracking > 0 || !self.has_pending() || self.auto_paused || self.stuck() {
            return;
        }
        let config = self.config(cx);
        self.run(Lane::Track, true, cx, move |session, out| {
            config.apply(session);
            let Some(log) = session.turns.with_front(|p| p.label) else { return };
            let result = session.poll().and_then(|poll| match poll {
                Poll::Ready => session.collect(&|text| out.log(log, text)).map(drop),
                Poll::Attention(_) => Err(BridgeError::new("请先处理原终端中的提示，再重试回译。")),
                Poll::Waiting | Poll::Unknown => Ok(()),
            });
            if let Err(error) = result {
                out.error(log, &error, true);
            }
        });
        cx.notify();
    }

    /// Loads the terminal catalog beside the workers, so it never waits for them.
    pub fn list_targets(&mut self, open_after: bool, cx: &mut Context<Self>) {
        if self.loading_targets || self.disposed || cx.global::<Settings>().preview {
            return;
        }
        self.loading_targets = true;
        let mut session = (cx.global::<Backend>().0)();
        self.config(cx).apply(&mut session);
        let herdr = strip(&cx.global::<Settings>().herdr).to_string();
        let load = cx.background_spawn(async move { load_terminals(session.core.as_ref(), &herdr) });
        cx.spawn(async move |this, cx| {
            let result = load.await;
            let _ = this.update(cx, |pane, cx| {
                pane.loading_targets = false;
                if pane.disposed {
                    return;
                }
                match result {
                    Ok((rows, warning)) => {
                        if !warning.is_empty() {
                            pane.log(warning);
                        }
                        cx.emit(PaneEvent::TargetsLoaded { rows, open_after });
                    }
                    Err(error) => pane.show_error(None, error.message().to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn refresh_models(&mut self, cx: &mut Context<Self>) {
        if self.loading_models || self.disposed || cx.global::<Settings>().preview {
            return;
        }
        self.loading_models = true;
        let codex = strip(&cx.global::<Settings>().codex).to_string();
        let load = cx.background_spawn(async move { list_models(&codex, Duration::from_secs(35)) });
        cx.spawn(async move |this, cx| {
            let result: Result<Vec<Value>> = load.await;
            let _ = this.update(cx, |pane, cx| {
                pane.loading_models = false;
                if pane.disposed {
                    return;
                }
                match result {
                    Ok(records) => cx.emit(PaneEvent::ModelsLoaded(records)),
                    Err(error) => pane.show_error(None, error.message().to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // ----- Rendering -----

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(u(10))
            .child(div().text_size(u(16)).font_weight(FontWeight::SEMIBOLD).child("选择 Herdr 终端"))
            .child(
                div()
                    .text_size(u(13))
                    .text_color(color(MUTED))
                    .child("每个会话固定绑定一个终端。中文译为英文后发送，回复再译回中文。"),
            )
            .child(
                Button::new("choose-target")
                    .primary()
                    .label("选择终端")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::OpenPicker))),
            )
            .into_any_element()
    }

    fn render_item(&self, index: usize, item: &ChatItem, cx: &mut Context<Self>) -> AnyElement {
        match item {
            ChatItem::Message { role, text } => {
                let label = match role {
                    Role::User => Some(("你", MUTED)),
                    Role::Attention => Some(("需要处理", ERROR)),
                    Role::Reply => None,
                };
                v_flex()
                    .w_full()
                    .pt(u(if label.is_some() { 18. } else { 4. }))
                    .pb(u(14))
                    .gap(u(6))
                    .when_some(label, |this, (label, tint)| {
                        this.child(div().text_size(u(12.5)).text_color(color(tint)).child(label))
                    })
                    .child(
                        v_flex()
                            .id(("message", index))
                            .w_full()
                            .line_height(relative(1.6))
                            .text_color(color(TEXT))
                            .children(selectable(text)),
                    )
                    .into_any_element()
            }
            ChatItem::Log { entries, runs, expanded, scroll } => {
                let arrow = if *expanded { "▾" } else { "▸" };
                v_flex()
                    .w_full()
                    .pb(u(6))
                    .gap(u(4))
                    .child(
                        h_flex().child(
                            Button::new(("log-toggle", index))
                                .ghost()
                                .xsmall()
                                .label(format!("{arrow} 活动日志 · {}", entries.len()))
                                .on_click(cx.listener(move |this, _, _, cx| this.toggle_log(index, cx))),
                        ),
                    )
                    .when(*expanded, |this| {
                        let handle = scroll.clone();
                        this.child(
                            div()
                                .relative()
                                .w_full()
                                .h(u(LOG_HEIGHT))
                                .rounded(u(6))
                                .bg(color(FIELD))
                                .border_1()
                                .border_color(color(BORDER))
                                .child(
                                    v_flex()
                                        .id(("log-body", index))
                                        .debug_selector(move || format!("log-{index}"))
                                        .size_full()
                                        .overflow_y_scroll()
                                        .track_scroll(scroll)
                                        .px(u(12))
                                        .py(u(10))
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .text_size(u(12.5))
                                        .text_color(color(MUTED))
                                        .children(
                                            runs.iter()
                                                .enumerate()
                                                .map(|(i, run)| SelectableText::new(("run", i), run.clone())),
                                        ),
                                )
                                // Always shown, unlike the chat's, which fades when idle.
                                .child(
                                    div().absolute().inset_0().child(
                                        Scrollbar::vertical(scroll)
                                            .id(("log-scrollbar", index))
                                            .mode(ScrollbarMode::Always)
                                            .viewport_from_layout(),
                                    ),
                                )
                                // The log scrolls first; at its top or bottom the chat takes over.
                                .on_scroll(move |event: &ScrollEvent, window, cx| {
                                    let delta = event.delta.pixel_delta(window.line_height()).y;
                                    let offset = handle.offset().y;
                                    let up = delta > px(0.) && offset < px(-0.5);
                                    let down = delta < px(0.) && offset > px(0.5) - handle.max_offset().y;
                                    if up || down {
                                        cx.stop_propagation();
                                    }
                                }),
                        )
                    })
                    .into_any_element()
            }
        }
    }

    fn render_selectors(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let model_menu = Button::new("model-pick").ghost().xsmall().icon(IconName::ChevronDown).dropdown_menu({
            let entity = entity.clone();
            move |mut menu, _, cx| {
                let models = &entity.read(cx).models;
                let (current, choices) = (models.model.clone(), models.choices().to_vec());
                for choice in choices {
                    let entity = entity.clone();
                    let checked = choice.label == current;
                    menu = menu.item(PopupMenuItem::new(choice.label.clone()).checked(checked).on_click(
                        move |_, window, cx| {
                            entity.update(cx, |this, cx| this.pick_model(&choice.label, window, cx));
                        },
                    ));
                }
                menu
            }
        });
        let effort_menu = Button::new("effort")
            .outline()
            .small()
            .w(u(84))
            .h(u(CONTROL))
            .label(self.models.effort.clone())
            .dropdown_caret(true)
            .tooltip(format!("推理强度 · {}", self.models.effort))
            .dropdown_menu({
                let entity = entity.clone();
                move |mut menu, _, cx| {
                    let models = &entity.read(cx).models;
                    let current = models.effort.clone();
                    for effort in models.efforts() {
                        let entity = entity.clone();
                        let checked = effort == current;
                        menu =
                            menu.item(PopupMenuItem::new(effort.clone()).checked(checked).on_click(move |_, _, cx| {
                                entity.update(cx, |this, cx| this.pick_effort(&effort, cx));
                            }));
                    }
                    menu
                }
            });
        let actions_menu = Button::new("conversation-menu")
            .ghost()
            .small()
            .w(u(CONTROL))
            .h(u(CONTROL))
            .icon(IconName::Ellipsis)
            .tooltip("会话操作")
            .dropdown_menu({
                let entity = entity.clone();
                move |menu, _, cx| {
                    let pane = entity.read(cx);
                    let (locked, translating, pending) = (pane.locked(), pane.translating(), pane.has_pending());
                    let item =
                        |label: &'static str,
                         enabled: bool,
                         action: fn(&mut ConversationPane, &mut Context<ConversationPane>)| {
                            let entity = entity.clone();
                            PopupMenuItem::new(label)
                                .disabled(!enabled)
                                .on_click(move |_, _, cx| entity.update(cx, action))
                        };
                    // Reloads the catalog first, so new Herdr terminals appear.
                    menu.item(item("选择终端", !locked, |this, cx| this.list_targets(true, cx)))
                        .item(item("回译最新回答", !translating, Self::current_screen))
                        .item(item("仅回译输入", !translating, Self::manual))
                        .separator()
                        .item(item("重试回译", pending && !translating, Self::collect))
                        .item(item("结束跟踪", pending, Self::abandon))
                }
            });
        h_flex()
            .w_full()
            .h(u(CONTROL))
            .gap(u(8))
            .items_center()
            .child(
                div()
                    .id("model")
                    .debug_selector(|| "model-field".into())
                    .w(u(224))
                    .min_w(u(48))
                    .flex_shrink(1.)
                    .tooltip({
                        let tip: SharedString = self.models.tooltip().into();
                        move |window, cx| gpui_component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                    })
                    .child(Input::new(&self.model_input).suffix(model_menu)),
            )
            .child(hover_frame("effort-frame").child(effort_menu))
            .child(
                Button::new("refresh-models")
                    .ghost()
                    .small()
                    .w(u(CONTROL))
                    .h(u(CONTROL))
                    .icon(IconName::RefreshCw)
                    .tooltip("刷新模型")
                    .loading(self.loading_models)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_models(cx))),
            )
            .child(div().flex_1())
            .child(div().flex_shrink_0().child(actions_menu))
    }

    fn render_status(&self) -> String {
        let mut parts = Vec::new();
        if self.sending > 0 {
            parts.push(if self.sending > 1 {
                format!("正在发送 {} 条…", self.sending)
            } else {
                "正在发送…".into()
            });
        }
        if self.translating() {
            parts.push("正在回译…".into());
        }
        match self.pending() {
            0 => {}
            1 => parts.push("等待目标终端回复…".into()),
            n => parts.push(format!("等待目标终端回复（{n} 条）…")),
        }
        if parts.is_empty() && self.offline {
            parts.push("终端不在当前列表中".into());
        }
        parts.join(" · ")
    }

    fn render_composer(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let can_send = !self.offline && !self.target.is_empty();
        let recovery = self.needs_attention() && !self.translating();
        v_flex()
            .w_full()
            .flex_shrink_0()
            .mt(u(8))
            .px(u(10))
            .py(u(8))
            .gap(u(6))
            .rounded(u(8))
            .bg(color(FIELD))
            .border_1()
            .border_color(color(if focused { FOCUS } else { BORDER }))
            .child(self.render_selectors(cx))
            // Only the message field sends on Enter; the model field keeps it.
            .child(
                div()
                    .key_context(COMPOSER_CONTEXT)
                    .on_action(cx.listener(|this, _: &SendMessage, window, cx| this.send(window, cx)))
                    .child(Textarea::new(&self.input).appearance(false).bg(color(FIELD))),
            )
            .child(
                h_flex()
                    .w_full()
                    .h(u(30))
                    .gap(u(8))
                    .when(recovery, |this| {
                        this.child(
                            hover_frame("recover-frame").child(
                                Button::new("recover")
                                    .outline()
                                    .small()
                                    .label("重试回译")
                                    .on_click(cx.listener(|this, _, _, cx| this.collect(cx))),
                            ),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(u(12.5))
                            .text_color(color(FAINT))
                            .child(self.render_status()),
                    )
                    .child(
                        Button::new("send")
                            .primary()
                            .small()
                            .label("发送")
                            .disabled(!can_send)
                            .tooltip(send_hint())
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx))),
                    ),
            )
    }
}

impl Render for ConversationPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if self.target.is_empty() && self.items.is_empty() {
            self.render_empty(cx)
        } else {
            let items: Vec<AnyElement> =
                self.items.iter().enumerate().map(|(i, item)| self.render_item(i, item, cx)).collect();
            v_flex()
                .id("chat")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.chat_scroll)
                .px(u(18))
                .py(u(6))
                .children(items)
                .into_any_element()
        };
        v_flex()
            .id(("conversation", self.id))
            .size_full()
            .px(u(12))
            .pt(u(4))
            .pb(u(10))
            .capture_any_mouse_down(cx.listener(|_, _, _, cx| cx.emit(PaneEvent::Activated)))
            .child(div().relative().flex_1().min_h_0().w_full().child(body).vertical_scrollbar(&self.chat_scroll))
            .child(self.render_composer(window, cx))
            .text_color(color(TEXT))
            .bg(color(super::theme::CARD))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_log_entries_show_their_label_and_last_lines() {
        let body: String = (1..=500).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let entry = format!("终端原始内容：\n{body}");
        let shown = log_display(&entry);
        let lines: Vec<&str> = shown.split('\n').collect();
        assert_eq!(lines[..2], ["终端原始内容：", "…（已省略前 420 行）"]);
        assert_eq!((lines.len(), lines[2], *lines.last().unwrap()), (LOG_ENTRY_LINES + 2, "line 421", "line 500"));
        for short in ["已发送到 t3，等待回复…", "英文译文：\nHello\nWorld"] {
            assert!(matches!(log_display(short), Cow::Borrowed(text) if text == short));
        }
        let runs = log_runs("英文译文：\nHello", false);
        assert_eq!(runs.concat(), "\n英文译文：\nHello");
    }

    #[test]
    fn runs_split_at_line_breaks_and_rejoin_exactly() {
        let long: String = (0..40).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        for text in [long.as_str(), "", "one line", "a\n\n\n", &format!("{long}\n\n\n\n\n\n\n\n\n\n\n\n\n\n")] {
            let runs = selectable_runs(text);
            assert_eq!(runs.join("\n"), text);
            assert!(runs.iter().all(|r| r.matches('\n').count() < 2 * RUN_LINES));
            assert!(runs.len() == 1 || runs.iter().all(|r| !is_blank(r)));
        }
        assert_eq!(selectable_runs(&long).len(), 4);
    }
}
