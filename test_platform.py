import json
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch
from platform_support import application_home,command_modifier,executable,font_families
from install_portable import install,MODULES


class PlatformTests(unittest.TestCase):
    def test_platform_shortcuts_fonts_and_config_are_separate(self):
        self.assertEqual(command_modifier('darwin'),'Command')
        self.assertEqual(command_modifier('win32'),'Control')
        self.assertEqual(font_families('darwin')[1],'Menlo')
        self.assertEqual(application_home(platform='darwin',home='/Users/demo',env={}),Path('/Users/demo/Library/Application Support/HerdrBridge'))
        self.assertEqual(application_home(platform='win32',home='C:/Users/demo',env={'APPDATA':'C:/Roaming'}),Path('C:/Roaming/HerdrBridge'))

    def test_gui_path_fallback_and_explicit_paths(self):
        with tempfile.TemporaryDirectory() as d:
            directory=Path(d)/'.local/bin'; directory.mkdir(parents=True)
            binary=directory/'codex'; binary.write_text('test')
            with patch('platform_support.shutil.which',return_value=None),patch('platform_support.os.access',return_value=True):
                self.assertEqual(executable('codex',platform='darwin',home=d,env={}),str(binary))
                self.assertEqual(executable(str(binary),platform='darwin',env={}),str(binary))

    def test_mac_bundle_contains_shared_code_and_preserves_preferences_on_update(self):
        with tempfile.TemporaryDirectory() as d:
            home=Path(d)/'home with spaces'; home.mkdir()
            source=Path(__file__).resolve().parent
            bundle=install(source,platform='darwin',home=home,env={},python='/Python Path/python3')
            plist=plistlib.loads((bundle/'Contents/Info.plist').read_bytes())
            self.assertTrue(plist['NSHighResolutionCapable'])
            script=(bundle/'Contents/MacOS/HerdrBridge').read_text(encoding='utf-8')
            self.assertIn("'/Python Path/python3'",script)
            self.assertIn('"$@"',script)
            data=application_home(platform='darwin',home=home,env={})
            prefs=data/'preferences.json'; prefs.write_text('{"tabs":[]}',encoding='utf-8')
            for name in MODULES: self.assertEqual((source/name).read_bytes(),(data/'app'/name).read_bytes())
            install(source,platform='darwin',home=home,env={},python='/Python Path/python3')
            self.assertEqual(prefs.read_text(encoding='utf-8'),'{"tabs":[]}')
            self.assertEqual(len(list((data/'backups').iterdir())),1)

    def test_unrelated_mac_application_is_not_overwritten(self):
        with tempfile.TemporaryDirectory() as d:
            bundle=Path(d)/'Applications/HerdrBridge.app'; bundle.mkdir(parents=True)
            marker=bundle/'keep.txt'; marker.write_text('untouched')
            with self.assertRaises(RuntimeError):
                install(Path(__file__).resolve().parent,platform='darwin',home=d,env={})
            self.assertEqual(marker.read_text(),'untouched')


if __name__=='__main__': unittest.main()
