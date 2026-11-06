"""A fixed-geometry tab strip. Selection and status never affect layout size."""
import tkinter as tk
from tkinter import ttk
from widgets import COLORS, Tooltip


def rounded(canvas, x1, y1, x2, y2, radius=6, **kwargs):
    r = min(radius, (x2-x1)/2, (y2-y1)/2)
    return canvas.create_polygon(x1+r,y1,x2-r,y1,x2,y1,x2,y1+r,x2,y2-r,x2,y2,
                                 x2-r,y2,x1+r,y2,x1,y2,x1,y2-r,x1,y1+r,x1,y1,
                                 smooth=True, splinesteps=16, **kwargs)


def elide(text, font, width):
    if font.measure(text) <= width: return text
    while text and font.measure(text+'…') > width: text=text[:-1]
    return text+'…'


class TabGroup(tk.Frame):
    def __init__(self, parent, appearance, *, activate, close, create, populate_menu, drag):
        super().__init__(parent, bg=COLORS['card'], highlightthickness=1,
                         highlightbackground=COLORS['border'], width=400, height=400)
        self.appearance, self.activate, self.close_tab = appearance, activate, close
        self.create_tab, self.drag = create, drag
        self._items, self._current, self.hover = {}, '', None
        self._press, self._dragging, self.drop_index = None, False, None
        self.header = tk.Frame(self, bg=COLORS['bg'])
        self.header.pack(fill='x'); self.header.pack_propagate(False)
        self.canvas = tk.Canvas(self.header, bg=COLORS['bg'], borderwidth=0,
                                highlightthickness=0, takefocus=True)
        self.canvas.pack(side='left', fill='both', expand=True)
        self.nav = ttk.Frame(self.header, style='Shell.TFrame'); self.nav.pack(side='right', fill='y')
        self.back_button = ttk.Button(self.nav, text='‹', width=2, style='Ghost.TButton', command=lambda:self.canvas.xview_scroll(-3,'units'))
        self.forward_button = ttk.Button(self.nav, text='›', width=2, style='Ghost.TButton', command=lambda:self.canvas.xview_scroll(3,'units'))
        self.menu_button = ttk.Menubutton(self.nav, text='⋯', width=2, style='Header.TMenubutton')
        self.menu_button.pack(side='left', fill='y')
        self.menu = tk.Menu(self.menu_button, tearoff=False)
        self.menu.configure(postcommand=lambda:populate_menu(self.menu))
        self.menu_button.configure(menu=self.menu)
        Tooltip(self.menu_button, '设置与会话')
        self.body = tk.Frame(self, bg=COLORS['card'])
        self.body.pack(fill='both', expand=True); self.body.pack_propagate(False)
        self.tooltip = Tooltip(self.canvas, self.hover_text)
        self.canvas.bind('<Configure>', self.draw)
        self.canvas.bind('<Button-1>', self.clicked)
        self.canvas.bind('<B1-Motion>', self.dragged)
        self.canvas.bind('<ButtonRelease-1>', self.released)
        self.canvas.bind('<Button-3>', self.popup_menu)
        self.canvas.bind('<Control-Button-1>', self.popup_menu)
        self.canvas.bind('<Motion>', self.motion)
        self.canvas.bind('<Leave>', lambda e:self.set_hover(None))
        self.canvas.bind('<MouseWheel>', self.wheel)
        self.canvas.bind('<Button-4>', lambda e:self.canvas.xview_scroll(-1,'units'))
        self.canvas.bind('<Button-5>', lambda e:self.canvas.xview_scroll(1,'units'))
        self.canvas.bind('<Left>', lambda e:self.step(-1))
        self.canvas.bind('<Right>', lambda e:self.step(1))
        self.appearance.listen(self.metrics)
        self.metrics()

    def metrics(self):
        self.tab_width = self.appearance.px(184)
        self.tab_height = self.appearance.px(36)
        self.header.configure(height=self.appearance.px(44))
        self.canvas.configure(xscrollincrement=self.appearance.px(40))
        self.draw()
        self.ensure_visible()

    def tabs(self): return tuple(self._items)

    def add(self, widget, *, text='', index=None):
        key=str(widget)
        self._items[key]=dict(widget=widget,text=text,tooltip=text,status='',closable=True)
        if index is not None: self.reorder(widget, index)
        if not self._current: self.select(key)
        else: self.draw()

    def forget(self, widget):
        key=str(widget)
        item=self._items.pop(key)
        item['widget'].place_forget()
        if self._current == key:
            self._current=''
            if self._items: self.select(next(iter(self._items)))
        self.draw()

    def select(self, widget=None):
        if widget is None: return self._current
        key=str(widget)
        if key not in self._items: raise tk.TclError('Unknown tab')
        if self._current == key: return key
        if self._current in self._items: self._items[self._current]['widget'].place_forget()
        self._current=key
        self._items[key]['widget'].place(in_=self.body, x=0,y=0,relwidth=1,relheight=1)
        self._items[key]['widget'].lift()
        self.draw(); self.ensure_visible()
        self.event_generate('<<NotebookTabChanged>>', when='tail')
        return key

    def tab(self, widget, **options):
        item=self._items[str(widget)]
        if not options: return item.copy()
        if any(item.get(k)!=v for k,v in options.items()):
            item.update(options); self.draw()

    def index(self, location):
        if isinstance(location,str) and location.startswith('@'):
            x=int(location[1:].split(',')[0]); n=int(self.canvas.canvasx(x)//self.tab_width)
            if not 0<=n<len(self._items): raise tk.TclError('Outside tabs')
            return n
        return list(self._items).index(str(location))

    def tab_bbox(self, widget):
        i=self.index(widget); gap=self.appearance.px(3); top=self.appearance.px(4)
        return (i*self.tab_width+gap,top,(i+1)*self.tab_width-gap,top+self.tab_height)

    def add_bbox(self):
        x = len(self._items)*self.tab_width + self.appearance.px(3)
        y = self.appearance.px(4)
        return x, y, x+self.appearance.px(32), y+self.tab_height

    def content_width(self): return self.add_bbox()[2]+self.appearance.px(3)

    def insertion_index(self, root_x):
        x = self.canvas.canvasx(root_x-self.canvas.winfo_rootx())
        return max(0, min(len(self._items), int((x+self.tab_width/2)//self.tab_width)))

    def reorder(self, widget, index):
        key = str(widget)
        keys = list(self._items)
        old = keys.index(key)
        keys.remove(key)
        if old < index: index -= 1
        keys.insert(max(0,min(len(keys),index)), key)
        self._items = {k:self._items[k] for k in keys}
        self.draw()

    def mark_insertion(self, index):
        if self.drop_index != index:
            self.drop_index = index
            self.draw()

    def ensure_visible(self):
        if not self._current: return
        total=max(self.canvas.winfo_width(),self.content_width())
        x1,_,x2,_=self.tab_bbox(self._current)
        if self._current == self.tabs()[-1]: x2 = self.content_width()
        left=self.canvas.canvasx(0); width=self.canvas.winfo_width()
        if x1<left: self.canvas.xview_moveto(max(0,(x1-3)/total))
        elif x2>left+width: self.canvas.xview_moveto(max(0,(x2-width+3)/total))

    def draw(self, _event=None):
        if not self.winfo_exists(): return
        c, a = self.canvas, self.appearance
        c.delete('all')
        overflow = self.content_width() > self.header.winfo_width()-self.menu_button.winfo_reqwidth()
        for button in (self.back_button, self.forward_button):
            if overflow and not button.winfo_manager(): button.pack(side='left', before=self.menu_button)
            elif not overflow and button.winfo_manager(): button.pack_forget()
        total=max(c.winfo_width(),self.content_width())
        c.configure(scrollregion=(0,0,total,a.px(44)))
        for key,item in self._items.items():
            x1,y1,x2,y2=self.tab_bbox(key)
            selected=key==self._current
            fill=COLORS['selected'] if selected else (COLORS['hover'] if self.hover==key else COLORS['bg'])
            rounded(c,x1,y1,x2,y2,a.px(6),fill=fill,outline=COLORS['border'] if selected else fill,width=1)
            status=item['status']
            dot={'working':COLORS['warning'],'waiting':COLORS['warning'],'error':COLORS['error'],
                 'unread':COLORS['accent'],'offline':COLORS['muted']}.get(status,COLORS['green'])
            cx,cy=x1+a.px(14),(y1+y2)/2
            radius=a.px(3)
            c.create_oval(cx-radius,cy-radius,cx+radius,cy+radius,fill=dot,outline='')
            title=elide(item['text'],a.fonts['small'],self.tab_width-a.px(60))
            c.create_text(x1+a.px(27),cy,text=title,anchor='w',font=a.fonts['small'],
                          fill=COLORS['text'] if selected else COLORS['muted'])
            c.create_text(x2-a.px(14),cy,text='×',font=a.fonts['body'],
                          fill=COLORS['muted'] if item['closable'] else COLORS['border'])
        x1,y1,x2,y2 = self.add_bbox()
        if self.hover == '+': rounded(c,x1,y1,x2,y2,a.px(6),fill=COLORS['hover'],outline='')
        c.create_text((x1+x2)/2,(y1+y2)/2,text='+',font=a.fonts['body'],fill=COLORS['muted'])
        if self.drop_index is not None:
            x = max(2,self.drop_index*self.tab_width)
            c.create_line(x,a.px(5),x,a.px(39),fill=COLORS['accent'],width=a.px(2))

    def hit(self, x):
        try: return self.tabs()[self.index(f'@{x},0')]
        except (tk.TclError,IndexError): return None

    def clicked(self,event):
        self._press = None
        x=self.canvas.canvasx(event.x)
        x1,y1,x2,y2 = self.add_bbox()
        if x1 <= x <= x2 and y1 <= event.y <= y2:
            self.tooltip.hide(); self.create_tab(); return 'break'
        key=self.hit(event.x)
        if key is None: return
        if x>self.tab_bbox(key)[2]-self.appearance.px(26):
            if self._items[key]['closable']: self.close_tab(self._items[key]['widget'])
        else:
            self.select(key); self.activate(self._items[key]['widget'])
            self._press = (key,event.x_root,event.y_root)

    def dragged(self, event):
        if self._press is None: return
        key,x,y = self._press
        if not self._dragging:
            if max(abs(event.x_root-x),abs(event.y_root-y)) < self.appearance.px(6): return
            if key not in self._items: return
            self._dragging = True
            self.tooltip.hide()
            self.canvas.configure(cursor='fleur')
            self.canvas.focus_set()
            self.canvas.grab_set()
            self.drag.begin_tab_drag(self, self._items[key]['widget'])
        self.drag.update_tab_drag(event.x_root,event.y_root)
        return 'break'

    def released(self, event):
        if self._dragging: self.drag.end_tab_drag(event.x_root,event.y_root)
        self.reset_drag()

    def reset_drag(self):
        self._press, self._dragging = None, False
        if self.canvas.grab_current() is self.canvas: self.canvas.grab_release()
        self.canvas.configure(cursor='')

    def popup_menu(self, event):
        self._press = None
        self.tooltip.hide()
        try: self.menu.tk_popup(event.x_root,event.y_root)
        finally: self.menu.grab_release()
        return 'break'

    def hover_text(self):
        if self.hover == '+': return '新建终端会话'
        return self._items.get(self.hover,{}).get('tooltip','')

    def set_hover(self,key):
        if self.hover!=key:
            self.hover=key; self.tooltip.hide(); self.draw()

    def motion(self,event):
        if self._dragging: return
        key=self.hit(event.x)
        x1,y1,x2,y2 = self.add_bbox()
        if x1 <= self.canvas.canvasx(event.x) <= x2 and y1 <= event.y <= y2: key = '+'
        if key!=self.hover:
            self.set_hover(key)
            if key: self.tooltip.schedule()

    def wheel(self,event):
        self.canvas.xview_scroll(-1 if event.delta>0 else 1,'units'); return 'break'

    def step(self,direction):
        if self._current:
            keys=self.tabs(); key=keys[(keys.index(self._current)+direction)%len(keys)]
            self.select(key); self.activate(self._items[key]['widget'])
            self.canvas.focus_set()
        return 'break'

    def set_active(self, active):
        self.configure(highlightbackground=COLORS['focus'] if active else COLORS['border'])
