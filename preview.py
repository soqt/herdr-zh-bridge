"""UI-only preview: no CLI calls or sending."""
import tempfile
from pathlib import Path
import tkinter as tk
from panel import Panel
from widgets import enable_dpi_awareness

enable_dpi_awareness()
with tempfile.TemporaryDirectory() as folder:
    root=tk.Tk()
    app=Panel(root,'preview-one',preferences_path=Path(folder)/'preferences.json')
    root.title('中文桥 · 0.7 UI Preview')
    first=app.active
    first.target_name='Codex · 界面设计'; app.refresh_tab(first)
    second=app.new_tab('preview-two'); second.target_name='Claude · 代码审查'; app.refresh_tab(second)
    third=app.new_tab('preview-three'); third.target_name='Grok · API 文档'; app.refresh_tab(third)
    for tab in app.tabs:
        tab.task=lambda *_args,**_kwargs:None
        tab.tick=lambda:None
        tab.model.set('GPT-6.1 Sol'); tab.effort.set('high')
    first.add_chat('你','把标签页做成固定大小，切换时不要改变布局。')
    first.begin_log(); first.log('英文已发送\nMake the tabs fixed-size and keep the layout stable when switching.')
    first.log('已提取本轮回复正文。')
    first.add_chat('','已调整标签栏：\n\n所有标签使用相同的宽度和高度。选中状态只改变颜色，任务状态用指示点显示。\n\n较长名称会自动省略，悬停可以查看完整信息。')
    first.input.insert('1.0','继续优化输入框的间距')
    third.add_chat('你','检查 API 的错误处理。')
    third.begin_log(); third.log('Request translated and submitted.\nWaiting for the terminal to finish.')
    third.add_chat('','建议统一错误响应结构，并为超时、权限不足和请求限流分别提供明确的处理方式。')
    app.show_split(); app.move_tab(third,1); app.focus_tab(first)
    root.after(180000,root.destroy)
    root.mainloop()
