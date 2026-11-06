"""Multi-terminal workspace. Each tab owns an independent conversation and worker."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import tkinter as tk
from tkinter import ttk
from catalog import herdr_executable
from conversation import ConversationPane
from widgets import Appearance, COLORS, enable_dpi_awareness
from tab_group import TabGroup
from platform_support import application_home, command_modifier


class Panel:
    def __init__(self, root, target='', *, preferences_path=None):
        self.root = root
        self.preferences_path = Path(preferences_path) if preferences_path else application_home()/'preferences.json'
        self.appearance = a = Appearance(root)
        root.title('中文桥 · Herdr')
        root.geometry(f'{min(a.px(1080), root.winfo_screenwidth()-60)}x{min(a.px(880), root.winfo_screenheight()-90)}')
        root.minsize(min(a.px(680), root.winfo_screenwidth()-60), min(a.px(540), root.winfo_screenheight()-90))
        try:
            p = json.loads(self.preferences_path.read_text(encoding='utf-8'))
            if not isinstance(p, dict): p = {}
        except (OSError, ValueError):
            p = {}
            if preferences_path is None:
                try:
                    legacy=json.loads(Path(__file__).with_name('.preferences.json').read_text(encoding='utf-8'))
                    if isinstance(legacy,dict): p=legacy
                except (OSError,ValueError): pass
        self.hc_path = tk.StringVar(value=p.get('hc', 'hc'))
        self.codex_path = tk.StringVar(value=p.get('codex', 'codex'))
        self.app_path = tk.StringVar(value=p.get('herdr', herdr_executable()))
        self.tabs, self.groups = [], {}
        self.active = None
        self.split = self.closed = self.ready = False
        self.models, self.targets = [], []
        self.catalog_revision = 0
        self.settings_window = None
        self.path_entries = []
        self._drag_source = self._drag_tab = self._drop_target = None
        self._drag_position = self._drag_timer = None
        box = ttk.Frame(root, padding=a.px(8), style='Shell.TFrame'); box.pack(fill='both', expand=True)
        self.panes = tk.PanedWindow(box, orient='horizontal', bg=COLORS['border'], borderwidth=0,
                                   sashwidth=a.px(5), showhandle=False, opaqueresize=True)
        self.panes.pack(fill='both', expand=True)
        self.notebooks = [TabGroup(self.panes,a,activate=self.activate_frame,close=self.close_frame,
                                   create=lambda g=g:self.new_tab(group=g,choose=True),
                                   populate_menu=lambda menu,g=g:self.update_tabs_menu(menu,g),drag=self)
                          for g in range(2)]
        self.panes.add(self.notebooks[0], minsize=a.px(300), stretch='always')
        self.drop_hint = tk.Frame(self.panes, bg=COLORS['selection'], highlightthickness=a.px(2),
                                  highlightbackground=COLORS['accent'])
        self.drop_label = tk.Label(self.drop_hint, text='', bg=COLORS['selection'],
                                    fg=COLORS['text'], font=a.fonts['small'])
        self.drop_label.place(relx=.5,rely=.5,anchor='center')
        a.listen(self.layout_metrics)
        for group, notebook in enumerate(self.notebooks):
            notebook.bind('<<NotebookTabChanged>>', lambda e, g=group: self.tab_changed(g))
        root.bind('<FocusIn>', self.focus_changed, add='+')
        root.bind('<Button-1>', self.focus_changed, add='+')
        modifier=command_modifier()
        root.bind(f'<{modifier}-Shift-T>', lambda e: (self.new_tab(choose=True), 'break')[1])
        root.bind(f'<{modifier}-Shift-W>', lambda e: (self.close_active(), 'break')[1])
        root.bind('<Control-Tab>', self.next_tab)
        root.bind('<Escape>', self.cancel_tab_drag, add='+')
        root.bind(f'<{modifier}-comma>', lambda e:self.open_settings())
        root.bind('<Destroy>', self.destroyed, add='+')
        saved = p.get('tabs')
        if isinstance(saved, list) and saved:
            for entry in saved:
                if not isinstance(entry, dict): continue
                if entry.get('target') and self.find_target(entry['target']): continue
                self.new_tab(preferences=entry, group=1 if entry.get('group') == 1 else 0, focus=False)
        if not self.tabs:
            self.new_tab(preferences={k: p[k] for k in ('target', 'model', 'effort') if k in p}, focus=False)
        if p.get('split') and self.notebooks[1].tabs(): self.show_split()
        elif self.notebooks[1].tabs():
            for tab in list(self.tabs):
                if self.groups[tab] == 1: self.move_tab(tab, 0)
        self.collapse_empty_group()
        if target:
            selected = self.find_target(target)
            if selected is None and len(self.tabs) == 1 and not self.tabs[0].target.get():
                selected = self.tabs[0]
                selected.apply_target({'address': target, 'name': target})
            if selected is None: selected = self.new_tab(target=target, focus=False)
        else:
            index = p.get('active', 0)
            selected = self.tabs[index] if type(index) is int and 0 <= index < len(self.tabs) else self.tabs[0]
        self.ready = True
        self.focus_tab(selected, focus_input=False)
        self._drain_timer = root.after(100, self.drain)
        self._poll_timer = root.after(2000, self.tick)

    def find_target(self, address):
        return next((tab for tab in self.tabs if tab.target.get() == address), None) if address else None

    def new_tab(self, target='', *, preferences=None, group=None, choose=False, focus=True):
        existing = self.find_target(target)
        if existing:
            if focus: self.focus_tab(existing)
            return existing
        if group is None: group = self.groups.get(self.active, 0)
        p = dict(preferences or {})
        if preferences is None and self.active:
            current = self.active.preferences()
            p.update(model=current['model'], effort=current['effort'])
        if target: p['target'] = target
        tab = ConversationPane(self, p)
        self.tabs.append(tab); self.groups[tab] = group
        tab.model_revision = self.catalog_revision
        if self.models: tab.apply_models(self.models)
        tab.terminal_rows = self.targets
        self.notebooks[group].add(tab.frame, text=tab.target_name or '新会话')
        if focus: self.focus_tab(tab)
        self.refresh_tab(tab)
        self.save_preferences()
        if choose: self.root.after_idle(tab.open_targets)
        return tab

    def bind_target(self, tab, row):
        existing = self.find_target(row['address'])
        if existing and existing is not tab:
            self.focus_tab(existing); return
        if tab.target.get() and tab.target.get() != row['address']:
            tab = self.new_tab(target=row['address'])
        tab.apply_target(row)
        self.save_preferences()

    def tab_title(self, tab):
        title = tab.target_name or tab.target.get() or '新会话'
        if tab.target.get() and tab.target_name != tab.target.get(): title += ' · ' + tab.target.get()
        if tab.auto_paused: status = '需处理'
        elif tab.busy and not tab.polling: status = '处理中'
        elif tab.session.pending: status = '等待'
        elif tab.offline: status = '离线'
        elif tab.unread: status = '新回复'
        else: status = ''
        return title + (' · ' + status if status else '')

    def refresh_tab(self, tab):
        if tab not in self.groups: return
        if tab.auto_paused: status='error'
        elif tab.busy: status='working'
        elif tab.session.pending: status='waiting'
        elif tab.offline: status='offline'
        elif tab.unread: status='unread'
        else: status=''
        self.notebooks[self.groups[tab]].tab(tab.frame, text=tab.target_name or tab.target.get() or '新会话',
                                            tooltip=self.tab_title(tab), status=status,
                                            closable=not tab.busy and not tab.session.pending)
        locked = any(t.busy or t.session.pending for t in self.tabs)
        for entry in self.path_entries: entry.configure(state='disabled' if locked else 'normal')

    def is_visible(self, tab):
        group = self.groups.get(tab)
        return group is not None and (group == 0 or self.split) and self.notebooks[group].select() == str(tab.frame)

    def focus_tab(self, tab, *, focus_input=True):
        if tab not in self.groups: return
        group = self.groups[tab]
        if group == 1 and not self.split: self.show_split()
        self.notebooks[group].select(tab.frame)
        self.active = tab
        tab.unread = False
        for i,notebook in enumerate(self.notebooks): notebook.set_active(i==group)
        if focus_input: tab.input.focus_set()
        self.refresh_tab(tab)
        self.save_preferences()

    def focus_changed(self, event):
        widget = event.widget
        while widget is not None:
            tab = next((t for t in self.tabs if t.frame is widget), None)
            if tab:
                if self.active is not tab: self.focus_tab(tab, focus_input=False)
                return
            widget = getattr(widget, 'master', None)

    def tab_changed(self, group):
        selected = self.notebooks[group].select()
        tab = next((t for t in self.tabs if str(t.frame) == selected), None)
        if tab and self.ready:
            tab.unread = False
            self.refresh_tab(tab)
            if self.active is None or self.groups.get(self.active) == group:
                self.focus_tab(tab, focus_input=False)

    def activate_frame(self, frame):
        tab=next((t for t in self.tabs if t.frame is frame),None)
        if tab: self.focus_tab(tab)

    def close_frame(self, frame):
        tab=next((t for t in self.tabs if t.frame is frame),None)
        if tab and not tab.busy and not tab.session.pending:
            self.focus_tab(tab,focus_input=False); self.close_active()

    def show_split(self):
        if self.split: return
        self.split = True
        self.panes.add(self.notebooks[1], minsize=self.appearance.px(300), stretch='always')
        self.root.after_idle(self.center_split)

    def layout_metrics(self):
        if self.closed: return
        a = self.appearance
        self.root.minsize(min(a.px(680),self.root.winfo_screenwidth()-60),
                          min(a.px(540),self.root.winfo_screenheight()-90))
        for group in self.panes.panes(): self.panes.paneconfigure(group,minsize=a.px(300))

    def center_split(self):
        if not self.closed and self.split: self.panes.sash_place(0, self.panes.winfo_width() // 2, 0)

    def move_tab(self, tab, group, index=None):
        old = self.groups[tab]
        if old == group:
            if index is not None: self.notebooks[group].reorder(tab.frame,index)
            return
        self.notebooks[old].forget(tab.frame)
        self.groups[tab] = group
        self.notebooks[group].add(tab.frame, text=tab.target_name or '新会话',index=index)
        self.notebooks[group].select(tab.frame)
        self.refresh_tab(tab)

    def collapse_empty_group(self):
        if not self.split: return
        if not self.notebooks[0].tabs():
            for key in self.notebooks[1].tabs():
                tab = next(t for t in self.tabs if str(t.frame)==key)
                self.move_tab(tab,0)
        if not self.notebooks[1].tabs():
            self.panes.forget(self.notebooks[1])
            self.split = False

    def sync_tab_order(self):
        by_frame = {str(t.frame):t for t in self.tabs}
        self.tabs = [by_frame[key] for group in self.notebooks for key in group.tabs()]

    @staticmethod
    def contains(widget, x, y):
        return (widget.winfo_ismapped() and widget.winfo_rootx() <= x < widget.winfo_rootx()+widget.winfo_width()
                and widget.winfo_rooty() <= y < widget.winfo_rooty()+widget.winfo_height())

    def begin_tab_drag(self, source, frame):
        self._drag_source = source
        self._drag_tab = next((t for t in self.tabs if t.frame is frame),None)
        self._drag_timer = self.root.after(80,self.drag_scroll)

    def drag_scroll(self):
        self._drag_timer = None
        if self.closed or self._drag_tab is None: return
        if self._drag_position:
            x,y = self._drag_position
            for group in self.notebooks:
                if self.contains(group.canvas,x,y):
                    margin = self.appearance.px(28)
                    local = x-group.canvas.winfo_rootx()
                    if local < margin: group.canvas.xview_scroll(-1,'units')
                    elif local > group.canvas.winfo_width()-margin: group.canvas.xview_scroll(1,'units')
            self.update_tab_drag(x,y)
        self._drag_timer = self.root.after(80,self.drag_scroll)

    def drop_target(self, x, y):
        if self._drag_tab not in self.groups: return None
        for i,group in enumerate(self.notebooks):
            if self.contains(group.canvas,x,y):
                return 'tabs',i,group.insertion_index(x)
        if not self.contains(self.panes,x,y): return None
        if self.split:
            for i,group in enumerate(self.notebooks):
                if self.contains(group.body,x,y) and self.groups[self._drag_tab] != i:
                    return 'pane',i,len(group.tabs())
        else:
            body = self.notebooks[0].body
            if (len(self.tabs)>1 and self.contains(body,x,y)
                    and x >= body.winfo_rootx()+body.winfo_width()*.62):
                return 'split',1,0
        return None

    def update_tab_drag(self, x, y):
        if self._drag_tab is None: return
        self._drag_position = x,y
        target = self._drop_target = self.drop_target(x,y)
        self.drop_hint.place_forget()
        for i,group in enumerate(self.notebooks):
            group.mark_insertion(target[2] if target and target[0]=='tabs' and target[1]==i else None)
        if target and target[0] != 'tabs':
            body = self.notebooks[0 if target[0]=='split' else target[1]].body
            left = body.winfo_rootx()-self.panes.winfo_rootx()
            top = body.winfo_rooty()-self.panes.winfo_rooty()
            width = body.winfo_width()
            if target[0]=='split':
                left += width//2
                width -= width//2
            self.drop_label.configure(text='右侧分栏' if target[0]=='split' else '移入此分栏')
            self.drop_hint.place(x=left,y=top,width=width,height=body.winfo_height())
            self.drop_hint.lift()

    def end_tab_drag(self, x, y):
        target, tab = self.drop_target(x,y), self._drag_tab
        self.cancel_tab_drag()
        if not target or tab not in self.groups: return
        kind,group,index = target
        if kind=='split': self.show_split()
        self.move_tab(tab,group,index)
        self.collapse_empty_group()
        self.sync_tab_order()
        self.focus_tab(tab)

    def cancel_tab_drag(self, _event=None):
        if self._drag_timer:
            self.root.after_cancel(self._drag_timer)
            self._drag_timer = None
        if self._drag_source: self._drag_source.reset_drag()
        self._drag_source = self._drag_tab = self._drop_target = self._drag_position = None
        self.drop_hint.place_forget()
        for group in self.notebooks: group.mark_insertion(None)

    def close_active(self):
        if not self.active: return False
        tab = self.active
        if tab.busy or tab.session.pending: return False
        group = self.groups.pop(tab)
        self.notebooks[group].forget(tab.frame)
        self.tabs.remove(tab); tab.dispose()
        self.active = None
        self.collapse_empty_group()
        self.sync_tab_order()
        if not self.tabs: self.new_tab()
        else:
            next_tab = next((t for t in self.tabs if self.groups[t] == group), self.tabs[0])
            self.focus_tab(next_tab)
        self.save_preferences()
        return True

    def next_tab(self, _event=None):
        if self.active: self.focus_tab(self.tabs[(self.tabs.index(self.active)+1) % len(self.tabs)])
        return 'break'

    def update_tabs_menu(self, menu, group=0):
        menu.delete(0, 'end')
        menu.add_command(label='新建会话', command=lambda:self.new_tab(group=group,choose=True))
        menu.add_command(label='设置', command=self.open_settings)
        menu.add_separator()
        for tab in self.tabs:
            menu.add_command(label=self.tab_title(tab), command=lambda t=tab: self.focus_tab(t))

    def apply_models(self, records, source):
        self.models, self.catalog_revision = records, self.catalog_revision + 1
        for tab in self.tabs:
            if tab is source or (not tab.busy and not tab.session.pending):
                tab.apply_models(records); tab.model_revision = self.catalog_revision

    def apply_targets(self, rows):
        self.targets = rows
        by_address = {r['address']: r for r in rows}
        for tab in self.tabs:
            tab.terminal_rows = rows
            if tab.target.get() in by_address: tab.apply_target(by_address[tab.target.get()])
            elif tab.target.get(): tab.offline = True; tab.refresh_controls()

    def open_settings(self):
        if self.settings_window and self.settings_window.winfo_exists():
            self.settings_window.lift(); return
        w = self.settings_window = tk.Toplevel(self.root)
        w.title('设置'); w.transient(self.root); w.configure(bg=COLORS['bg'])
        box = ttk.Frame(w, padding=self.appearance.px(18)); box.pack(fill='both', expand=True)
        self.path_entries = []
        for label, var in [('Herdr core', self.hc_path), ('翻译 Codex', self.codex_path), ('Herdr', self.app_path)]:
            row = ttk.Frame(box); row.pack(fill='x', pady=5)
            ttk.Label(row, text=label, width=13).pack(side='left')
            entry = ttk.Entry(row, textvariable=var, width=50); entry.pack(side='left', fill='x', expand=True)
            self.path_entries.append(entry)
        row = ttk.Frame(box); row.pack(fill='x', pady=5)
        ttk.Label(row, text='字号', width=13).pack(side='left')
        zoom = ttk.Combobox(row, values=('100%', '115%', '130%', '150%'), state='readonly', width=8)
        zoom.set(f'{round(self.appearance.zoom*100)}%'); zoom.pack(side='left')
        zoom.bind('<<ComboboxSelected>>', lambda e: self.appearance.set_zoom(int(zoom.get().strip('%'))/100))
        ttk.Button(box, text='完成', command=self.close_settings).pack(anchor='e', pady=(10, 0))
        w.protocol('WM_DELETE_WINDOW', self.close_settings)
        if self.active: self.refresh_tab(self.active)

    def close_settings(self):
        self.save_preferences()
        self.path_entries = []
        if self.settings_window: self.settings_window.destroy()
        self.settings_window = None

    def save_preferences(self):
        if not self.ready or self.closed: return
        tabs = [dict(**t.preferences(), group=self.groups[t]) for t in self.tabs]
        data = dict(version=7, hc=self.hc_path.get(), codex=self.codex_path.get(), herdr=self.app_path.get(),
                    tabs=tabs, split=self.split, active=self.tabs.index(self.active) if self.active in self.tabs else 0)
        try:
            self.preferences_path.parent.mkdir(parents=True,exist_ok=True)
            temp = self.preferences_path.with_suffix('.tmp')
            temp.write_text(json.dumps(data, ensure_ascii=False), encoding='utf-8'); temp.replace(self.preferences_path)
        except OSError: pass

    def drain(self):
        if self.closed: return
        for tab in list(self.tabs):
            tab.drain()
            if tab.model_revision != self.catalog_revision and not tab.busy and not tab.session.pending:
                tab.apply_models(self.models); tab.model_revision = self.catalog_revision
        self._drain_timer = self.root.after(100, self.drain)

    def tick(self):
        if self.closed: return
        for tab in list(self.tabs): tab.tick()
        self._poll_timer = self.root.after(2000, self.tick)

    def destroyed(self, event):
        if event.widget is not self.root: return
        self.closed = True
        for name in ('_drain_timer', '_poll_timer', '_drag_timer'):
            timer = getattr(self, name, None)
            if timer: self.root.after_cancel(timer)
        for tab in self.tabs: tab.disposed = True


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', default='')
    parser.add_argument('--detach', action='store_true')
    parser.add_argument('--smoke-test', action='store_true')
    args = parser.parse_args()
    if args.detach:
        subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '--target', args.target],
                         creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0),
                         stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        enable_dpi_awareness()
        root = tk.Tk()
        if args.smoke_test: root.withdraw()
        workspace = Panel(root, args.target)
        if args.smoke_test:
            root.update_idletasks(); root.destroy(); print('Workspace constructed successfully')
        else: root.mainloop()
