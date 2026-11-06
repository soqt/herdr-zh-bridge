//! Offscreen renders of the real interface for visual review (macOS, Metal).
//! Run: cargo run --example screenshot -- [output folder]
//! Writes one PNG per scene; no CLI is started and nothing is saved.

#[cfg(target_os = "macos")]
fn main() {
    shots::main();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("The screenshot tool needs macOS (offscreen Metal rendering).");
}

#[cfg(target_os = "macos")]
mod shots {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    use gpui::{AnyWindowHandle, AppContext, Entity, VisualTestAppContext, px, size};
    use gpui_component::Root;
    use herdr_zh_bridge::core::catalog::terminal_catalog;
    use herdr_zh_bridge::core::prefs::Preferences;
    use herdr_zh_bridge::ui::{self, Backend, Settings, preview, workspace::Workspace};
    use serde_json::json;

    fn settings(zoom: f32) -> Settings {
        Settings { hc: "hc".into(), codex: "codex".into(), herdr: "herdr-app".into(), zoom, preview: true }
    }

    fn open(
        cx: &mut VisualTestAppContext,
        width: f32,
        height: f32,
        prefs: Preferences,
        demo: bool,
    ) -> (AnyWindowHandle, Entity<Workspace>) {
        let mut workspace = None;
        let window = cx
            .open_offscreen_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|cx| Workspace::new(prefs, None, "", window, cx));
                if demo {
                    view.update(cx, |w, cx| preview::populate(w, window, cx));
                }
                workspace = Some(view.clone());
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("offscreen window");
        settle(cx);
        (window.into(), workspace.unwrap())
    }

    fn settle(cx: &mut VisualTestAppContext) {
        for _ in 0..3 {
            cx.run_until_parked();
            cx.advance_clock(Duration::from_millis(400));
        }
        cx.run_until_parked();
    }

    fn shot(cx: &mut VisualTestAppContext, window: AnyWindowHandle, folder: &Path, name: &str) {
        for _ in 0..2 {
            cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx)).unwrap();
            settle(cx);
        }
        let image = cx.capture_screenshot(window).expect("capture");
        let (w, h) = image.dimensions();
        let bmp = folder.join(format!("{name}.bmp"));
        write_bmp(&bmp, w, h, image.as_raw());
        let png = folder.join(format!("{name}.png"));
        let converted = std::process::Command::new("sips")
            .args(["-s", "format", "png"])
            .arg(&bmp)
            .arg("--out")
            .arg(&png)
            .output()
            .is_ok_and(|o| o.status.success());
        if converted {
            let _ = std::fs::remove_file(&bmp);
            println!("{}", png.display());
        } else {
            println!("{}", bmp.display());
        }
    }

    fn demo_rows() -> Vec<herdr_zh_bridge::core::catalog::TerminalRow> {
        let world = json!({"spaces": [
            {"id": "s1", "name": "herdr-zh-bridge", "machine": null, "folder": "/Users/demo/herdr-zh-bridge", "tabs": [
                {"id": "tab1", "name": "开发", "terminals": [{"terminal": "t1", "sub_agents": ["t2"]}, {"terminal": "t3"}]}]},
            {"id": "remote/s1", "name": "api-server", "machine": "build-box", "folder": "/srv/api", "tabs": [
                {"id": "remote/tab1", "name": "部署", "terminals": [{"terminal": "build-box/t1"}]}]}
        ]});
        let records = vec![
            json!({"address": "t1", "name": "Codex", "cwd": "/Users/demo/herdr-zh-bridge", "status": {"state": "idle", "app": "codex"}, "turn": 4}),
            json!({"address": "t2", "name": "Reviewer", "parent": "t1", "status": "working"}),
            json!({"address": "t3", "name": "Claude", "agent": "claude", "cwd": "/Users/demo/herdr-zh-bridge", "status": "idle"}),
            json!({"address": "build-box/t1", "name": "Grok", "cwd": "/srv/api", "status": "done"}),
            json!({"address": "t9", "agent": "zsh", "cwd": "/Users/demo", "state": "live"}),
        ];
        terminal_catalog(&records, &world)
    }

    pub fn main() {
        let folder = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "/tmp/herdr-zh-bridge-shots".into()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut cx = VisualTestAppContext::with_asset_source(
            gpui_platform::current_platform(false),
            Arc::new(gpui_kit_assets::Assets),
        );
        cx.update(|cx| ui::init(settings(1.0), Backend::system(), cx));

        let (window, _) = open(&mut cx, 1080., 760., preview::preferences(), true);
        shot(&mut cx, window, &folder, "01-split");

        let mut single = preview::preferences();
        single.tabs.truncate(1);
        single.tabs[0].target.clear();
        single.tabs[0].name.clear();
        single.split = false;
        let (window, _) = open(&mut cx, 900., 620., single, false);
        shot(&mut cx, window, &folder, "02-empty");

        let mut many = preview::preferences();
        many.split = false;
        for i in 4..14 {
            let mut tab = many.tabs[0].clone();
            tab.target = format!("t{i}");
            tab.name = format!("会话 {i} · 一个很长的终端名称 / project / feature");
            many.tabs.push(tab);
        }
        let (window, workspace) = open(&mut cx, 1080., 640., many, true);
        shot(&mut cx, window, &folder, "03-overflow");
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                let last = *w.layout().order().last().unwrap();
                w.focus_tab(last, true, window, cx);
            })
        })
        .unwrap();
        shot(&mut cx, window, &folder, "04-overflow-last");

        let (window, workspace) = open(&mut cx, 1080., 760., preview::preferences(), true);
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.apply_targets(demo_rows(), cx);
                let first = w.layout().order()[1];
                w.open_picker(first, window, cx);
            })
        })
        .unwrap();
        shot(&mut cx, window, &folder, "05-picker");

        let (window, workspace) = open(&mut cx, 1080., 760., preview::preferences(), true);
        cx.update_window(window, |_, window, cx| workspace.update(cx, |w, cx| w.open_settings(window, cx))).unwrap();
        shot(&mut cx, window, &folder, "06-settings");

        // An opened activity log shows its newest entries.
        let (window, workspace) = open(&mut cx, 1080., 760., preview::preferences(), true);
        cx.update_window(window, |_, _, cx| {
            let pane = workspace.read(cx).pane(workspace.read(cx).layout().order()[0]).unwrap().clone();
            pane.update(cx, |pane, cx| {
                let screen: String = (1..=80).map(|i| format!("{i:>3} │ terminal output line\n")).collect();
                let raw = format!("终端原始内容（仅供核对，不作为回答回译）：\n{screen}");
                pane.add_demo(
                    "再检查一遍日志。",
                    &["英文译文：\nCheck the log again.", &raw, "最后一条：已完成"],
                    None,
                );
                let last = pane.items.len() - 1;
                pane.toggle_log(last, cx);
            });
        })
        .unwrap();
        shot(&mut cx, window, &folder, "09-log-open");

        cx.update(|cx| {
            cx.global_mut::<Settings>().zoom = 1.3;
            ui::theme::set_zoom(1.3, cx);
        });
        let (window, _) = open(&mut cx, 1000., 720., preview::preferences(), true);
        shot(&mut cx, window, &folder, "07-zoom-130");

        let (window, _) = open(&mut cx, 700., 600., preview::preferences(), true);
        shot(&mut cx, window, &folder, "08-narrow-zoom-130");
    }

    fn write_bmp(path: &Path, w: u32, h: u32, rgba: &[u8]) {
        let data_len = (w * 4 * h) as usize;
        let mut out = Vec::with_capacity(54 + data_len);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&((54 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(-(h as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 16]);
        for px in rgba.as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
        std::fs::write(path, out).unwrap();
    }
}
