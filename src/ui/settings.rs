//! Shared settings: CLI paths and interface zoom.

use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, WeakEntity, Window, div,
};
use gpui_component::{
    Sizable, WindowExt,
    button::{Button, ButtonVariants},
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use super::Settings;
use super::theme::{self, MUTED, color, hover_frame, u};
use super::workspace::Workspace;
use crate::core::prefs::ZOOM_LEVELS;

#[derive(Clone, Copy)]
enum Field {
    Hc,
    Codex,
    Herdr,
}

pub struct SettingsPanel {
    workspace: WeakEntity<Workspace>,
    fields: Vec<(&'static str, Field, Entity<InputState>)>,
    _subscriptions: Vec<Subscription>,
}

impl SettingsPanel {
    fn new(workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.global::<Settings>().clone();
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for (label, field, value) in [
            ("Herdr core", Field::Hc, settings.hc),
            ("翻译 Codex", Field::Codex, settings.codex),
            ("Herdr", Field::Herdr, settings.herdr),
        ] {
            let input = cx.new(|cx| {
                let mut state = InputState::new(window, cx);
                state.set_value(value, window, cx);
                state
            });
            subscriptions.push(cx.subscribe(&input, move |this, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = state.read(cx).value().to_string();
                    let settings = cx.global_mut::<Settings>();
                    match field {
                        Field::Hc => settings.hc = value,
                        Field::Codex => settings.codex = value,
                        Field::Herdr => settings.herdr = value,
                    }
                    this.save(cx);
                }
            }));
            fields.push((label, field, input));
        }
        SettingsPanel { workspace, fields, _subscriptions: subscriptions }
    }

    fn save(&self, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.read(cx).save_preferences(cx);
        }
    }

    fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        cx.global_mut::<Settings>().zoom = zoom;
        theme::set_zoom(zoom, cx);
        self.save(cx);
        cx.notify();
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let zoom = cx.global::<Settings>().zoom;
        let row = |label: &'static str| {
            h_flex().w_full().gap(u(12)).child(div().w(u(96)).flex_shrink_0().text_color(color(MUTED)).child(label))
        };
        v_flex()
            .w_full()
            .gap(u(10))
            .text_size(u(13))
            .children(
                self.fields
                    .iter()
                    .map(|(label, _, input)| row(label).child(div().flex_1().min_w_0().child(Input::new(input)))),
            )
            .child(row("字号").children(ZOOM_LEVELS.into_iter().map(|level| {
                let id = (level * 100.) as usize;
                let button = Button::new(("zoom", id))
                    .small()
                    .label(format!("{}%", (level * 100.).round()))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_zoom(level, cx)));
                if (level - zoom).abs() < 0.01 {
                    button.primary().into_any_element()
                } else {
                    hover_frame(("zoom-frame", id)).child(button.outline()).into_any_element()
                }
            })))
            .child(
                div()
                    .text_size(u(12))
                    .text_color(color(MUTED))
                    .child("名称会在 PATH 及常见安装目录中查找，也可以填写完整路径。新路径用于之后的操作。"),
            )
    }
}

pub fn open(workspace: WeakEntity<Workspace>, window: &mut Window, cx: &mut gpui::App) {
    if window.has_active_dialog(cx) {
        return;
    }
    let panel = cx.new(|cx| SettingsPanel::new(workspace, window, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .title("设置")
            .w(theme::to_px(560., window))
            .keyboard(true)
            .overlay_closable(true)
            .child(panel.clone())
            .footer(DialogFooter::new().child(
                Button::new("settings-done").primary().label("完成").on_click(|_, window, cx| window.close_dialog(cx)),
            ))
    });
}
