"""Build the portable source bundle with Unix executable bits for Mac launchers."""
from pathlib import Path
import zipfile
from install_portable import MODULES


def build(source, destination):
    source=Path(source)
    names=[*MODULES,'install_portable.py','install-macos.command','launch.command','launch.pyw',
           'install.ps1','update.ps1','README.md','LICENSE','package_release.py','preview.py',
           'test_bridge.py','test_catalog.py','test_panel.py','test_platform.py','test_replies.py',
           'mod/main.js','mod/mod.json']
    with zipfile.ZipFile(destination,'w',compression=zipfile.ZIP_DEFLATED) as archive:
        for name in names:
            info=zipfile.ZipInfo(name)
            info.create_system=3
            info.external_attr=(0o100755 if name.endswith('.command') else 0o100644)<<16
            info.compress_type=zipfile.ZIP_DEFLATED
            archive.writestr(info,(source/name).read_bytes())


if __name__=='__main__':
    source=Path(__file__).resolve().parent
    build(source,source/'herdr-translator-preview.zip')
    print('Packaged Windows/macOS source release')
