import { View, div } from 'gpui-kit';
import { v_flex } from 'gpui-base';
import { focused, appHome, exec, theme } from 'herdr';

// Preview launcher. Uses only documented Herdr APIs; run herdr-app check locally.
export default class TranslatorLauncher extends View {
  init() { this.message = '选中目标终端，然后打开中文面板。'; this.busy = false; }
  render(cx) {
    const t = theme();
    return v_flex().size_full().p(12).gap(10).bg(t.surface).text_color(t.foreground)
      .child(div().text_size(16).font_semibold().child('中文翻译 · 预览版'))
      .child(div().text_size(12).child(this.message))
      .child(div().id('launch-translator').p(10).rounded(6).bg(t.accent)
        .text_color(t.accent_foreground).child(this.busy ? '正在打开…' : '打开中文面板')
        .on_click((_event, cx) => {
          if (this.busy) return;
          const target = focused()?.terminal;
          if (!target) { this.message = '请先选中一个目标终端窗格。'; cx.notify(); return; }
          this.busy = true;
          cx.spawn(async (cx) => {
            try {
              const result = await exec('python', [appHome() + '/mods/zh-bridge/panel.py', '--detach', '--target', target]);
              if (result.code && result.code !== 0) throw new Error(result.stderr || '启动失败');
              this.message = '已请求打开面板，目标：' + target;
            } catch (e) { this.message = String(e); }
            this.busy = false;
            cx.notify();
          });
        }));
  }
}
