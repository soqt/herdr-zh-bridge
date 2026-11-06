"""Read-only catalogs: Herdr's actual layout and Codex's model/list RPC."""
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import tempfile
import threading
import time
from bridge import BridgeError, codex_argv, run, unwrap, status
from platform_support import executable


def herdr_executable():
    return executable('herdr-app')


def reasoning_efforts(model):
    """None means unknown; an empty advertised list means no effort override."""
    if not isinstance(model, dict) or not isinstance(model.get('supportedReasoningEfforts'), list):
        return None
    choices = []
    for item in model['supportedReasoningEfforts']:
        effort = item.get('reasoningEffort') if isinstance(item, dict) else None
        if isinstance(effort, str) and effort and effort not in choices:
            choices.append(effort)
    return choices


def terminal_catalog(records, world):
    """Preserve source records, ordering, duplicate display names and full addresses."""
    indexed = {r.get('address', r.get('terminal', '')): r for r in records}
    result, seen = [], set()
    def add(address, groups, view=None, space=None, tab=None):
        if not address or address in seen:
            return
        seen.add(address)
        raw = indexed.get(address, {})
        view = view or {}
        s = status(raw) or view.get('status') or 'unknown'
        st = raw.get('status') if isinstance(raw.get('status'), dict) else {}
        fg = raw.get('foreground') or {}
        name = raw.get('name') or raw.get('title') or view.get('name') or address
        app = st.get('app') or raw.get('agent') or view.get('app') or fg.get('name') or '未上报'
        result.append(dict(address=address, groups=groups, name=name, app=app, status=s,
                           cwd=raw.get('cwd') or fg.get('cwd') or (space or {}).get('folder', ''),
                           raw={'terminal': raw, 'herdr_view': view, 'space': space, 'tab': tab}))
        children = list(view.get('sub_agents') or [])
        children += [a for a, r in indexed.items() if r.get('parent') == address and a not in children]
        for child in children:
            add(child, groups + ((f'parent:{address}', f'子 agent · {name}'),), space=space, tab=tab)
    spaces = world.get('spaces', []) if isinstance(world, dict) else []
    for space in spaces:
        machine = space.get('machine') or '本机'
        for tab in space.get('tabs', []):
            groups = ((f'machine:{machine}', machine),
                      (f'space:{space.get("id")}', space.get('name') or space.get('folder') or '未命名工作区'),
                      (f'tab:{tab.get("id")}', (tab.get('name') or '未命名标签页') + (' · 已收起' if tab.get('settled') else '')))
            for terminal in tab.get('terminals', []):
                add(terminal.get('terminal'), groups, terminal, space, tab)
    for address, raw in indexed.items():
        if not address or address in seen:
            continue
        machine = address.rsplit('/', 1)[0] if '/' in address else '本机'
        add(address, ((f'machine:{machine}', machine), ('unmapped', '未关联到当前 Herdr 布局')))
    return result


def load_terminals(core, app_binary):
    records = core.terminals()
    warning = ''
    try:
        world = unwrap(run([app_binary, 'ls']))
        if not isinstance(world, dict) or not isinstance(world.get('spaces'), list):
            raise BridgeError('Herdr 布局返回格式不符')
    except Exception as e:
        world = {}
        warning = '暂时无法读取 Herdr 布局，终端仍完整列出：' + str(e)
    return terminal_catalog(records, world), warning


def list_models(binary='codex', *, argv=None, timeout=35):
    """No credentials are read by this app. Codex uses its own login/config."""
    messages = queue.Queue()
    deadline = time.monotonic() + timeout
    with tempfile.TemporaryFile() as errors:
        process = subprocess.Popen(argv or [*codex_argv(binary), 'app-server'],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors,
                                   text=True, encoding='utf-8', errors='replace',
                                   creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        def reader():
            for line in process.stdout:
                try: messages.put(json.loads(line))
                except ValueError: continue
            messages.put(None)
        thread = threading.Thread(target=reader, daemon=True)
        thread.start()
        sequence = 0
        def write(data):
            process.stdin.write(json.dumps(data) + '\n')
            process.stdin.flush()
        def request(method, params):
            nonlocal sequence
            sequence += 1
            write({'id': sequence, 'method': method, 'params': params})
            while True:
                left = deadline - time.monotonic()
                if left <= 0: raise BridgeError('查询模型超时；可以手动输入模型 ID。')
                try: message = messages.get(timeout=left)
                except queue.Empty: raise BridgeError('查询模型超时；可以手动输入模型 ID。')
                if message is None: raise BridgeError('Codex 模型服务已退出，请检查 Codex 版本或登录状态。')
                if message.get('method') and 'id' in message:
                    write({'id': message['id'], 'error': {'code': -32601, 'message': 'Read-only model catalog client'}})
                    continue
                if message.get('id') != sequence: continue
                if 'error' in message: raise BridgeError(str(message['error']))
                return message.get('result', {})
        try:
            request('initialize', {'clientInfo': {'name': 'herdr_zh_bridge', 'title': 'Herdr Chinese translator', 'version': '0.5.0'}, 'capabilities': {}})
            write({'method': 'initialized', 'params': {}})
            models, cursor, seen = [], None, set()
            for _ in range(20):
                params = {'limit': 100, 'includeHidden': False}
                if cursor: params['cursor'] = cursor
                result = request('model/list', params)
                for model in result.get('data', []):
                    slug = model.get('model') or model.get('id')
                    if slug and slug not in seen:
                        models.append(model)
                        seen.add(slug)
                next_cursor = result.get('nextCursor')
                if not next_cursor: return models
                if next_cursor == cursor: raise BridgeError('模型分页游标未变化。')
                cursor = next_cursor
            raise BridgeError('模型目录页数过多，请手动输入模型 ID。')
        finally:
            if process.poll() is None:
                process.terminate()
                try: process.wait(timeout=3)
                except subprocess.TimeoutExpired: process.kill(); process.wait(timeout=3)
            thread.join(timeout=1)
            process.stdin.close()
            process.stdout.close()
