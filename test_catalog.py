import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock
from catalog import terminal_catalog, list_models, reasoning_efforts
from bridge import Translator


WORLD = {'spaces': [
    {'id': 's1', 'name': 'DemoProject', 'machine': None, 'folder': 'C:/work/DemoProject', 'tabs': [
        {'id': 'tab1', 'name': '开发', 'terminals': [{'terminal': 't1', 'sub_agents': ['t2']}]},
        {'id': 'tab2', 'name': '开发', 'terminals': [{'terminal': 't3'}]}]},
    {'id': 'remote/s1', 'name': 'DemoProject', 'machine': 'remote', 'folder': '/work/DemoProject', 'tabs': [
        {'id': 'remote/tab1', 'name': '开发', 'terminals': [{'terminal': 'remote/t1'}]}]}
]}
RECORDS = [
    {'address': 't1', 'name': 'Codex', 'cwd': 'C:/work/DemoProject', 'status': {'state': 'idle', 'app': 'codex'}, 'extra': {'keep': 'all'}},
    {'address': 't2', 'name': 'Reviewer', 'parent': 't1', 'status': 'working'},
    {'address': 't3', 'name': 'Codex', 'status': 'idle'},
    {'address': 'remote/t1', 'name': 'Codex', 'cwd': '/work/DemoProject', 'status': 'done'},
    {'address': 't9', 'name': 'Other', 'status': None},
]


class CatalogTests(unittest.TestCase):
    def test_actual_layout_order_and_children(self):
        rows = terminal_catalog(RECORDS, WORLD)
        self.assertEqual([r['address'] for r in rows], ['t1', 't2', 't3', 'remote/t1', 't9'])
        self.assertEqual(rows[1]['groups'][-1][0], 'parent:t1')
        self.assertNotEqual(rows[0]['groups'], rows[2]['groups'])
        self.assertEqual(rows[3]['groups'][0][1], 'remote')
        self.assertEqual(rows[0]['raw']['terminal']['extra']['keep'], 'all')
        self.assertIn('未关联', rows[-1]['groups'][-1][1])

    def test_cycles_do_not_duplicate_or_loop(self):
        records = [{'address': 't1', 'parent': 't2'}, {'address': 't2', 'parent': 't1'}]
        self.assertEqual(len(terminal_catalog(records, WORLD)), 4)  # includes layout-only t3 and remote/t1

    def test_model_selection_reaches_translation_command(self):
        calls = []
        def runner(argv, **kwargs):
            calls.append(argv)
            Path(argv[argv.index('-o') + 1]).write_text('translated', encoding='utf-8')
        worker = Translator(binary='fake.exe', runner=runner, model='chosen-model')
        worker.translate('你好', 'English'); worker.translate('Hello', 'Simplified Chinese')
        self.assertTrue(all(argv[argv.index('--model') + 1] == 'chosen-model' for argv in calls))

    def test_effort_catalog_preserves_supported_levels_without_guessing(self):
        self.assertIsNone(reasoning_efforts(None))
        self.assertIsNone(reasoning_efforts({}))
        self.assertEqual(reasoning_efforts({'supportedReasoningEfforts': []}), [])
        self.assertEqual(reasoning_efforts({'supportedReasoningEfforts': [
            {'reasoningEffort': 'high'}, {'reasoningEffort': 'max'}, {'reasoningEffort': 'high'},
            {'reasoningEffort': 'future-level'}]}), ['high', 'max', 'future-level'])

    def test_effort_reaches_both_translation_directions_and_default_is_omitted(self):
        calls = []
        def runner(argv, **kwargs):
            calls.append(argv)
            Path(argv[argv.index('-o')+1]).write_text('translated', encoding='utf-8')
        worker = Translator(binary='fake.exe', runner=runner, effort='max')
        worker.translate('你好', 'English'); worker.translate('Hello', 'Simplified Chinese')
        self.assertTrue(all(argv[argv.index('--config')+1] == 'model_reasoning_effort="max"' for argv in calls))
        worker.effort = ''
        worker.translate('你好', 'English')
        self.assertNotIn('--config', calls[-1])

    def test_model_rpc_handshake_and_pagination(self):
        server = '''import sys,json
initialized=False
for line in sys.stdin:
 r=json.loads(line)
 if r['method']=='initialize':
  print(json.dumps({'id':r['id'],'result':{}}),flush=True)
 elif r['method']=='initialized': initialized=True
 elif r['method']=='model/list':
  assert initialized
  cursor=r['params'].get('cursor')
  result={'data':[{'id':'m2','model':'m2','displayName':'Model 2'}], 'nextCursor':None} if cursor else {'data':[{'id':'m1','model':'m1','displayName':'Model 1'}], 'nextCursor':'page2'}
  print(json.dumps({'id':r['id'],'result':result}),flush=True)
'''
        with tempfile.TemporaryDirectory() as folder:
            script = Path(folder) / 'server.py'; script.write_text(server)
            models = list_models(argv=[sys.executable, '-u', str(script)], timeout=5)
        self.assertEqual([m['model'] for m in models], ['m1', 'm2'])


if __name__ == '__main__': unittest.main()
