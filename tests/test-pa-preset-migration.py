#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('migration',ROOT/'update_manager/saturn-bridge/scripts/migrate-pa-preset.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

class Migration(unittest.TestCase):
    def test_reviewed_table_is_the_only_imported_state(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);source=root/'settings.json';table=[50.0]*12
            source.write_text(json.dumps({'radioPrefs':{'txPaCalibration':table,'tx_enabled':True,'txDrive':100,'pureSignalEnabled':True}}))
            self.assertIn('migrated',m.migrate(source,root/'state','|'.join(map(str,table))))
            text=(root/'state/pa-calibration-v1.txt').read_text()
            self.assertEqual(text,'format=saturn-pa-calibration\nversion=1\nprofile='+'|'.join(map(str,table))+'\n')
    def test_drafts_invalid_values_and_null_never_replace_a_host_preset(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);source=root/'settings.json'
            for value in [None,[50]*11,[True]*12,[39]*12,[float('inf')]*12,[49]*12]:
                source.write_text(json.dumps({'radioPrefs':{'txPaCalibration':value}}))
                with self.subTest(value=value):
                    with self.assertRaises(ValueError):m.migrate(source,root/'state','|'.join(['50']*12))
                    self.assertFalse((root/'state/pa-calibration-v1.txt').exists())
    def test_existing_host_file_and_symlink_are_never_overwritten(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);source=root/'settings.json';source.write_text('{}');target=root/'pa-calibration-v1.txt';target.write_text('existing')
            self.assertIn('retained',m.migrate(source,root,'invalid'))
            self.assertEqual(target.read_text(),'existing');target.unlink()
            other=root/'other';other.write_text('keep');target.symlink_to(other)
            self.assertIn('retained',m.migrate(source,root,'invalid'));self.assertEqual(other.read_text(),'keep')

if __name__=='__main__':unittest.main()
