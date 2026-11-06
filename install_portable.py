"""Install the shared desktop application, preserving user preferences."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import shlex
import shutil
import subprocess
import sys
import uuid
from platform_support import application_home

MODULES = ('panel.py','conversation.py','bridge.py','replies.py','catalog.py','widgets.py',
           'tab_group.py','platform_support.py')


def install(source, *, platform=None, home=None, env=None, python=None):
    platform,home = platform or sys.platform, Path.home() if home is None else Path(home)
    source=Path(source)
    destination=application_home(platform=platform,home=home,env=env)/'app'
    python=python or sys.executable
    # Validate the complete payload before touching the previous installation.
    for name in MODULES: compile((source/name).read_text(encoding='utf-8'),name,'exec')
    bundle=home/'Applications/HerdrBridge.app'
    marker=bundle/'Contents/Resources/herdr-bridge.json'
    if platform=='darwin' and bundle.exists() and not marker.exists():
        raise RuntimeError('An unrelated HerdrBridge.app already exists; no files were changed.')
    destination.mkdir(parents=True,exist_ok=True)
    existing=[name for name in MODULES if (destination/name).is_file()]
    if existing:
        backup=destination.parent/'backups'/uuid.uuid4().hex
        backup.mkdir(parents=True)
        for name in existing: shutil.copy2(destination/name,backup/name)
    for name in MODULES:
        if (source/name).resolve() != (destination/name).resolve():
            shutil.copy2(source/name,destination/name)
    if platform=='darwin':
        executable=bundle/'Contents/MacOS/HerdrBridge'
        executable.parent.mkdir(parents=True,exist_ok=True)
        marker.parent.mkdir(parents=True,exist_ok=True)
        executable.write_text('#!/bin/sh\nexec '+shlex.quote(str(python))+' '+
                              shlex.quote(str(destination/'panel.py'))+' "$@"\n',encoding='utf-8',newline='\n')
        executable.chmod(0o755)
        metadata=dict(CFBundleName='Herdr Bridge',CFBundleDisplayName='中文桥',
                      CFBundleIdentifier='local.herdrbridge.desktop',CFBundleExecutable='HerdrBridge',
                      CFBundlePackageType='APPL',CFBundleShortVersionString='0.7',CFBundleVersion='7',
                      NSHighResolutionCapable=True)
        with (bundle/'Contents/Info.plist').open('wb') as file: plistlib.dump(metadata,file)
        marker.write_text(json.dumps({'app':'HerdrBridge','version':7}),encoding='utf-8')
        return bundle
    launcher=destination/'HerdrBridge.pyw'
    launcher.write_text("from pathlib import Path\nimport runpy\nrunpy.run_path(str(Path(__file__).with_name('panel.py')), run_name='__main__')\n",encoding='utf-8')
    return launcher


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--launch',action='store_true')
    args=parser.parse_args()
    import tkinter  # Fail before installing if this Python cannot run the UI.
    entry=install(Path(__file__).resolve().parent)
    print('Installed:',entry)
    if args.launch:
        if sys.platform=='darwin': subprocess.Popen(['open',str(entry)])
        else:
            pythonw=Path(sys.executable).with_name('pythonw.exe')
            subprocess.Popen([str(pythonw if pythonw.exists() else sys.executable),str(entry)],
                             creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
