"""OS boundaries for the otherwise shared UI and translation engine."""
import os
from pathlib import Path
import shutil
import sys


def command_modifier(platform=None):
    return 'Command' if (platform or sys.platform) == 'darwin' else 'Control'


def application_home(*, platform=None, env=None, home=None):
    platform, env = platform or sys.platform, os.environ if env is None else env
    home = Path.home() if home is None else Path(home)
    if env.get('HERDR_BRIDGE_HOME'): return Path(env['HERDR_BRIDGE_HOME']).expanduser()
    if platform == 'win32': return Path(env.get('APPDATA', home/'AppData/Roaming'))/'HerdrBridge'
    if platform == 'darwin': return home/'Library/Application Support/HerdrBridge'
    return Path(env.get('XDG_CONFIG_HOME', home/'.config'))/'herdr-bridge'


def search_directories(*, platform=None, env=None, home=None):
    platform, env = platform or sys.platform, os.environ if env is None else env
    home = Path.home() if home is None else Path(home)
    if platform == 'win32':
        return [Path(env.get('LOCALAPPDATA', home/'AppData/Local'))/'Programs/herdr-app',
                Path(env.get('APPDATA', home/'AppData/Roaming'))/'npm', home/'.local/bin', home/'.cargo/bin']
    return [home/'.local/bin', home/'.cargo/bin', Path('/opt/homebrew/bin'), Path('/usr/local/bin')]


def executable(name, *, platform=None, env=None, home=None):
    platform, env = platform or sys.platform, os.environ if env is None else env
    explicit = Path(name).expanduser()
    if explicit.is_absolute() or '/' in name or '\\' in name: return str(explicit)
    found = shutil.which(name, path=env.get('PATH', ''))
    if found: return found
    suffixes = ('', '.exe', '.cmd') if platform == 'win32' else ('',)
    for directory in search_directories(platform=platform, env=env, home=home):
        for suffix in suffixes:
            candidate = directory/(name+suffix)
            if candidate.is_file() and (platform == 'win32' or os.access(candidate, os.X_OK)):
                return str(candidate)
    return name


def font_families(platform=None):
    platform = platform or sys.platform
    if platform == 'darwin': return ('Helvetica Neue', 'Menlo')
    if platform == 'win32': return ('Microsoft YaHei UI', 'Cascadia Mono')
    return ('DejaVu Sans', 'DejaVu Sans Mono')
