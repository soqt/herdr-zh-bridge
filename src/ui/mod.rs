//! GPUI interface: one window with a tabbed, splittable workspace.

pub mod conversation;
pub mod layout;
pub mod models;
pub mod picker;
pub mod preview;
pub mod settings;
pub mod theme;
pub mod workspace;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, Global, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowOptions, actions,
    px, size,
};
use gpui_component::Root;

use crate::Options;
use crate::core::catalog::herdr_executable;
use crate::core::herdr::Core;
use crate::core::platform::application_home;
use crate::core::prefs::{self, Preferences};
use crate::core::session::Session;
use crate::core::translator::Translator;
use workspace::Workspace;

actions!(herdr_zh_bridge, [NewTab, CloseTab, NextTab, OpenSettings, SendMessage, CancelDrag, Quit]);

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const COMPOSER_CONTEXT: &str = "Composer";

/// Paths and zoom shared by every tab. Workers copy them when a task starts.
#[derive(Clone, Debug)]
pub struct Settings {
    pub hc: String,
    pub codex: String,
    pub herdr: String,
    pub zoom: f32,
    /// Demo mode: no CLI is ever started.
    pub preview: bool,
}

impl Global for Settings {}

/// Builds each conversation's session; tests substitute fakes.
#[derive(Clone)]
pub struct Backend(pub Arc<dyn Fn() -> Session + Send + Sync>);

impl Global for Backend {}

impl Backend {
    pub fn system() -> Self {
        Backend(Arc::new(|| Session::new(Box::new(Core::new("hc")), Box::new(Translator::new("codex")))))
    }
}

pub fn send_hint() -> &'static str {
    if cfg!(target_os = "macos") { "↩ 发送，⇧↩ 换行" } else { "Enter 发送，Shift+Enter 换行" }
}

pub fn bind_keys(cx: &mut App) {
    let workspace = Some(WORKSPACE_CONTEXT);
    // Registered after gpui_component::init, so these win over the input's
    // own Enter bindings. Shift-Enter still inserts a newline, and Enter
    // while an IME is composing goes to the IME first.
    let composer = Some("Composer > Input");
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-shift-t", NewTab, workspace),
        KeyBinding::new("cmd-shift-w", CloseTab, workspace),
        KeyBinding::new("cmd-,", OpenSettings, workspace),
        KeyBinding::new("enter", SendMessage, composer),
        KeyBinding::new("cmd-enter", SendMessage, composer),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-t", NewTab, workspace),
        KeyBinding::new("ctrl-shift-w", CloseTab, workspace),
        KeyBinding::new("ctrl-,", OpenSettings, workspace),
        KeyBinding::new("enter", SendMessage, composer),
        KeyBinding::new("ctrl-enter", SendMessage, composer),
    ]);
    cx.bind_keys([KeyBinding::new("ctrl-tab", NextTab, workspace), KeyBinding::new("escape", CancelDrag, workspace)]);
}

fn set_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu {
            name: "中文桥".into(),
            disabled: false,
            items: vec![
                MenuItem::action("设置…", OpenSettings),
                MenuItem::separator(),
                MenuItem::action("退出中文桥", Quit),
            ],
        },
        Menu {
            name: "会话".into(),
            disabled: false,
            items: vec![
                MenuItem::action("新建会话", NewTab),
                MenuItem::action("关闭会话", CloseTab),
                MenuItem::action("下一个会话", NextTab),
            ],
        },
    ]);
}

/// Registers components, theme and bindings on a fresh app.
pub fn init(settings: Settings, backend: Backend, cx: &mut App) {
    gpui_component::init(cx);
    theme::apply(settings.zoom, cx);
    cx.set_global(settings);
    cx.set_global(backend);
    bind_keys(cx);
    cx.on_action(|_: &Quit, cx| cx.quit());
}

pub fn preferences_path() -> PathBuf {
    application_home().join("preferences.json")
}

pub fn run(options: Options) {
    let (prefs, prefs_path) = if options.preview {
        (preview::preferences(), None)
    } else {
        let path = preferences_path();
        (prefs::load(&path, prefs::legacy_path().as_deref(), herdr_executable()), Some(path))
    };
    let settings = Settings {
        hc: prefs.hc.clone(),
        codex: prefs.codex.clone(),
        herdr: prefs.herdr.clone(),
        zoom: prefs.zoom,
        preview: options.preview,
    };
    gpui_platform::application().with_assets(gpui_kit_assets::Assets).run(move |cx: &mut App| {
        init(settings, Backend::system(), cx);
        set_menus(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        open_window(prefs, prefs_path, options, cx);
        cx.activate(true);
    });
}

fn open_window(prefs: Preferences, prefs_path: Option<PathBuf>, options: Options, cx: &mut App) {
    let zoom = prefs.zoom;
    let screen = cx.primary_display().map(|d| d.bounds().size);
    let fit =
        |want: f32, margin: f32, available: Option<f32>| available.map_or(want, |a| want.min(a - margin)).max(320.);
    let width = fit(1080. * zoom, 60., screen.map(|s| f32::from(s.width)));
    let height = fit(880. * zoom, 90., screen.map(|s| f32::from(s.height)));
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    let title = if options.preview { "中文桥 · 界面预览" } else { "中文桥 · Herdr" };
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(title.into()), ..Default::default() }),
            window_min_size: Some(size(px(680. * zoom.min(1.15)), px(540.))),
            show: !options.smoke_test,
            ..Default::default()
        },
        |window, cx| {
            let target = options.target.clone();
            let view = cx.new(|cx| Workspace::new(prefs, prefs_path, &target, window, cx));
            if options.preview {
                view.update(cx, |workspace, cx| preview::populate(workspace, window, cx));
            }
            cx.new(|cx| Root::new(view, window, cx))
        },
    );
    if let Err(error) = window {
        eprintln!("无法打开窗口：{error}");
        cx.quit();
        return;
    }
    if options.smoke_test {
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(300)).await;
            println!("Workspace constructed successfully");
            cx.update(|cx| cx.quit());
        })
        .detach();
    }
}
