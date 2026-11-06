//! One terminal-bound round trip: translate, send once, wait, extract, translate back.
//! A conversation runs one session for sending and another for tracking; they
//! share a queue of turns, so a message never waits for an earlier reply.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::Value;

use super::error::{BridgeError, Result};
use super::herdr::{Identity, Terminals, identity, own_terminal, status, turn};
use super::text::is_blank;
use super::translator::{Language, Translate};

/// Log lines the UI recognises as a finished translation.
pub const CHINESE_PREFIX: &str = "中文译文：\n";
pub const LATEST_CHINESE_PREFIX: &str = "最新回答的中文译文：\n";
pub const TURN_DONE: &str = "本轮已完成，可以发送下一条。";
pub const TURN_CONTINUES: &str = "已翻译当前输出；目标尚未报告本轮完成，继续跟踪。";

/// Without a reported status, a reply counts as finished once the visible
/// screen reads the same this many polls in a row after a change.
const SETTLED_POLLS: u32 = 2;

/// A sent (or possibly sent) message awaiting its reply.
#[derive(Clone, Debug)]
pub struct Pending {
    pub id: u64,
    /// The caller's tag for this round (the UI's activity log).
    pub label: usize,
    pub target: String,
    pub identity: Identity,
    pub turn: Option<Value>,
    pub baseline: String,
    pub english: String,
    pub sent: bool,
    pub observed_working: bool,
    /// Last visible screen polled, for targets that report no status.
    pub screen: Option<String>,
    pub stable_polls: u32,
}

/// Turns awaiting replies, oldest first. Sending appends; tracking collects
/// from the front. The lock is never held across a CLI call.
#[derive(Clone, Debug, Default)]
pub struct Turns(Arc<Mutex<VecDeque<Pending>>>);

impl Turns {
    fn lock(&self) -> MutexGuard<'_, VecDeque<Pending>> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    pub fn front(&self) -> Option<Pending> {
        self.lock().front().cloned()
    }

    /// Reads the oldest turn without copying its screens.
    pub fn with_front<R>(&self, read: impl FnOnce(&Pending) -> R) -> Option<R> {
        self.lock().front().map(read)
    }

    pub fn has_label(&self, label: usize) -> bool {
        self.lock().iter().any(|p| p.label == label)
    }

    /// Stops tracking every turn; the original terminal is not affected.
    pub fn clear(&self) {
        self.lock().clear();
    }

    fn push(&self, pending: Pending) {
        self.lock().push_back(pending);
    }

    /// Edits a turn that is still queued.
    fn update(&self, id: u64, edit: impl FnOnce(&mut Pending)) {
        if let Some(pending) = self.lock().iter_mut().find(|p| p.id == id) {
            edit(pending);
        }
    }

    fn remove(&self, id: u64) -> bool {
        let mut turns = self.lock();
        let before = turns.len();
        turns.retain(|p| p.id != id);
        turns.len() != before
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Poll {
    Unknown,
    Waiting,
    Ready,
    /// The target reported `blocked` or `error` and needs the user.
    Attention(String),
}

pub type Emit<'a> = &'a dyn Fn(String);

pub struct Session {
    pub core: Box<dyn Terminals>,
    pub translator: Box<dyn Translate>,
    /// Shared with the conversation's other session.
    pub turns: Turns,
    /// The terminal this bridge itself runs in, which is never a target.
    pub own_terminal: Option<String>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl Session {
    pub fn new(core: Box<dyn Terminals>, translator: Box<dyn Translate>) -> Self {
        Session { core, translator, turns: Turns::default(), own_terminal: own_terminal() }
    }

    /// Translates and submits a message whatever the target's status, even
    /// while earlier turns await replies. The turn is queued before sending
    /// so an uncertain send is never retried.
    pub fn start(&mut self, target: &str, chinese: &str, label: usize, emit: Emit) -> Result<()> {
        if self.own_terminal.as_deref() == Some(target) {
            return Err(BridgeError::new(format!(
                "目标 {target} 是中文桥自己所在的终端。请在 ⋯ 菜单中“选择终端”换成运行 agent 的终端。"
            )));
        }
        let first = self.core.get(target)?;
        emit("正在翻译为英文…".into());
        let english = self.translator.translate(chinese, Language::English)?;
        let now = self.core.get(target)?;
        if identity(&now) != identity(&first) {
            return Err(BridgeError::new("翻译期间目标终端中的程序已改变，未发送。"));
        }
        let baseline = self.core.read(target)?;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        self.turns.push(Pending {
            id,
            label,
            target: target.to_string(),
            identity: identity(&now),
            turn: turn(&now),
            baseline,
            english: english.clone(),
            sent: false,
            observed_working: false,
            screen: None,
            stable_polls: 0,
        });
        emit(format!("英文译文：\n{english}"));
        if let Err(error) = self.core.send(target, &english) {
            emit("发送结果未知，已保留本轮；请查看目标终端。".into());
            return Err(error);
        }
        self.turns.update(id, |p| p.sent = true);
        emit(format!("已发送到 {target}，等待回复…"));
        Ok(())
    }

    /// Whether the oldest turn's reply is complete.
    pub fn poll(&mut self) -> Result<Poll> {
        let Some(pending) = self.turns.front().filter(|p| p.sent) else {
            return Ok(Poll::Unknown);
        };
        let record = self.core.get(&pending.target)?;
        if identity(&record) != pending.identity {
            return Err(BridgeError::new("目标终端程序已改变，自动回译停止。"));
        }
        let Some(state) = status(&record) else {
            // Visible screen only: a history read would scroll a full-screen app.
            let screen = self.core.screen(&pending.target)?;
            let mut stable = 0;
            self.turns.update(pending.id, |p| {
                if p.screen.as_deref() == Some(screen.as_str()) {
                    p.stable_polls += 1;
                } else {
                    p.screen = Some(screen);
                    p.stable_polls = 0;
                }
                stable = p.stable_polls;
            });
            return Ok(if stable >= SETTLED_POLLS { Poll::Ready } else { Poll::Waiting });
        };
        let observed_working = pending.observed_working || state == "working";
        if observed_working && !pending.observed_working {
            self.turns.update(pending.id, |p| p.observed_working = true);
        }
        let advanced = match (turn(&record).and_then(|t| t.as_i64()), pending.turn.as_ref().and_then(Value::as_i64)) {
            (Some(now), Some(then)) => now > then,
            _ => false,
        };
        Ok(match state.as_str() {
            "blocked" | "error" => Poll::Attention(state),
            "idle" | "done" if advanced || observed_working => Poll::Ready,
            _ => Poll::Waiting,
        })
    }

    /// Releases only the same completed turn, never a still-running request.
    fn finish_if_ready(&mut self, id: u64) -> Result<bool> {
        if self.turns.front().map(|p| p.id) != Some(id) {
            return Ok(false);
        }
        if self.poll()? == Poll::Ready {
            return Ok(self.turns.remove(id));
        }
        Ok(false)
    }

    fn check_unchanged_turn(&self, target: &str, record: &Value) -> Result<()> {
        let now = self.core.get(target)?;
        if identity(&now) != identity(record) || turn(&now) != turn(record) {
            return Err(BridgeError::new("提取或回译期间目标开始了新任务；未显示可能过期的回答，请核对原终端。"));
        }
        Ok(())
    }

    /// Translates the latest complete answer on screen, or the oldest pending
    /// turn's reply when that turn belongs to this terminal.
    pub fn translate_screen(&mut self, target: &str, emit: Emit) -> Result<String> {
        if self.turns.front().is_some_and(|p| p.target == target) {
            return self.collect(emit);
        }
        let record = self.core.get(target)?;
        let snapshot = self.core.read(target)?;
        emit(format!("终端原始内容（仅供核对，不作为回答回译）：\n{snapshot}"));
        emit("正在识别最新完整回答…".into());
        let answer = self.translator.extract_reply(&snapshot, None, "")?;
        self.check_unchanged_turn(target, &record)?;
        emit(format!("提取的回答原文：\n{answer}"));
        emit("正在翻译为中文…".into());
        let chinese = self.translator.translate(&answer, Language::SimplifiedChinese)?;
        self.check_unchanged_turn(target, &record)?;
        emit(format!("{LATEST_CHINESE_PREFIX}{chinese}"));
        Ok(chinese)
    }

    /// Extracts and translates the oldest pending turn's reply.
    pub fn collect(&mut self, emit: Emit) -> Result<String> {
        let Some(pending) = self.turns.front() else {
            return Err(BridgeError::new("没有待取回的轮次。"));
        };
        let record = self.core.get(&pending.target)?;
        if identity(&record) != pending.identity {
            return Err(BridgeError::new("目标程序已改变，请手动选择需要回译的内容。"));
        }
        let after = self.core.read(&pending.target)?;
        if after == pending.baseline || is_blank(&after) {
            return Err(BridgeError::new("尚未发现新的回复内容。"));
        }
        emit(format!("终端原始内容（仅供核对，不作为回答回译）：\n{after}"));
        emit("正在识别本轮完整回答…".into());
        let answer = self.translator.extract_reply(&after, Some(&pending.baseline), &pending.english)?;
        self.check_unchanged_turn(&pending.target, &record)?;
        emit(format!("提取的回答原文：\n{answer}"));
        emit("正在翻译为中文…".into());
        let chinese = self.translator.translate(&answer, Language::SimplifiedChinese)?;
        self.check_unchanged_turn(&pending.target, &record)?;
        emit(format!("{CHINESE_PREFIX}{chinese}"));
        emit(if self.finish_if_ready(pending.id)? { TURN_DONE } else { TURN_CONTINUES }.into());
        Ok(chinese)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    type Extract = Box<dyn FnMut(&mut Value) -> Result<String> + Send>;

    /// Shared, mutable fake state so tests can change the terminal mid-turn.
    #[derive(Default)]
    pub struct World {
        pub record: Value,
        pub screen: String,
        pub sent: Vec<(String, String)>,
        pub send_error: Option<String>,
        pub translations: VecDeque<Result<String>>,
        pub translated: Vec<(String, Language)>,
        pub extract: Option<Extract>,
        pub extracted: Vec<(String, Option<String>, String)>,
    }

    pub type Shared = Arc<Mutex<World>>;

    pub struct FakeCore(pub Shared);
    pub struct FakeTranslator(pub Shared);

    impl Terminals for FakeCore {
        fn terminals(&self) -> Result<Vec<Value>> {
            Ok(vec![])
        }
        fn get(&self, _address: &str) -> Result<Value> {
            Ok(self.0.lock().unwrap().record.clone())
        }
        fn read(&self, _address: &str) -> Result<String> {
            Ok(self.0.lock().unwrap().screen.clone())
        }
        fn send(&self, address: &str, text: &str) -> Result<()> {
            let mut world = self.0.lock().unwrap();
            world.sent.push((address.into(), text.into()));
            match world.send_error.clone() {
                Some(error) => Err(BridgeError::new(error)),
                None => Ok(()),
            }
        }
    }

    impl Translate for FakeTranslator {
        fn translate(&self, text: &str, language: Language) -> Result<String> {
            let mut world = self.0.lock().unwrap();
            world.translated.push((text.into(), language));
            world.translations.pop_front().unwrap_or_else(|| Ok(format!("translated {text}")))
        }
        fn extract_reply(&self, screen: &str, before: Option<&str>, expected: &str) -> Result<String> {
            let mut world = self.0.lock().unwrap();
            world.extracted.push((screen.into(), before.map(String::from), expected.into()));
            let mut extract = world.extract.take();
            let result = match extract.as_mut() {
                Some(f) => f(&mut world.record),
                None => Ok("Answer in English".into()),
            };
            world.extract = extract;
            result
        }
    }

    pub fn idle(turn: i64) -> Value {
        json!({"pid": 10, "foreground": {"pid": 11, "name": "claude"}, "status": {"state": "idle"}, "turn": turn})
    }

    fn setup() -> (Session, Shared) {
        let world: Shared = Arc::new(Mutex::new(World {
            record: idle(4),
            screen: "old\n".into(),
            translations: VecDeque::from([Ok("Translate me".into()), Ok("中文回复".into())]),
            ..Default::default()
        }));
        let mut session = Session::new(Box::new(FakeCore(world.clone())), Box::new(FakeTranslator(world.clone())));
        session.own_terminal = None;
        (session, world)
    }

    fn quiet(_: String) {}

    fn set(world: &Shared, f: impl FnOnce(&mut World)) {
        f(&mut world.lock().unwrap());
    }

    #[test]
    fn existing_terminal_and_chinese_never_sent_back() {
        let (mut session, world) = setup();
        session.start("t3", "翻译我", 0, &quiet).unwrap();
        set(&world, |w| w.record = idle(5));
        assert_eq!(session.poll().unwrap(), Poll::Ready);
        set(&world, |w| w.screen = "old\nAnswer in English".into());
        assert_eq!(session.collect(&quiet).unwrap(), "中文回复");
        let w = world.lock().unwrap();
        assert_eq!(w.sent, [("t3".to_string(), "Translate me".to_string())]);
        assert_eq!(w.translated.last().unwrap(), &("Answer in English".to_string(), Language::SimplifiedChinese));
        drop(w);
        assert!(session.turns.is_empty());
    }

    #[test]
    fn send_ignores_target_status() {
        for state in [json!({"state": "working"}), json!(null)] {
            let (mut session, world) = setup();
            set(&world, |w| w.record["status"] = state);
            session.start("t3", "a", 0, &quiet).unwrap();
            assert_eq!(world.lock().unwrap().sent.len(), 1);
        }
    }

    #[test]
    fn own_terminal_never_targeted() {
        let (mut session, world) = setup();
        session.own_terminal = Some("t3".into());
        assert!(session.start("t3", "a", 0, &quiet).is_err());
        let w = world.lock().unwrap();
        assert!(w.sent.is_empty() && w.translated.is_empty());
    }

    #[test]
    fn status_less_reply_collected_once_screen_settles() {
        let (mut session, world) = setup();
        set(&world, |w| w.record["status"] = json!(null));
        session.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| w.screen = "old\n✻ Working".into());
        assert_eq!(session.poll().unwrap(), Poll::Waiting);
        set(&world, |w| w.screen = "old\nAnswer in English".into());
        assert_eq!(session.poll().unwrap(), Poll::Waiting);
        assert_eq!(session.poll().unwrap(), Poll::Waiting);
        assert_eq!(session.poll().unwrap(), Poll::Ready);
        assert_eq!(session.collect(&quiet).unwrap(), "中文回复");
        assert!(session.turns.is_empty());
        assert_eq!(world.lock().unwrap().sent.len(), 1);
    }

    #[test]
    fn uncertain_send_is_kept_and_never_retried() {
        let (mut session, world) = setup();
        set(&world, |w| w.send_error = Some("timeout".into()));
        assert!(session.start("t3", "a", 7, &quiet).is_err());
        assert_eq!(world.lock().unwrap().sent.len(), 1);
        let front = session.turns.front().unwrap();
        assert!(!front.sent && front.label == 7 && session.turns.has_label(7));
        assert_eq!(session.poll().unwrap(), Poll::Unknown);
    }

    #[test]
    fn messages_queue_behind_a_pending_reply_and_resolve_in_order() {
        let (mut session, world) = setup();
        set(&world, |w| {
            w.translations = VecDeque::from([Ok("First".into()), Ok("Second".into()), Ok("第一条回复".into())]);
        });
        session.start("t3", "一", 1, &quiet).unwrap();
        set(&world, |w| {
            w.record["status"] = json!({"state": "working"});
            w.screen = "old\nFirst\n✻ Working".into();
        });
        session.start("t3", "二", 2, &quiet).unwrap();
        assert_eq!(session.turns.len(), 2);
        assert_eq!(world.lock().unwrap().sent.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["First", "Second"]);
        set(&world, |w| {
            w.record = idle(5);
            w.screen = "old\nFirst\nAnswer one\nSecond\nAnswer two".into();
        });
        assert_eq!(session.poll().unwrap(), Poll::Ready);
        assert_eq!(session.collect(&quiet).unwrap(), "第一条回复");
        assert_eq!(world.lock().unwrap().extracted[0].2, "First");
        let next = session.turns.front().unwrap();
        assert_eq!((next.label, next.english.as_str()), (2, "Second"));
    }

    #[test]
    fn tracking_session_collects_what_the_sending_session_queued() {
        let (mut sender, world) = setup();
        let mut tracker = Session::new(Box::new(FakeCore(world.clone())), Box::new(FakeTranslator(world.clone())));
        tracker.turns = sender.turns.clone();
        sender.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| {
            w.record = idle(5);
            w.screen = "old\nAnswer in English".into();
        });
        assert_eq!(tracker.poll().unwrap(), Poll::Ready);
        assert_eq!(tracker.collect(&quiet).unwrap(), "中文回复");
        assert!(sender.turns.is_empty());
    }

    #[test]
    fn initial_idle_is_not_completion() {
        let (mut session, _) = setup();
        session.start("t3", "a", 0, &quiet).unwrap();
        assert_eq!(session.poll().unwrap(), Poll::Waiting);
    }

    #[test]
    fn replacement_agent_stops_collection() {
        let (mut session, world) = setup();
        session.start("t3", "a", 0, &quiet).unwrap();
        set(&world, |w| w.record["foreground"]["pid"] = json!(99));
        assert!(session.collect(&quiet).is_err());
    }

    #[test]
    fn repaint_translates_only_extracted_answer_without_resending() {
        let (mut session, world) = setup();
        let messages = Mutex::new(Vec::new());
        let emit = |m: String| messages.lock().unwrap().push(m);
        session.start("t3", "你好", 0, &emit).unwrap();
        set(&world, |w| {
            w.screen = "New full screen\nHello! How can I help?".into();
            w.record["turn"] = json!(5);
            w.extract = Some(Box::new(|_| Ok("Hello! How can I help?".into())));
        });
        session.collect(&emit).unwrap();
        let w = world.lock().unwrap();
        assert_eq!(w.translated.last().unwrap().0, "Hello! How can I help?");
        assert_eq!(w.extracted, [(w.screen.clone(), Some("old\n".to_string()), "Translate me".to_string())]);
        assert!(messages.lock().unwrap().iter().any(|m| m.contains("提取的回答原文")));
        assert_eq!(w.sent.len(), 1);
    }

    #[test]
    fn unchanged_screen_is_not_retranslated() {
        let (mut session, world) = setup();
        session.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| w.record["turn"] = json!(5));
        assert!(session.collect(&quiet).is_err());
        let w = world.lock().unwrap();
        assert_eq!(w.translated.len(), 1);
        assert!(w.extracted.is_empty());
    }

    #[test]
    fn manual_screen_finishes_completed_pending_turn() {
        let (mut session, world) = setup();
        session.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| {
            w.record["turn"] = json!(5);
            w.screen = "Hello!".into();
        });
        session.translate_screen("t3", &quiet).unwrap();
        assert!(session.turns.is_empty());
        assert_eq!(world.lock().unwrap().sent.len(), 1);
    }

    #[test]
    fn manual_screen_translates_running_turn_and_keeps_tracking() {
        let (mut session, world) = setup();
        session.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| {
            w.record["status"] = json!({"state": "working"});
            w.screen = "old\nStill working".into();
        });
        session.translate_screen("t3", &quiet).unwrap();
        assert!(!session.turns.is_empty());
        assert_eq!(world.lock().unwrap().extracted.len(), 1);
    }

    #[test]
    fn other_screen_does_not_release_pending_turn() {
        let (mut session, world) = setup();
        session.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| w.record["turn"] = json!(5));
        session.translate_screen("t4", &quiet).unwrap();
        assert!(!session.turns.is_empty());
    }

    #[test]
    fn failed_manual_translation_keeps_recoverable_turn() {
        let (mut session, world) = setup();
        session.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| {
            w.translations = VecDeque::from([Err(BridgeError::new("offline"))]);
            w.record["turn"] = json!(5);
            w.screen = "new".into();
        });
        assert!(session.translate_screen("t3", &quiet).is_err());
        assert!(!session.turns.is_empty());
    }

    #[test]
    fn manual_collect_during_work_keeps_tracking() {
        let (mut session, world) = setup();
        let messages = Mutex::new(Vec::new());
        let emit = |m: String| messages.lock().unwrap().push(m);
        session.start("t3", "你好", 0, &emit).unwrap();
        set(&world, |w| {
            w.record["status"] = json!({"state": "working"});
            w.screen = "old\nStill working".into();
        });
        session.collect(&emit).unwrap();
        assert!(!session.turns.is_empty());
        assert_eq!(messages.lock().unwrap().last().unwrap(), TURN_CONTINUES);
    }

    #[test]
    fn uncertain_extraction_does_not_fall_back_to_screen() {
        let (mut session, world) = setup();
        session.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| {
            w.record["turn"] = json!(5);
            w.screen = "header\nold answer\nstatus".into();
            w.extract = Some(Box::new(|_| Err(BridgeError::new("ambiguous"))));
        });
        assert!(session.collect(&quiet).is_err());
        let w = world.lock().unwrap();
        assert_eq!(w.translated.len(), 1);
        assert_eq!(w.sent.len(), 1);
        drop(w);
        assert!(!session.turns.is_empty());
    }

    #[test]
    fn new_turn_during_extraction_is_not_translated() {
        let (mut session, world) = setup();
        session.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| {
            w.record["turn"] = json!(5);
            w.screen = "new reply".into();
            w.extract = Some(Box::new(|record| {
                record["turn"] = json!(6);
                Ok("answer".into())
            }));
        });
        assert!(session.collect(&quiet).is_err());
        assert_eq!(world.lock().unwrap().translated.len(), 1);
        assert!(!session.turns.is_empty());
    }

    #[test]
    fn blocked_target_needs_attention() {
        let (mut session, world) = setup();
        session.start("t3", "你好", 0, &quiet).unwrap();
        set(&world, |w| w.record["status"] = json!({"state": "blocked"}));
        assert_eq!(session.poll().unwrap(), Poll::Attention("blocked".into()));
    }
}
