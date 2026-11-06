import json
import unittest
from pathlib import Path
from bridge import Translator, BridgeError
from replies import select_reply, ReplySelectionError

SCREEN = '''  >_ OpenAI Codex (v0.166.0)
     ~

› "你好"

• 你好！有什么可以帮你的吗？

  耗时 2 秒 • 12:51

› "现在几点了？"

• 我来查看一下当前时间。

• It’s 1:24 PM on November 5, 2026, in your timezone (China Standard Time, UTC+8).
  └ 提示：启用插件后，可使用 /plugins 浏览插件。

  耗时 6 秒 • 13:24

› 让 Codex 执行任何任务

  GPT-6-Astra 高 · ~ · 问候用户
  ← 查看智能体 · ? 查看快捷键'''
ANSWER = 'It’s 1:24 PM on November 5, 2026, in your timezone (China Standard Time, UTC+8).'


def selection(ps=10, pe=10, start=14, end=14):
    return dict(status='found', prompt_start=ps, prompt_end=pe,
                answer_spans=[dict(start_line=start, end_line=end)], reason='最新回答')


class ReplyTests(unittest.TestCase):
    def test_users_report_only_final_answer_is_copied(self):
        before = '\n'.join(SCREEN.splitlines()[:9])
        self.assertEqual(select_reply(SCREEN, selection(), before=before, expected='"现在几点了？"'), ANSWER)

    def test_history_and_same_snapshot_rejected(self):
        for response in [selection(ps=4, pe=4, start=6, end=6), selection()]:
            with self.assertRaises(ReplySelectionError):
                select_reply(SCREEN, response, before=SCREEN, expected='"现在几点了？"')

    def test_ui_chrome_and_invalid_ranges_rejected(self):
        for response in [selection(end=17), selection(start=0), selection(end=99), selection(start=4, end=4)]:
            with self.assertRaises(ReplySelectionError): select_reply(SCREEN, response)

    def test_repeated_question_must_be_new_occurrence(self):
        old = '› Repeat\n• Same answer\n'
        new = old + '› Repeat\n• Same answer\n'
        self.assertEqual(select_reply(new, selection(3, 3, 4, 4), before=old, expected='Repeat'), 'Same answer')
        with self.assertRaises(ReplySelectionError):
            select_reply(new, selection(1, 1, 2, 2), before=old, expected='Repeat')

    def test_multiline_answer_and_code_are_not_reduced_to_last_bullet(self):
        screen = '❯ Explain\n● First paragraph.\n\n- One\n- Two\n\n```python\n    print("x")\n```\n\nConclusion.'
        self.assertEqual(select_reply(screen, selection(1, 1, 2, 11), expected='Explain'),
                         screen.split('● ', 1)[1])

    def test_wrapped_user_prompt_anchors_turn(self):
        screen = '› Explain how\n  this works\n• Answer'
        self.assertEqual(select_reply(screen, selection(1, 2, 3, 3), before='', expected='Explain how this works'), 'Answer')

    def test_extraction_worker_uses_schema_and_returns_source_not_model_prose(self):
        calls = []
        def runner(argv, **kwargs):
            calls.append(argv)
            schema = json.loads(Path(argv[argv.index('--output-schema')+1]).read_text(encoding='utf-8'))
            self.assertFalse(schema['additionalProperties'])
            data = json.loads(kwargs['input'].split('DATA_JSON:\n', 1)[1])
            self.assertEqual(data['current_lines'][13]['text'], '• ' + ANSWER)
            Path(argv[argv.index('-o')+1]).write_text(json.dumps(selection()), encoding='utf-8')
        tr = Translator(binary='mock.exe', runner=runner, model='chosen-model', effort='high')
        self.assertEqual(tr.extract_reply(SCREEN, expected='"现在几点了？"'), ANSWER)
        self.assertEqual(calls[0][calls[0].index('--model')+1], 'chosen-model')
        self.assertEqual(calls[0][calls[0].index('--config')+1], 'model_reasoning_effort="high"')

    def test_uncertain_or_malformed_model_output_stops(self):
        for result in ['not json', 'null', '{"status":"ambiguous"}']:
            def runner(argv, **kwargs):
                Path(argv[argv.index('-o')+1]).write_text(result, encoding='utf-8')
            with self.assertRaises(BridgeError): Translator(binary='mock.exe', runner=runner).extract_reply(SCREEN)


if __name__ == '__main__': unittest.main()
