//! Neutral dark palette and zoom. Every dimension is in rems, so the font
//! size setting scales the whole interface; fixed sizes stay fixed per zoom.

use gpui::{App, Hsla, Pixels, Rems, Window, px, rems, rgb};
use gpui_component::{Theme, ThemeMode};

use crate::core::platform::font_families;

pub const BASE_FONT: f32 = 14.0;
const BASE_MONO: f32 = 13.0;

/// Logical pixels at 100% zoom.
pub fn u(value: impl Into<f64>) -> Rems {
    rems(value.into() as f32 / BASE_FONT)
}

/// The same logical units in pixels at the current zoom.
pub fn to_px(value: f32, window: &Window) -> Pixels {
    window.rem_size() * (value / BASE_FONT)
}

pub fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub const BG: u32 = 0x101113;
pub const CARD: u32 = 0x181a1d;
pub const FIELD: u32 = 0x202328;
pub const TEXT: u32 = 0xe8eaee;
pub const MUTED: u32 = 0x959ba5;
pub const FAINT: u32 = 0x606670;
pub const BORDER: u32 = 0x30343a;
pub const ACCENT: u32 = 0x9caafb;
pub const ACCENT_TEXT: u32 = 0x141827;
pub const SELECTION: u32 = 0x343d5d;
pub const SELECTED: u32 = 0x272b32;
pub const HOVER: u32 = 0x202329;
pub const FOCUS: u32 = 0x626d88;
pub const ERROR: u32 = 0xee9696;
pub const WARNING: u32 = 0xd6b47a;
pub const GREEN: u32 = 0x8db9a0;

pub fn apply(zoom: f32, cx: &mut App) {
    // The mode switch reloads the stock palette; override it afterwards.
    Theme::change(ThemeMode::Dark, None, cx);
    let (family, mono) = font_families();
    Theme::update(cx, |t| {
        t.background = color(BG);
        t.foreground = color(TEXT);
        t.border = color(BORDER);
        t.muted = color(FIELD);
        t.muted_foreground = color(MUTED);
        t.accent = color(HOVER);
        t.accent_foreground = color(TEXT);
        t.primary = color(ACCENT);
        t.primary_hover = color(0xb2bdff);
        t.primary_active = color(0x8796ef);
        t.primary_foreground = color(ACCENT_TEXT);
        t.button_primary = color(ACCENT);
        t.button_primary_hover = color(0xb2bdff);
        t.button_primary_active = color(0x8796ef);
        t.button_primary_foreground = color(ACCENT_TEXT);
        t.secondary = color(FIELD);
        t.secondary_hover = color(SELECTED);
        t.secondary_active = color(SELECTED);
        t.secondary_foreground = color(TEXT);
        t.input = color(BORDER);
        t.ring = color(FOCUS);
        t.caret = color(ACCENT);
        t.selection = color(SELECTION);
        t.popover = color(FIELD);
        t.popover_foreground = color(TEXT);
        t.list_hover = color(SELECTED);
        t.list_active = color(SELECTION);
        t.list_active_border = color(SELECTION);
        t.drop_target = color(SELECTION).opacity(0.6);
        t.drag_border = color(ACCENT);
        t.scrollbar = color(CARD).opacity(0.0);
        t.scrollbar_thumb = color(BORDER);
        t.scrollbar_thumb_hover = color(FOCUS);
        t.overlay = color(0x000000).opacity(0.45);
        t.font_family = family.into();
        t.mono_font_family = mono.into();
        t.radius = px(6.);
        t.radius_lg = px(8.);
    });
    set_zoom(zoom, cx);
}

/// 1rem is the theme font size (Root applies it every frame).
pub fn set_zoom(zoom: f32, cx: &mut App) {
    Theme::update(cx, |t| {
        t.font_size = px(BASE_FONT * zoom);
        t.mono_font_size = px(BASE_MONO * zoom);
    });
}
