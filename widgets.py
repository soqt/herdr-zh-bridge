import ctypes
import json
import sys
import tkinter as tk
from tkinter import ttk, font
from tkinter.scrolledtext import ScrolledText
from platform_support import font_families

COLORS = dict(bg='#101113', card='#181a1d', field='#202328', text='#e8eaee', muted='#959ba5',
              border='#30343a', accent='#9caafb', selection='#343d5d', selected='#272b32',
              hover='#202329', focus='#626d88', error='#ee9696', warning='#d6b47a', green='#8db9a0')


def enable_dpi_awareness():
    # Must run before creating any Tk window; never change system-wide settings.
    if sys.platform != 'win32': return
    try:
        ctypes.windll.user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
    except (AttributeError, OSError):
        try: ctypes.windll.shcore.SetProcessDpiAwareness(2)
        except (AttributeError, OSError): ctypes.windll.user32.SetProcessDPIAware()


class Appearance:
    def __init__(self, root):
        self.root, self.dpi, self.zoom = root, 96, 1.0
        self.listeners = []
        self.style = ttk.Style(root)
        self.style.theme_use('clam')
        family, mono = font_families()
        self.fonts = {name: font.Font(root=root, family=family, size=size, weight=weight)
                      for name, size, weight in [('body', 11, 'normal'), ('small', 10, 'normal'),
                                                 ('title', 23, 'bold'), ('heading', 12, 'bold'), ('mono', 10, 'normal')]}
        self.fonts['mono'].configure(family=mono)
        root.bind('<Configure>', self.monitor_changed, add='+')
        self.apply()

    def actual_dpi(self):
        # Aqua draws backing pixels at Retina scale; layout units stay logical.
        if sys.platform == 'darwin': return 96
        if sys.platform == 'win32':
            try:
                fn = ctypes.windll.user32.GetDpiForWindow
                fn.argtypes, fn.restype = [ctypes.c_void_p], ctypes.c_uint
                return fn(self.root.winfo_id()) or 96
            except (AttributeError, OSError): pass
        return round(self.root.winfo_fpixels('1i'))

    def px(self, value): return max(1, round(value * self.dpi / 96 * self.zoom))

    def apply(self):
        self.dpi = self.actual_dpi()
        self.root.tk.call('tk', 'scaling', self.dpi / 72 * self.zoom)
        for item in self.fonts.values(): item.configure(size=item.cget('size'))
        c, s = COLORS, self.style
        self.root.configure(bg=c['bg'])
        s.configure('.', font=self.fonts['body'], background=c['card'], foreground=c['text'])
        s.configure('TFrame', background=c['card'])
        s.configure('Shell.TFrame', background=c['bg'])
        s.configure('Input.TFrame', background=c['field'])
        s.configure('Card.TFrame', background=c['card'])
        s.configure('TLabel', background=c['card'], foreground=c['text'])
        s.configure('Shell.TLabel', background=c['bg'], foreground=c['muted'], font=self.fonts['small'])
        s.configure('Muted.TLabel', foreground=c['muted'], font=self.fonts['small'])
        s.configure('Title.TLabel', font=self.fonts['title'])
        s.configure('TButton', font=self.fonts['small'], background=c['field'], borderwidth=0,
                    bordercolor=c['field'], lightcolor=c['field'], darkcolor=c['field'],
                    padding=(self.px(11), self.px(7)), relief='flat', focusthickness=0)
        s.map('TButton', background=[('active', c['hover']), ('disabled', c['card'])], foreground=[('disabled', '#606670')])
        s.configure('Ghost.TButton', background=c['bg'], padding=(self.px(7), self.px(5)))
        s.configure('Log.TButton', background=c['card'], foreground=c['muted'], padding=(self.px(4), self.px(4)))
        s.configure('Primary.TButton', background=c['accent'], foreground='#141827', font=self.fonts['small'])
        s.map('Primary.TButton', background=[('disabled', '#303546'), ('active', '#b2bdff')], foreground=[('disabled', '#707a99')])
        s.configure('TMenubutton', font=self.fonts['small'], background=c['field'], borderwidth=0, padding=self.px(6))
        s.layout('Composer.TMenubutton', [('Menubutton.border', {'sticky': 'nswe', 'children': [
            ('Menubutton.padding', {'sticky': 'nswe', 'children': [('Menubutton.label', {'sticky': 'nswe'})]})]})])
        s.configure('Composer.TMenubutton', anchor='center', padding=self.px(4))
        s.map('Composer.TMenubutton', background=[('active', c['hover'])])
        s.layout('Header.TMenubutton', s.layout('Composer.TMenubutton'))
        s.configure('Header.TMenubutton', background=c['bg'], padding=self.px(6))
        s.map('Header.TMenubutton', background=[('active', c['hover'])])
        s.configure('InputIcon.TButton', background=c['field'], padding=self.px(4), anchor='center')
        s.configure('TEntry', fieldbackground=c['field'], foreground=c['text'], insertcolor=c['text'], padding=self.px(8), bordercolor=c['border'])
        s.configure('TCombobox', font=self.fonts['small'], fieldbackground=c['field'], background=c['field'],
                    foreground=c['muted'], arrowcolor=c['muted'], padding=self.px(5), borderwidth=0,
                    bordercolor=c['field'], lightcolor=c['field'], darkcolor=c['field'])
        s.map('TCombobox', fieldbackground=[('readonly', c['field'])], foreground=[('disabled', c['muted'])])
        s.configure('Treeview', background=c['field'], fieldbackground=c['field'], foreground=c['text'], rowheight=self.px(33), bordercolor=c['border'])
        s.map('Treeview', background=[('selected', c['selection'])], foreground=[('selected', c['text'])])
        s.configure('Treeview.Heading', background=c['card'], foreground=c['muted'], padding=self.px(6))
        s.configure('TNotebook', background=c['bg'], borderwidth=0)
        s.configure('TNotebook.Tab', padding=(self.px(16), self.px(7)), background=c['card'])
        s.map('TNotebook.Tab', background=[('selected', c['selection'])])
        s.configure('Vertical.TScrollbar', background=c['border'], troughcolor=c['card'], borderwidth=0,
                    arrowcolor=c['card'], arrowsize=self.px(8), width=self.px(8))
        self.root.option_add('*TCombobox*Listbox.font', str(self.fonts['body']))
        self.root.option_add('*TCombobox*Listbox.background', c['field'])
        self.root.option_add('*TCombobox*Listbox.foreground', c['text'])
        self.root.option_add('*Menu.background', c['field'])
        self.root.option_add('*Menu.foreground', c['text'])
        self.root.option_add('*Menu.activeBackground', c['selected'])
        self.root.option_add('*Menu.activeForeground', c['text'])
        self.root.option_add('*Menu.font', str(self.fonts['small']))
        for listener in self.listeners:
            try: listener()
            except tk.TclError: pass  # A closed transient window may still have a listener.

    def listen(self, callback): self.listeners.append(callback)

    def monitor_changed(self, event):
        if event.widget == self.root and abs(self.actual_dpi() - self.dpi) > 1:
            self.apply()

    def set_zoom(self, value):
        self.zoom = value
        self.apply()


def text_area(parent, appearance, **kwargs):
    background=kwargs.pop('bg',COLORS['card'])
    widget=ScrolledText(parent, wrap='word', font=appearance.fonts['body'], bg=background,
                        fg=COLORS['text'], insertbackground=COLORS['accent'],
                        selectbackground=COLORS['selection'], relief='flat', borderwidth=0,
                        highlightthickness=0, padx=appearance.px(18), pady=appearance.px(12), **kwargs)
    widget.vbar.destroy()
    widget.vbar=ttk.Scrollbar(widget.frame,orient='vertical',command=widget.yview)
    widget.vbar.pack(side='right',fill='y')
    widget.configure(yscrollcommand=widget.vbar.set)
    widget.frame.configure(bg=background)
    return widget


class Tooltip:
    def __init__(self, widget, text):
        self.widget,self.text,self.window,self.timer=widget,text,None,None
        widget.bind('<Enter>',self.schedule,add='+')
        widget.bind('<Leave>',self.hide,add='+')
        widget.bind('<ButtonPress>',self.hide,add='+')
        widget.bind('<Destroy>',self.hide,add='+')

    def schedule(self,_event=None):
        self.hide()
        self.timer=self.widget.after(550,self.show)

    def show(self):
        self.timer=None
        value=self.text() if callable(self.text) else self.text
        if not value or not self.widget.winfo_exists(): return
        w=self.window=tk.Toplevel(self.widget)
        w.overrideredirect(True)
        ttk.Label(w,text=value,padding=8).pack()
        w.geometry(f'+{self.widget.winfo_pointerx()+10}+{self.widget.winfo_pointery()+14}')

    def hide(self,_event=None):
        if self.timer:
            self.widget.after_cancel(self.timer); self.timer=None
        if self.window:
            self.window.destroy(); self.window=None


class ActivityLog:
    """A collapsed, expandable record embedded directly in the conversation."""
    def __init__(self, chat, appearance):
        self.frame = ttk.Frame(chat)
        self.expanded, self.count = False, 0
        self.button = ttk.Button(self.frame, text='▸ 活动日志', style='Log.TButton', command=self.toggle)
        self.button.grid(row=0, column=0, sticky='w')
        self.body = text_area(self.frame, appearance, width=1, height=10, state='disabled')
        self.body.grid(row=1, column=0, sticky='ew', pady=(4, 6))
        self.body.grid_remove()

    def set_width(self, width):
        self.frame.columnconfigure(0, minsize=width)

    def append(self, text):
        self.count += 1
        self.body.configure(state='normal')
        self.body.insert('end', text + '\n\n')
        self.body.see('end'); self.body.configure(state='disabled')
        self.update_title()

    def update_title(self):
        self.button.configure(text=f'{"▾" if self.expanded else "▸"} 活动日志 · {self.count}')

    def toggle(self):
        self.expanded = not self.expanded
        if self.expanded: self.body.grid()
        else: self.body.grid_remove()
        self.update_title()


class TerminalPicker:
    """Tree dropdown: headings cannot select a terminal; IDs never come from labels."""
    def __init__(self, parent, anchor, appearance, rows, selected, choose):
        self.rows, self.choose, self.by_item = rows, choose, {}
        self.window = tk.Toplevel(parent)
        w = self.window
        w.title('选择 Herdr 目标终端')
        w.transient(parent)
        w.configure(bg=COLORS['bg'])
        width = min(appearance.px(1080), parent.winfo_screenwidth() - 40)
        height = min(appearance.px(650), parent.winfo_screenheight() - 80)
        x = min(max(0, anchor.winfo_rootx()), parent.winfo_screenwidth() - width)
        y = min(max(0, anchor.winfo_rooty() + anchor.winfo_height()), parent.winfo_screenheight() - height - 40)
        w.geometry(f'{width}x{height}+{x}+{y}')
        body = ttk.Frame(w, padding=appearance.px(16))
        body.pack(fill='both', expand=True)
        ttk.Label(body, text='选择目标终端', font=appearance.fonts['heading']).pack(anchor='w')
        self.query = tk.StringVar()
        search = ttk.Entry(body, textvariable=self.query)
        search.pack(fill='x', pady=appearance.px(8))
        ttk.Label(body, text='按机器 / 工作区 / 标签页分组 · 搜索名称、目录、ID 或任意详情字段', style='Muted.TLabel').pack(anchor='w')
        container = ttk.Frame(body)
        container.pack(fill='both', expand=True, pady=appearance.px(8))
        self.tree = ttk.Treeview(container, columns=('app', 'status', 'cwd', 'address'), selectmode='browse')
        self.tree.heading('#0', text='Herdr 层级 / 终端名称')
        self.tree.column('#0', width=appearance.px(300), minwidth=180)
        for key, title, size in [('app', '程序', 130), ('status', '状态', 90), ('cwd', '完整工作目录', 280), ('address', '终端 ID', 170)]:
            self.tree.heading(key, text=title)
            self.tree.column(key, width=appearance.px(size), minwidth=80, stretch=False)
        sy = ttk.Scrollbar(container, orient='vertical', command=self.tree.yview)
        sx = ttk.Scrollbar(container, orient='horizontal', command=self.tree.xview)
        self.tree.configure(yscrollcommand=sy.set, xscrollcommand=sx.set)
        self.tree.grid(row=0, column=0, sticky='nsew'); sy.grid(row=0, column=1, sticky='ns'); sx.grid(row=1, column=0, sticky='ew')
        container.rowconfigure(0, weight=1); container.columnconfigure(0, weight=1)
        ttk.Label(body, text='完整记录 · 可选择、复制；没有删除字段', style='Muted.TLabel').pack(anchor='w')
        self.details = text_area(body, appearance, height=6, state='disabled')
        self.details.pack(fill='x', pady=appearance.px(6))
        footer = ttk.Frame(body); footer.pack(fill='x')
        self.count = tk.StringVar()
        ttk.Label(footer, textvariable=self.count, style='Muted.TLabel').pack(side='left')
        self.accept_button = ttk.Button(footer, text='使用此终端', style='Primary.TButton', command=self.accept, state='disabled')
        self.accept_button.pack(side='right')
        ttk.Button(footer, text='取消', command=w.destroy).pack(side='right', padx=8)
        self.tree.bind('<<TreeviewSelect>>', self.on_select)
        self.tree.bind('<Double-1>', lambda e: self.accept())
        w.bind('<Return>', lambda e: self.accept())
        w.bind('<Escape>', lambda e: w.destroy())
        self.query.trace_add('write', lambda *_: self.populate())
        self.populate(selected)
        search.focus_set()

    def populate(self, selected=''):
        self.tree.delete(*self.tree.get_children())
        self.by_item.clear()
        groups, query, count = {}, self.query.get().casefold().strip(), 0
        for row in self.rows:
            if query and query not in json.dumps(row, ensure_ascii=False).casefold(): continue
            parent, path = '', ()
            for key, label in row['groups']:
                path += (key,)
                if path not in groups:
                    groups[path] = self.tree.insert(parent, 'end', text=label, open=True)
                parent = groups[path]
            item = self.tree.insert(parent, 'end', text=row['name'], values=(row['app'], row['status'], row['cwd'], row['address']))
            self.by_item[item] = row
            count += 1
            if row['address'] == selected: self.tree.selection_set(item); self.tree.see(item)
        self.count.set(f'{count} / {len(self.rows)} 个终端')
        self.on_select()

    def on_select(self, _event=None):
        selection = self.tree.selection()
        row = self.by_item.get(selection[0]) if selection else None
        self.accept_button.configure(state='normal' if row else 'disabled')
        self.details.configure(state='normal'); self.details.delete('1.0', 'end')
        if row: self.details.insert('end', json.dumps(row['raw'], ensure_ascii=False, indent=2))
        self.details.configure(state='disabled')

    def accept(self):
        selection = self.tree.selection()
        row = self.by_item.get(selection[0]) if selection else None
        if row:
            self.choose(row)
            self.window.destroy()
