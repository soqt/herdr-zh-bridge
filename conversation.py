"""One terminal-bound conversation; workers and queues never follow UI focus."""
import queue
import threading
import tkinter as tk
from tkinter import ttk
from bridge import Core, Translator, Session
from catalog import load_terminals, list_models, reasoning_efforts
from widgets import TerminalPicker, ActivityLog, COLORS, Tooltip, text_area
from platform_support import command_modifier

DEFAULT_MODEL = '默认模型'
DEFAULT_EFFORT = '默认'
COMMON_EFFORTS = ('low', 'medium', 'high', 'xhigh', 'max')


class ConversationPane:
    def __init__(self, workspace, preferences=None):
        p = preferences or {}
        self.workspace, self.root = workspace, workspace.root
        self.appearance = a = workspace.appearance
        self.frame = ttk.Frame(workspace.panes, padding=(a.px(12),a.px(8)))
        self.messages = queue.Queue()
        self.session = Session(Core(), Translator())
        self.busy = self.polling = self.auto_paused = self.disposed = False
        self.unread = self.offline = False
        self.input_revision = 0
        self.picker = None
        self.logs, self.current_log = [], None
        self.target = tk.StringVar(value=p.get('target', ''))
        self.target_name = p.get('name') or self.target.get()
        self.target_label = tk.StringVar(value=self.target_name or '选择终端')
        self.model = tk.StringVar(value=p.get('model') or DEFAULT_MODEL)
        self.effort = tk.StringVar(value=p.get('effort') or DEFAULT_EFFORT)
        self.model_choices = {DEFAULT_MODEL: ''}
        self.model_records = {}
        self.terminal_rows = []

        top = self.target_row = ttk.Frame(self.frame)
        top.pack(fill='x', pady=(0, a.px(6)))
        self.target_button = ttk.Button(top, textvariable=self.target_label, command=self.open_targets, width=16)
        self.target_button.pack(side='left', fill='x', expand=True)
        self.chat = text_area(self.frame, a, width=1, height=12, state='disabled')
        self.chat.pack(fill='both', expand=True)
        self.chat.tag_configure('role', foreground=COLORS['muted'], font=a.fonts['small'], spacing1=a.px(20), spacing3=a.px(7))
        self.chat.tag_configure('body', spacing1=a.px(4),spacing2=a.px(3),spacing3=a.px(18))
        self.chat.bind('<Configure>', self.resize_logs)
        self.composer = tk.Frame(self.frame,bg=COLORS['field'],highlightbackground=COLORS['border'],
                                 highlightthickness=1,padx=a.px(10),pady=a.px(8))
        self.composer.pack(fill='x', pady=(a.px(10),a.px(4)))
        self.selector_area = ttk.Frame(self.composer,style='Input.TFrame'); self.selector_area.pack(fill='x', pady=(0, a.px(6)))
        self.selector_area.pack_propagate(False)
        self.model_combo = ttk.Combobox(self.selector_area, width=1, font=a.fonts['small'],
                                        textvariable=self.model, values=(DEFAULT_MODEL,), height=10)
        self.effort_combo = ttk.Combobox(self.selector_area, width=1, font=a.fonts['small'],
                                         textvariable=self.effort, state='readonly', height=8)
        Tooltip(self.model_combo,self.model_tooltip)
        Tooltip(self.effort_combo,lambda:'推理强度 · '+self.effort.get())
        self.refresh_button = ttk.Button(self.selector_area, text='↻', width=1,
                                         style='InputIcon.TButton', command=self.refresh_models)
        Tooltip(self.refresh_button,'刷新模型')
        self.more_button = ttk.Menubutton(self.selector_area, text='⋯', width=1, style='Composer.TMenubutton')
        self.selector_area.bind('<Configure>', self.layout_selectors)
        a.listen(self.layout_selectors)
        self.layout_selectors()
        Tooltip(self.more_button,'会话操作')
        self.menu = tk.Menu(self.more_button, tearoff=False, postcommand=self.update_menu)
        self.more_button.configure(menu=self.menu)
        for label, command in [('选择终端', lambda: self.list_targets(open_after=True)),
                               ('回译最新回答', self.current_screen),
                               ('仅回译输入', self.manual),
                               ('重试回译', self.collect),
                               ('结束跟踪', self.abandon)]:
            self.menu.add_command(label=label, command=command)
        self.model.trace_add('write', self.update_efforts)
        self.model_combo.bind('<<ComboboxSelected>>', lambda e: self.save_preferences())
        self.effort_combo.bind('<<ComboboxSelected>>', lambda e: self.save_preferences())
        self.input = text_area(self.composer, a, width=1, height=3,bg=COLORS['field'])
        self.input.pack(fill='x')
        self.input.bind(f'<{command_modifier()}-Return>', lambda e: (self.send(), 'break')[1])
        self.input.bind('<<Modified>>', self.input_changed)
        self.input.bind('<FocusIn>',lambda e:self.composer.configure(highlightbackground=COLORS['focus']),add='+')
        self.input.bind('<FocusOut>',lambda e:self.composer.configure(highlightbackground=COLORS['border']),add='+')
        bottom = ttk.Frame(self.composer,style='Input.TFrame'); bottom.pack(fill='x', pady=(a.px(6), 0))
        self.recovery = ttk.Button(bottom, text='重试回译', command=self.collect)
        self.send_button = ttk.Button(bottom, text='发送', style='Primary.TButton', command=self.send)
        self.send_button.pack(side='right')
        self.update_efforts()
        self.refresh_controls()

    def preferences(self):
        return dict(target=self.target.get(), name=self.target_name,
                    model=self.model_choices.get(self.model.get().strip(), self.model.get().strip()),
                    effort='' if self.effort.get() == DEFAULT_EFFORT else self.effort.get().strip())

    def layout_selectors(self, _event=None):
        if self.disposed: return
        a = self.appearance
        height, gap = a.px(32), a.px(8)
        effort_width, icon_width = a.px(84), a.px(32)
        width = self.selector_area.winfo_width()
        model_width = max(1, min(a.px(224), width - effort_width - 2 * icon_width - 3 * gap))
        self.selector_area.configure(height=height)
        self.model_combo.place(x=0, y=0, width=model_width, height=height)
        self.effort_combo.place(x=model_width + gap, y=0, width=effort_width, height=height)
        self.refresh_button.place(x=model_width + effort_width + 2 * gap, y=0, width=icon_width, height=height)
        self.more_button.place(relx=1, x=0, y=0, anchor='ne', width=icon_width, height=height)

    def model_tooltip(self):
        label = self.model.get()
        slug = self.model_choices.get(label, label)
        return '翻译模型 · ' + label + ('\n' + slug if slug and slug != label else '')

    def save_preferences(self):
        self.workspace.save_preferences()

    def configure(self):
        # Capture shared paths only before a task; other tabs cannot mutate this worker.
        self.session.core.binary = self.workspace.hc_path.get().strip()
        self.session.translator.binary = self.workspace.codex_path.get().strip()
        p = self.preferences()
        self.session.translator.model, self.session.translator.effort = p['model'], p['effort']
        self.save_preferences()

    def add_chat(self, role, text):
        self.chat.configure(state='normal')
        if role: self.chat.insert('end', role + '\n', 'role')
        self.chat.insert('end', text + '\n', 'body')
        self.chat.see('end'); self.chat.configure(state='disabled')

    def begin_log(self):
        self.current_log = ActivityLog(self.chat, self.appearance)
        self.logs.append(self.current_log)
        self.chat.configure(state='normal')
        self.chat.window_create('end', window=self.current_log.frame)
        self.chat.insert('end', '\n')
        self.chat.configure(state='disabled')
        self.resize_logs()

    def log(self, text):
        # Completion is conveyed by the reply itself, not repetitive success notices.
        if text.startswith(('本轮已完成，可以发送下一条', '已翻译当前输出；')): return
        if self.current_log is None: self.begin_log()
        self.current_log.append(text)

    def resize_logs(self, _event=None):
        width = max(80, self.chat.winfo_width() - self.appearance.px(30))
        for log in self.logs: log.set_width(width)

    def select_target(self, row):
        if self.busy or self.session.pending: return
        self.workspace.bind_target(self, row)

    def apply_target(self, row):
        self.target.set(row['address'])
        self.target_name = row['name']
        self.target_label.set(f'{row["name"]} · {row["address"]}')
        self.offline = False
        self.refresh_controls()

    def open_targets(self):
        if self.disposed or self.busy or self.session.pending: return
        if not self.terminal_rows:
            self.list_targets(open_after=True); return
        if self.picker and self.picker.window.winfo_exists(): self.picker.window.destroy()
        anchor = self.more_button if self.target.get() else self.target_button
        self.picker = TerminalPicker(self.root, anchor, self.appearance, self.terminal_rows, self.target.get(), self.select_target)

    def list_targets(self, open_after=False):
        if self.busy or self.session.pending: return
        self.configure()
        binary = self.workspace.app_path.get().strip()
        def load():
            rows, warning = load_terminals(self.session.core, binary)
            self.messages.put(('targets', (rows, warning, open_after)))
        self.task(load)

    def refresh_models(self):
        if self.busy or self.session.pending: return
        self.configure()
        binary = self.workspace.codex_path.get().strip()
        self.task(lambda: self.messages.put(('models', list_models(binary))))

    def apply_models(self, records):
        selected = self.model_choices.get(self.model.get().strip(), self.model.get().strip())
        self.model_records, self.model_choices = {}, {DEFAULT_MODEL: ''}
        for model in records:
            slug = model.get('model') or model.get('id')
            if not slug: continue
            self.model_records[slug] = model
        names = [m.get('displayName') or slug for slug, m in self.model_records.items()]
        for slug, model in self.model_records.items():
            name = model.get('displayName') or slug
            label = f'{name}  [{slug}]' if names.count(name) > 1 or name == DEFAULT_MODEL else name
            self.model_choices[label] = slug
        self.model_combo.configure(values=tuple(self.model_choices))
        self.model.set(next((label for label, slug in self.model_choices.items() if slug == selected), selected))
        self.update_efforts()

    def update_efforts(self, *_):
        slug = self.model_choices.get(self.model.get().strip(), self.model.get().strip())
        supported = reasoning_efforts(self.model_records.get(slug))
        current = self.effort.get()
        if supported is None:
            choices = list(COMMON_EFFORTS)
            if current and current != DEFAULT_EFFORT and current not in choices: choices.append(current)
        else:
            choices = supported
            if current != DEFAULT_EFFORT and current not in choices:
                self.effort.set(DEFAULT_EFFORT)
                self.log(f'{slug} 不支持 {current}，Effort 已恢复默认。')
        self.effort_combo.configure(values=(DEFAULT_EFFORT, *choices))

    def refresh_controls(self):
        if self.disposed: return
        pending = self.session.pending
        locked = self.busy or bool(pending)
        self.model_combo.configure(state='disabled' if locked else 'normal')
        self.effort_combo.configure(state='disabled' if locked else 'readonly')
        self.refresh_button.configure(state='disabled' if locked else 'normal')
        self.target_button.configure(state='disabled' if locked else 'normal')
        if self.target.get(): self.target_row.pack_forget()
        self.send_button.configure(state='disabled' if locked or self.offline or not self.target.get() else 'normal',
                                   text='发送')
        if pending and self.auto_paused and not self.busy: self.recovery.pack(side='left')
        else: self.recovery.pack_forget()
        self.workspace.refresh_tab(self)

    def update_menu(self):
        locked = self.busy or bool(self.session.pending)
        self.menu.entryconfigure(0, state='disabled' if locked else 'normal')
        for i in (1, 2): self.menu.entryconfigure(i, state='disabled' if self.busy else 'normal')
        for i in (3, 4): self.menu.entryconfigure(i, state='normal' if self.session.pending and not self.busy else 'disabled')

    def emit(self, text):
        self.messages.put(('text', str(text)))

    def task(self, fn, *, polling=False):
        if self.busy or self.disposed: return
        self.busy, self.polling = True, polling
        self.refresh_controls()
        def work():
            try: fn()
            except Exception as e: self.messages.put(('error', str(e)))
            finally: self.messages.put(('done', ''))
        threading.Thread(target=work, daemon=True).start()

    def send(self):
        if self.busy or self.session.pending or self.disposed: return
        target, draft = self.target.get().strip(), self.input.get('1.0', 'end-1c')
        if not target:
            self.open_targets(); return
        if not draft.strip() or self.offline: return
        self.configure()
        self.auto_paused = False
        self.add_chat('你', draft.strip())
        self.begin_log()
        self.input_changed()
        revision = self.input_revision
        def start():
            self.session.start(target, draft.strip(), self.emit)
            self.messages.put(('sent', (draft, revision)))
        self.task(start)

    def input_changed(self, _event=None):
        if self.input.edit_modified():
            self.input_revision += 1
            self.input.edit_modified(False)

    def collect(self):
        if self.busy or not self.session.pending: return
        self.auto_paused = False
        self.task(lambda: self.session.collect(self.emit))

    def current_screen(self):
        if self.busy: return
        target = self.target.get().strip()
        if not target: self.open_targets(); return
        if not self.session.pending: self.configure(); self.begin_log()
        self.auto_paused = False
        self.task(lambda: self.session.translate_screen(target, self.emit))

    def manual(self):
        if self.busy: return
        text = self.input.get('1.0', 'end-1c').strip()
        if not text: return
        if not self.session.pending: self.configure(); self.begin_log()
        self.task(lambda: self.emit('中文译文：\n' + self.session.translator.translate(text, 'Simplified Chinese')))

    def abandon(self):
        if self.busy: return
        self.session.pending = None
        self.auto_paused = False
        self.refresh_controls()

    def tick(self):
        if self.busy or not self.session.pending or self.auto_paused or self.disposed: return
        def poll():
            status = self.session.poll()
            if status == 'ready': self.session.collect(self.emit)
            elif status in ('blocked', 'error'):
                self.messages.put(('error', '请先处理原终端中的提示，再重试回译。'))
        self.task(poll, polling=True)

    def drain(self):
        if self.disposed: return
        while not self.messages.empty():
            kind, text = self.messages.get_nowait()
            if kind == 'done':
                self.busy = self.polling = False
            elif kind == 'sent':
                draft, revision = text
                self.input_changed()
                if self.input_revision == revision and self.input.get('1.0', 'end-1c') == draft:
                    self.input.delete('1.0', 'end')
            elif kind == 'targets':
                rows, warning, open_after = text
                self.workspace.apply_targets(rows)
                if warning: self.log(warning)
                if open_after and rows: self.root.after_idle(self.open_targets)
            elif kind == 'models':
                self.workspace.apply_models(text, source=self)
            else:
                if kind == 'error':
                    self.auto_paused = True
                    self.add_chat('需要处理', text)
                    self.unread = not self.workspace.is_visible(self)
                elif text.startswith(('中文译文：', '最新回答的中文译文：')):
                    self.auto_paused = False
                    self.add_chat('', text.split('\n', 1)[-1])
                    self.unread = not self.workspace.is_visible(self)
                self.log(text)
        self.refresh_controls()

    def dispose(self):
        self.disposed = True
        if self.picker and self.picker.window.winfo_exists(): self.picker.window.destroy()
        self.frame.destroy()
