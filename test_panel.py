import json
from pathlib import Path
import tempfile
import threading
import time
import tkinter as tk
import unittest
from unittest.mock import Mock, patch
from bridge import BridgeError
from panel import Panel
from conversation import DEFAULT_EFFORT, ConversationPane
from widgets import TerminalPicker
from catalog import terminal_catalog
from test_catalog import RECORDS, WORLD


class PanelTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.root = tk.Tk(); self.root.withdraw()
        self.app = Panel(self.root, 't1', preferences_path=Path(self.folder.name)/'prefs.json')
        self.pane = self.app.active

    def tearDown(self):
        for timer in self.root.tk.call('after', 'info'): self.root.after_cancel(timer)
        self.root.destroy(); self.folder.cleanup()

    def pump(self, predicate, timeout=3):
        deadline = time.monotonic()+timeout
        while time.monotonic() < deadline:
            self.root.update()
            if predicate(): return
            time.sleep(0.005)
        self.fail('UI task did not finish')

    def drag_tab(self, tab, destination, *, release=True):
        group = self.app.notebooks[self.app.groups[tab]]
        group.select(tab.frame); self.root.update()
        x1,y1,x2,y2 = group.tab_bbox(tab.frame)
        x,y = round(x1-group.canvas.canvasx(0)+30),round((y1+y2)/2)
        group.canvas.event_generate('<ButtonPress-1>',x=x,y=y)
        self.root.update()
        rx,ry = destination
        x,y = round(rx-group.canvas.winfo_rootx()),round(ry-group.canvas.winfo_rooty())
        group.canvas.event_generate('<B1-Motion>',x=x,y=y,rootx=rx,rooty=ry)
        self.root.update()
        if release:
            group.canvas.event_generate('<ButtonRelease-1>',x=x,y=y,rootx=rx,rooty=ry)
            self.root.update()
        return group

    def right_drop(self):
        body = self.app.notebooks[0].body
        return body.winfo_rootx()+body.winfo_width()*4//5,body.winfo_rooty()+body.winfo_height()//2

    def strip_start(self, index=0):
        canvas = self.app.notebooks[index].canvas
        canvas.xview_moveto(0)
        return canvas.winfo_rootx()+8,canvas.winfo_rooty()+12

    def test_input_cleared_after_confirmed_send(self):
        p = self.pane
        p.task = lambda fn: fn(); p.session.start = Mock()
        p.input.insert('1.0', '  你好\n'); p.send()
        self.assertEqual(p.input.get('1.0', 'end-1c'), '  你好\n')
        p.drain()
        self.assertEqual(p.input.get('1.0', 'end-1c'), '')
        self.assertIn('你好', p.chat.get('1.0', 'end'))
        p.session.start.assert_called_once_with('t1', '你好', p.emit)

    def test_tab_and_window_geometry_stay_fixed_across_selection_and_status(self):
        a=self.pane; b=self.app.new_tab('t2')
        b.target_name='这是一个非常长的终端名称 / project / feature / full address'
        self.app.refresh_tab(b)
        self.root.deiconify(); self.root.update()
        group=self.app.notebooks[0]
        expected_a=group.tab_bbox(a.frame); expected_b=group.tab_bbox(b.frame)
        self.assertEqual(expected_a[2]-expected_a[0],expected_b[2]-expected_b[0])
        window=(self.root.winfo_width(),self.root.winfo_height())
        content=(group.body.winfo_width(),group.body.winfo_height())
        for tab,status in [(a,'waiting'),(b,'working'),(a,'error'),(b,'idle')]:
            tab.busy=status=='working'; tab.auto_paused=status=='error'
            tab.session.pending={'target':tab.target.get(),'sent':True} if status=='waiting' else None
            self.app.refresh_tab(tab); self.app.focus_tab(tab); self.root.update()
            self.assertEqual(group.tab_bbox(a.frame),expected_a)
            self.assertEqual(group.tab_bbox(b.frame),expected_b)
            self.assertEqual((self.root.winfo_width(),self.root.winfo_height()),window)
            self.assertEqual((group.body.winfo_width(),group.body.winfo_height()),content)
            self.assertEqual((tab.frame.winfo_width(),tab.frame.winfo_height()),content)
        self.assertIn('full address',group.tab(b.frame)['tooltip'])

    def test_tab_overflow_scrolls_without_resizing_tabs(self):
        for i in range(2,13): self.app.new_tab('t'+str(i))
        self.root.deiconify(); self.root.update()
        group=self.app.notebooks[0]
        self.app.focus_tab(self.app.tabs[-1]); self.root.update()
        self.assertGreater(group.canvas.xview()[0],0)
        last=group.tab_bbox(self.app.tabs[-1].frame)
        self.app.focus_tab(self.app.tabs[0]); self.root.update()
        self.assertLess(group.canvas.xview()[0],0.02)
        first=group.tab_bbox(self.app.tabs[0].frame)
        self.assertEqual(first[2]-first[0],last[2]-last[0])

    def test_failure_preserves_input_and_other_tabs_remain_usable(self):
        p = self.pane; other = self.app.new_tab('t2')
        p.session.start = Mock(side_effect=BridgeError('发送结果未知'))
        p.input.insert('1.0', '保留原文'); p.send()
        self.pump(lambda: not p.busy)
        self.assertEqual(p.input.get('1.0', 'end-1c'), '保留原文')
        self.assertTrue(p.auto_paused)
        self.assertEqual(str(other.send_button['state']), 'normal')
        self.assertNotIn('发送结果未知', other.chat.get('1.0', 'end'))

    def test_edits_during_send_preserved_even_when_text_matches(self):
        p = self.pane; p.task = lambda fn: fn(); p.session.start = Mock()
        for next_message in ['下一条', '原文']:
            p.input.delete('1.0', 'end'); p.input.insert('1.0', '原文'); p.send()
            p.input.delete('1.0', 'end'); p.input.insert('1.0', next_message); p.drain()
            self.assertEqual(p.input.get('1.0', 'end-1c'), next_message)

    def test_effort_catalog_selection_and_independence(self):
        p = self.pane
        p.model_choices['Old label [m1]'] = 'm1'
        p.model.set('Old label [m1]'); p.effort.set('max')
        other = self.app.new_tab('t2'); other.model.set('m2'); other.effort.set('low')
        records = [{'model':'m1','displayName':'New label','supportedReasoningEfforts':[
            {'reasoningEffort':'high'},{'reasoningEffort':'max'}]},
            {'model':'m2','displayName':'Small','supportedReasoningEfforts':[{'reasoningEffort':'low'}]}]
        self.app.apply_models(records, source=p); p.configure(); other.configure()
        self.assertEqual(p.session.translator.model, 'm1')
        self.assertEqual(p.session.translator.effort, 'max')
        self.assertEqual(other.session.translator.effort, 'low')
        self.assertEqual(p.effort_combo['values'], (DEFAULT_EFFORT, 'high', 'max'))
        p.model.set('Small'); p.configure()
        self.assertEqual(p.effort.get(), DEFAULT_EFFORT)
        self.assertEqual(p.session.translator.effort, '')
        p.session.pending = {'target':'t1','sent':True}; p.refresh_controls()
        self.assertEqual(str(p.effort_combo['state']), 'disabled')
        self.assertEqual(str(other.effort_combo['state']), 'readonly')

    def test_logs_inline_collapsed_and_no_success_noise(self):
        p = self.pane
        p.add_chat('你', '你好'); p.begin_log()
        first = p.current_log
        p.emit('英文译文：\nHello'); p.emit('中文译文：\n你好')
        p.emit('本轮已完成，可以发送下一条。'); p.drain()
        self.assertFalse(first.expanded)
        self.assertIn('Hello', first.body.get('1.0', 'end'))
        self.assertNotIn('Hello', p.chat.get('1.0', 'end'))
        self.assertNotIn('本轮已完成', first.body.get('1.0', 'end'))
        first.toggle(); self.assertTrue(first.expanded)
        p.begin_log(); p.emit('第二轮日志'); p.drain()
        self.assertNotIn('第二轮日志', first.body.get('1.0', 'end'))
        self.assertFalse(p.current_log.expanded)
        first.toggle(); self.assertFalse(first.expanded)

    def test_split_move_merge_preserve_widgets_drafts_and_running_session(self):
        p = self.pane; other = self.app.new_tab('t2')
        p.input.insert('1.0', '草稿 A'); p.add_chat('', '历史 A')
        p.session.pending = {'target':'t1','sent':True}
        original_frame, original_session = p.frame, p.session
        self.root.deiconify(); self.root.update()
        window = self.root.winfo_width(),self.root.winfo_height()
        self.drag_tab(p,self.right_drop())
        self.assertEqual(len(self.app.panes.panes()), 2)
        self.assertEqual(self.app.groups[p],1)
        self.assertIs(p.frame, original_frame); self.assertIs(p.session, original_session)
        self.drag_tab(p,self.strip_start())
        self.assertEqual(len(self.app.panes.panes()), 1)
        self.assertFalse(self.app.split)
        self.assertEqual((self.root.winfo_width(),self.root.winfo_height()),window)
        self.assertEqual(p.input.get('1.0', 'end-1c'), '草稿 A')
        self.assertIn('历史 A', p.chat.get('1.0', 'end'))
        self.assertEqual(p.session.pending['target'], 't1')
        self.assertEqual(other.target.get(), 't2')
        self.assertLess(p.selector_area.winfo_rooty(), p.input.winfo_rooty())
        self.assertLessEqual(p.send_button.winfo_rooty()+p.send_button.winfo_height(),
                             self.root.winfo_rooty()+self.root.winfo_height())

    def test_drag_reorders_tabs_and_persists_order(self):
        second = self.app.new_tab('t2'); third = self.app.new_tab('t3')
        self.root.deiconify(); self.root.update()
        self.drag_tab(third,self.strip_start())
        self.assertEqual([t.target.get() for t in self.app.tabs],['t3','t1','t2'])
        self.assertFalse(self.app.split)
        saved = json.loads(self.app.preferences_path.read_text(encoding='utf-8'))
        self.assertEqual([t['target'] for t in saved['tabs']],['t3','t1','t2'])
        self.assertIs(self.app.active,third)

    def test_drag_preview_escape_and_outside_drop_do_not_move_tabs(self):
        second = self.app.new_tab('t2')
        self.root.deiconify(); self.root.update()
        source = self.drag_tab(second,self.right_drop(),release=False)
        self.assertTrue(self.app.drop_hint.winfo_ismapped())
        self.assertFalse(self.app.split)
        source.canvas.focus_force()
        source.canvas.event_generate('<Escape>'); self.root.update()
        self.assertIsNone(self.app._drag_tab)
        self.assertIsNone(source.canvas.grab_current())
        self.assertFalse(self.app.drop_hint.winfo_ismapped())
        source.canvas.event_generate('<ButtonRelease-1>',x=0,y=0)
        self.drag_tab(second,(self.root.winfo_rootx()-30,self.root.winfo_rooty()-30))
        self.assertFalse(self.app.split)
        self.assertEqual(self.app.groups[second],0)

    def test_last_tab_in_left_pane_can_merge_into_right_strip(self):
        second = self.app.new_tab('t2')
        self.root.deiconify(); self.root.update()
        self.drag_tab(second,self.right_drop())
        self.drag_tab(self.pane,self.strip_start(1))
        self.assertFalse(self.app.split)
        self.assertEqual(len(self.app.panes.panes()),1)
        self.assertEqual(self.app.notebooks[0].tabs(),(str(self.pane.frame),str(second.frame)))
        self.assertTrue(self.app.is_visible(self.pane))

    def test_closing_only_tab_in_either_split_collapses_empty_pane(self):
        for close_left in (False,True):
            second = self.app.new_tab('another')
            self.root.deiconify(); self.root.update()
            self.drag_tab(second,self.right_drop())
            survivor = second if close_left else self.app.tabs[0]
            closing = self.app.tabs[0] if close_left else second
            self.app.focus_tab(closing); self.app.close_active(); self.root.update()
            self.assertFalse(self.app.split)
            self.assertTrue(self.app.is_visible(survivor))

    def test_inline_plus_opens_in_its_own_group_and_menu_keeps_settings(self):
        second = self.app.new_tab('t2')
        self.root.deiconify(); self.root.update()
        self.drag_tab(second,self.right_drop())
        group = self.app.notebooks[0]
        x1,y1,x2,y2 = group.add_bbox()
        self.assertGreater(x1,group.tab_bbox(self.pane.frame)[2])
        with patch.object(ConversationPane,'open_targets') as picker:
            group.canvas.event_generate('<ButtonPress-1>',
                x=round((x1+x2)/2-group.canvas.canvasx(0)),y=round((y1+y2)/2))
            self.root.update()
            picker.assert_called_once()
        self.assertEqual(self.app.groups[self.app.active],0)
        self.assertEqual(len(self.app.tabs),3)
        self.app.update_tabs_menu(group.menu,0)
        self.assertEqual(group.menu.entrycget(1,'label'),'设置')
        self.assertLess(group.header.winfo_rooty()-self.root.winfo_rooty(),self.app.appearance.px(15))

    def test_refresh_icon_invokes_current_model_refresh_and_locks_during_work(self):
        p = self.pane; p.task = Mock(); p.configure = Mock()
        self.root.deiconify(); self.root.update()
        p.refresh_button.invoke(); p.task.assert_called_once()
        self.assertLess(p.effort_combo.winfo_x()+p.effort_combo.winfo_width(),p.refresh_button.winfo_x())
        self.assertLess(p.refresh_button.winfo_x()+p.refresh_button.winfo_width(),p.more_button.winfo_x())
        p.busy = True; p.refresh_controls(); p.refresh_button.invoke()
        p.task.assert_called_once()
        self.assertEqual(str(p.refresh_button['state']),'disabled')

    def test_drag_overflow_scrolls_to_hidden_tabs(self):
        for i in range(2,13): self.app.new_tab('t'+str(i))
        self.root.deiconify(); self.root.update()
        group = self.app.notebooks[0]
        self.app.focus_tab(self.pane); self.root.update()
        right = group.canvas.winfo_rootx()+group.canvas.winfo_width()-5
        top = group.canvas.winfo_rooty()+15
        self.drag_tab(self.pane,(right,top),release=False)
        self.pump(lambda:group.canvas.xview()[0]>0)
        self.assertIsNotNone(group.drop_index)
        group.canvas.event_generate('<ButtonRelease-1>',x=group.canvas.winfo_width()-5,y=15,rootx=right,rooty=top)
        self.root.update()
        self.assertIsNone(self.app._drag_timer)
        self.assertEqual(len(self.app.tabs),12)
        self.assertGreater(self.app.tabs.index(self.pane),0)

    def test_narrow_split_controls_fit_at_large_text_scale(self):
        second = self.app.new_tab('t2')
        self.root.deiconify(); self.root.update()
        self.drag_tab(second,self.right_drop())
        for zoom in (1,1.25,1.5):
            self.app.appearance.set_zoom(zoom); self.root.update()
            self.app.panes.sash_place(0,0,0); self.root.update()
            for pane in self.app.tabs:
                controls = (pane.model_combo,pane.effort_combo,pane.refresh_button,pane.more_button)
                for left,right in zip(controls,controls[1:]):
                    self.assertLess(left.winfo_x()+left.winfo_width(),right.winfo_x())
                self.assertEqual(len({w.winfo_height() for w in controls}),1)
                self.assertEqual(pane.more_button.winfo_x()+pane.more_button.winfo_width(),pane.selector_area.winfo_width())

    def test_duplicate_terminal_focuses_existing_and_rebinding_creates_new_tab(self):
        p = self.pane; other = self.app.new_tab('t2')
        self.assertIs(self.app.new_tab('t1'), p)
        self.assertEqual(len(self.app.tabs), 2)
        other.select_target({'address':'t1','name':'One'})
        self.assertIs(self.app.active, p); self.assertEqual(other.target.get(), 't2')
        p.select_target({'address':'t3','name':'Three'})
        self.assertEqual(len(self.app.tabs), 3)
        self.assertEqual(self.app.active.target.get(), 't3')
        self.assertEqual(p.target.get(), 't1')

    def test_missing_terminal_keeps_binding_and_disables_only_its_send(self):
        p = self.pane; other = self.app.new_tab('t2')
        self.app.apply_targets([{'address':'t2','name':'Two'}])
        self.assertEqual(p.target.get(), 't1'); self.assertTrue(p.offline)
        self.assertEqual(str(p.send_button['state']), 'disabled')
        self.assertEqual(str(other.send_button['state']), 'normal')

    def test_running_tab_cannot_close_or_send_twice(self):
        p = self.pane
        p.session.pending = {'target':'t1','sent':True}; p.refresh_controls()
        p.session.start = Mock(); p.send(); p.session.start.assert_not_called()
        self.assertFalse(self.app.close_active())
        p.abandon(); self.assertTrue(self.app.close_active())
        self.assertTrue(p.disposed); self.assertEqual(len(self.app.tabs), 1)

    def test_preference_migration_and_restore_do_not_resume_or_save_chat(self):
        path = self.app.preferences_path
        path.write_text(json.dumps({'target':'legacy','model':'m1','effort':'max'}), encoding='utf-8')
        # Recreate on another Toplevel to keep tests isolated from the current workspace.
        r = tk.Toplevel(self.root)
        migrated = Panel(r, preferences_path=path)
        self.assertEqual(migrated.active.target.get(), 'legacy')
        self.assertEqual(migrated.active.effort.get(), 'max')
        migrated.active.input.insert('1.0', 'private draft')
        migrated.active.add_chat('', 'private answer')
        second = migrated.new_tab('second'); migrated.show_split(); migrated.move_tab(second,1); migrated.save_preferences()
        saved = json.loads(path.read_text(encoding='utf-8'))
        self.assertEqual(len(saved['tabs']), 2)
        self.assertNotIn('private', path.read_text(encoding='utf-8'))
        r.destroy()
        r = tk.Toplevel(self.root)
        restored = Panel(r, preferences_path=path)
        self.assertTrue(restored.split)
        self.assertEqual([t.target.get() for t in restored.tabs], ['legacy','second'])
        self.assertTrue(all(t.session.pending is None for t in restored.tabs))
        r.destroy()

    def test_tree_picker_retains_hierarchy_full_details_and_leaf_selection(self):
        rows = terminal_catalog(RECORDS, WORLD); choose = Mock()
        picker = TerminalPicker(self.root, self.pane.target_button, self.app.appearance, rows, '', choose)
        self.assertEqual(len(picker.by_item), 5)
        picker.tree.selection_set(picker.tree.get_children()[0]); picker.on_select(); picker.accept()
        choose.assert_not_called()
        item = next(k for k,r in picker.by_item.items() if r['address']=='t1')
        picker.tree.selection_set(item); picker.on_select()
        self.assertIn('"keep": "all"', picker.details.get('1.0','end'))
        picker.query.set('remote/t1'); self.assertEqual(len(picker.by_item), 1)
        picker.tree.selection_set(next(iter(picker.by_item))); picker.accept()
        self.assertEqual(choose.call_args.args[0]['address'], 'remote/t1')

    def test_concurrent_roundtrips_stay_with_origin_tab_when_focus_changes(self):
        barrier = threading.Barrier(3)
        sent = []
        class FakeCore:
            def __init__(self, address): self.address, self.sent, self.done = address, False, False
            def get(self, address):
                assert address == self.address
                return dict(pid=10, foreground={'pid':11,'name':'codex'}, status={'state':'idle'}, turn=2 if self.done else 1)
            def read(self, address): return 'Answer '+self.address if self.sent else 'before'
            def send(self, address, text):
                assert address == self.address
                sent.append((address,text)); self.sent=True
        class FakeTranslator:
            def __init__(self, address): self.address=address
            def translate(self, text, language):
                if language == 'English': barrier.wait(timeout=3); return 'English '+self.address
                assert text == 'Answer '+self.address
                return '中文 '+self.address
            def extract_reply(self, screen, **kwargs):
                assert kwargs['expected'] == 'English '+self.address
                return screen
        a = self.pane; b = self.app.new_tab('t2')
        for p in (a,b):
            p.session.core = FakeCore(p.target.get())
            p.session.translator = FakeTranslator(p.target.get())
            p.input.insert('1.0','问题 '+p.target.get()); p.send()
        self.assertTrue(a.busy and b.busy)
        a.input.delete('1.0','end'); a.input.insert('1.0','下一条 A')
        barrier.wait(timeout=3)
        self.pump(lambda: not a.busy and not b.busy)
        self.assertCountEqual(sent,[('t1','English t1'),('t2','English t2')])
        self.assertEqual(a.input.get('1.0','end-1c'),'下一条 A')
        self.assertEqual(b.input.get('1.0','end-1c'),'')
        b.session.core.done=True; b.tick()
        self.pump(lambda: not b.busy and b.session.pending is None)
        self.assertIsNotNone(a.session.pending)
        self.app.focus_tab(b)
        a.session.core.done=True; a.tick()
        self.pump(lambda: not a.busy and a.session.pending is None)
        self.assertIn('中文 t1',a.chat.get('1.0','end')); self.assertNotIn('中文 t2',a.chat.get('1.0','end'))
        self.assertIn('中文 t2',b.chat.get('1.0','end')); self.assertNotIn('中文 t1',b.chat.get('1.0','end'))
        self.assertTrue(a.unread); self.assertFalse(b.unread)


if __name__ == '__main__': unittest.main()
