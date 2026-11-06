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

    use gpui::{AnyWindowHandle, AppContext, Entity, Modifiers, MouseButton, VisualTestAppContext, point, px, size};
    use gpui_component::Root;
    use herdr_zh_bridge::core::catalog::Catalog;
    use herdr_zh_bridge::core::prefs::{Axis, TabPrefs, Tree};
    use herdr_zh_bridge::ui::{self, Backend, Settings, preview, workspace::Workspace};

    fn settings(zoom: f32) -> Settings {
        Settings { hc: "hc".into(), codex: "codex".into(), herdr: "herdr-app".into(), zoom, preview: true }
    }

    fn open(
        cx: &mut VisualTestAppContext,
        width: f32,
        height: f32,
        prefs: herdr_zh_bridge::core::prefs::Preferences,
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

    pub fn main() {
        let folder = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "/tmp/herdr-zh-bridge-shots".into()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut cx =
            VisualTestAppContext::with_asset_source(gpui_platform::current_platform(false), Arc::new(ui::Assets));
        cx.update(|cx| ui::init(settings(1.0), Backend::system(), cx));

        let (window, _) = open(&mut cx, 1080., 760., preview::preferences(), true);
        shot(&mut cx, window, &folder, "01-spaces");

        // A Herdr space without conversations lists its terminals.
        let (window, workspace) = open(&mut cx, 900., 620., preview::preferences(), true);
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                let playground = w.spaces().iter().find(|s| s.name == "playground").unwrap().id;
                w.select_space(playground, window, cx);
            })
        })
        .unwrap();
        shot(&mut cx, window, &folder, "02-empty");

        let mut many = preview::preferences();
        let mut tabs: Vec<TabPrefs> = many.tabs().into_iter().cloned().collect();
        for i in 7..17 {
            let mut tab = tabs[0].clone();
            tab.target = format!("t{i}");
            tab.name = format!("会话 {i} · 一个很长的终端名称 / project / feature");
            tabs.push(tab);
        }
        many.spaces.truncate(1);
        many.spaces[0].layout = Some(Tree::pane(tabs));
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

        // The + of a space offers that space's terminals.
        let (window, workspace) = open(&mut cx, 1080., 760., preview::preferences(), true);
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                let first = w.spaces()[0].id;
                w.add_session(first, window, cx);
            })
        })
        .unwrap();
        shot(&mut cx, window, &folder, "05-picker");

        let (window, workspace) = open(&mut cx, 1080., 760., preview::preferences(), true);
        cx.update_window(window, |_, window, cx| workspace.update(cx, |w, cx| w.open_settings(window, cx))).unwrap();
        shot(&mut cx, window, &folder, "06-settings");

        // Activity logs are open at a fixed height and show their newest entries.
        let (window, workspace) = open(&mut cx, 1080., 760., preview::preferences(), true);
        cx.update_window(window, |_, _, cx| {
            let pane = workspace.read(cx).pane(workspace.read(cx).layout().order()[0]).unwrap().clone();
            pane.update(cx, |pane, _| {
                let screen: String = (1..=300).map(|i| format!("{i:>3} │ terminal output line\n")).collect();
                let raw = format!("终端原始内容（仅供核对，不作为回答回译）：\n{screen}");
                pane.add_demo(
                    "再检查一遍日志。",
                    &["英文译文：\nCheck the log again.", &raw, "最后一条：已完成"],
                    None,
                );
            });
        })
        .unwrap();
        shot(&mut cx, window, &folder, "09-logs");

        cx.update(|cx| {
            cx.global_mut::<Settings>().zoom = 1.3;
            ui::theme::set_zoom(1.3, cx);
        });
        let (window, _) = open(&mut cx, 1000., 720., preview::preferences(), true);
        shot(&mut cx, window, &folder, "07-zoom-130");

        let (window, _) = open(&mut cx, 700., 600., preview::preferences(), true);
        shot(&mut cx, window, &folder, "08-narrow-zoom-130");
        let mut rail = preview::preferences();
        rail.sidebar_collapsed = true;
        let (window, _) = open(&mut cx, 700., 600., rail, true);
        shot(&mut cx, window, &folder, "08b-narrow-rail-zoom-130");

        cx.update(|cx| {
            cx.global_mut::<Settings>().zoom = 1.0;
            ui::theme::set_zoom(1.0, cx);
        });
        // Four terminals on one screen.
        let mut grid = preview::preferences();
        let cell =
            |t: &str, n: &str| Tree::pane(vec![TabPrefs { target: t.into(), name: n.into(), ..Default::default() }]);
        let column = |a, b| Tree::Split { axis: Axis::Column, children: vec![(0.5, a), (0.5, b)] };
        grid.spaces[0].layout = Some(Tree::Split {
            axis: Axis::Row,
            children: vec![
                (0.5, column(cell("preview-one", "Codex · 界面设计"), cell("g2", "Claude · 代码审查"))),
                (0.5, column(cell("preview-three", "Grok · API 文档"), cell("g4", "Codex · 测试"))),
            ],
        });
        let (window, _) = open(&mut cx, 1280., 860., grid, true);
        shot(&mut cx, window, &folder, "10-grid");

        // A tab dragged to the lower edge of its pane, over the split preview.
        let (window, _) = open(&mut cx, 1080., 760., preview::preferences(), true);
        let start = point(px(305.), px(31.));
        cx.simulate_mouse_down(window, start, MouseButton::Left, Modifiers::default());
        for to in [point(px(330.), px(31.)), point(px(420.), px(690.)), point(px(421.), px(690.))] {
            cx.simulate_mouse_move(window, to, MouseButton::Left, Modifiers::default());
        }
        shot(&mut cx, window, &folder, "11-drag-split");
        cx.simulate_mouse_up(window, point(px(421.), px(690.)), MouseButton::Left, Modifiers::default());
        settle(&mut cx);

        // A folded machine, and a space Herdr closed while it had a conversation.
        let (window, workspace) = open(&mut cx, 1080., 760., preview::preferences(), true);
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.toggle_group("machine:build-box", cx);
                let mut catalog: Catalog = preview::catalog();
                catalog.spaces.as_mut().unwrap().retain(|s| s.name != "文档");
                catalog.rows.retain(|r| r.space() != Some("s2"));
                w.apply_catalog(catalog, window, cx);
            })
        })
        .unwrap();
        shot(&mut cx, window, &folder, "12-folded-and-closed");
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
