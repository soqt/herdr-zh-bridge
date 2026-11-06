import { View, div } from 'gpui-kit';
import { v_flex } from 'gpui-base';
import { focused, appHome, exec, theme, window as herdrWindow } from 'herdr';

// Launcher for the native Herdr ZH Bridge app installed beside this mod.
// Uses only documented Herdr APIs; run herdr-app check locally.
function bridgeBinary() {
  const name = herdrWindow().platform === 'windows' ? 'herdr-zh-bridge.exe' : 'herdr-zh-bridge';
  return appHome() + '/mods/zh-bridge/' + name;
}

export default class TranslatorLauncher extends View {
  init() { this.message = '选中目标终端，然后打开中文桥。'; this.busy = false; }
  render(cx) {
    const t = theme();
    return v_flex().size_full().p(12).gap(10).bg(t.surface).text_color(t.foreground)
      .child(div().text_size(16).font_semibold().child('中文桥'))
      .child(div().text_size(12).child(this.message))
      .child(div().id('launch-translator').p(10).rounded(6).bg(t.accent)
        .text_color(t.accent_foreground).child(this.busy ? '正在打开…' : '打开中文桥')
        .on_click((_event, cx) => {
          if (this.busy) return;
          const target = focused()?.terminal;
          if (!target) { this.message = '请先选中一个目标终端窗格。'; cx.notify(); return; }
          this.busy = true;
          cx.spawn(async (cx) => {
            try {
              // --detach returns at once; the window outlives exec's time limit.
              const result = await exec(bridgeBinary(), ['--detach', '--target', target]);
              if (result.code && result.code !== 0) throw new Error(result.stderr || '启动失败');
              this.message = '已请求打开中文桥，目标：' + target;
            } catch (e) { this.message = String(e); }
            this.busy = false;
            cx.notify();
          });
        }));
  }
}
