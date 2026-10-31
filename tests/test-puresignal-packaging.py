#!/usr/bin/env python3
import importlib.util
import tempfile
import unittest
import subprocess
import os
import json
import shutil
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
def module(path, name):
    spec=importlib.util.spec_from_file_location(name, ROOT/path); m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
profile=module('update_manager/scripts/saturn-puresignal-profile.py','profile')
pin=module('update_manager/saturn-bridge/scripts/source-pin.py','pin')
class Packaging(unittest.TestCase):
    def environment(self):
        d={k:'1' for k in profile.REQUIRED};d.update(SATURN_BRIDGE_TX_OWNER='shared',SATURN_PS_MAX_WATTS='100',SATURN_PS_MEASUREMENT_ONLY='0',SATURN_PS_GENERATED_TRIALS_ONLY='0',SATURN_BRIDGE_RADIO_BACKEND='xdma',SATURN_REMOTE_TX_RF_ENABLED='1',SATURN_REMOTE_BASIC_AUTH='must-not-be-exported');return d
    def test_daily_configuration_keeps_power_permission_and_dynamic_backend(self):
        f=profile.compose(self.environment(),'/var/lib/saturn-bridge/puresignal');env=f['puresignal.env'];self.assertIn('SATURN_PS_MAX_WATTS="100"',env);self.assertIn('SATURN_REMOTE_TX_RF_ENABLED="1"',env);self.assertNotIn('RADIO_BACKEND',env);self.assertNotIn('must-not-be-exported',str(f));self.assertNotIn('/run/',str(f));self.assertIn('/etc/default/saturn-puresignal',f['saturn-bridge.conf']);self.assertIn('SATURN_P3_RT_AUDIO_PRIORITY=10',f['p2app.conf'])
    def test_lifecycle_contract_cannot_be_omitted(self):
        for key in profile.REQUIRED:
            d=self.environment();del d[key]
            with self.subTest(key=key),self.assertRaises(ValueError):profile.compose(d,'/var/lib/ps')
    def test_injection_and_volatile_paths_rejected(self):
        for val in ['1\nExecStart=/bad','"bad','x%h','x\\n','/run/temporary']:
            d=self.environment();d['SATURN_PS_MEASUREMENT_DIR']=val
            # Canonical persistent path replaces the old capture path; other
            # settings must still reject arbitrary control/path injection.
            d['SATURN_PS_FEEDBACK_INPUT']=val
            with self.subTest(value=val),self.assertRaises(ValueError):profile.compose(d,'/var/lib/ps')
        with self.assertRaises(ValueError):profile.compose(self.environment(),'/run/ps')
    def test_measurement_only_is_not_promoted(self):
        for key in ['SATURN_PS_MEASUREMENT_ONLY','SATURN_PS_GENERATED_TRIALS_ONLY']:
            d=self.environment();d[key]='1'
            with self.assertRaises(ValueError):profile.compose(d,'/var/lib/ps')
    def test_clean_source_pin_changes_only_with_tracked_source(self):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);subprocess.run(['git','init','-q',name],check=True);p=root/'update_manager/saturn-bridge/src/main.rs';p.parent.mkdir(parents=True);p.write_text('source');subprocess.run(['git','-C',name,'add','.'],check=True);one=pin.source_pin(root);(p.parent/'scratch').write_text('ignored output');self.assertEqual(pin.source_pin(root),one);p.write_text('changed');self.assertNotEqual(pin.source_pin(root),one)
class BuildProfile(unittest.TestCase):
    def run_fixture(self, content, mode='644', override=None):
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);source=root/'source';source.mkdir();subprocess.run(['git','init','-q',str(source)],check=True)
            subprocess.run(['git','-C',str(source),'config','user.name','Fixture'],check=True);subprocess.run(['git','-C',str(source),'config','user.email','test@example.invalid'],check=True)
            (source/'input').write_text('fixed');subprocess.run(['git','-C',str(source),'add','.'],check=True);subprocess.run(['git','-C',str(source),'commit','-qm','fixture'],check=True);subprocess.run(['git','-C',str(source),'branch','-M','main'],check=True)
            config=root/'profile';config.write_text(content);bin=root/'bin';bin.mkdir();record=root/'environment.json';sentinel=root/'must-not-exist'
            stat=shutil.which('stat');git=shutil.which('git');python=shutil.which('python3')
            (bin/'stat').write_text('#!/bin/sh\nif [ "$3" = '+str(config)+' ]; then if [ "$2" = "%u" ]; then echo 0; else echo '+mode+'; fi; else exec '+stat+' "$@"; fi\n')
            (bin/'git').write_text('#!'+python+'\nimport os,json\nfrom pathlib import Path\nPath('+repr(str(record))+').write_text(json.dumps({k:v for k,v in os.environ.items() if k in ["SATURN_BRIDGE_MANAGED_PURESIGNAL","SATURN_BRIDGE_REVIEWED_WDSP_DIR"]}))\nos.execv('+repr(git)+', ['+repr(git)+']+__import__("sys").argv[1:])\n')
            for f in bin.iterdir():f.chmod(0o755)
            env=dict(os.environ,PATH=str(bin)+':'+os.environ['PATH'],SATURN_PURESIGNAL_BUILD_CONFIG=str(config))
            for k in ['SATURN_BRIDGE_MANAGED_PURESIGNAL','SATURN_BRIDGE_REVIEWED_WDSP_DIR']:env.pop(k,None)
            if override:env.update(override)
            run=subprocess.run(['bash',str(ROOT/'update_manager/scripts/saturn-release-build.sh'),'--source-remote',str(source),'--source-ref','main','--resolve-only'],env=env,text=True,capture_output=True)
            return run, json.loads(record.read_text()) if record.exists() else {},sentinel.exists()
    def test_profile_is_passed_to_real_source_resolution(self):
        r,e,_=self.run_fixture('SATURN_BRIDGE_MANAGED_PURESIGNAL="1"\nSATURN_BRIDGE_REVIEWED_WDSP_DIR="/var/lib/reviewed-native"\n');self.assertEqual(r.returncode,0,r.stderr);self.assertEqual(e['SATURN_BRIDGE_MANAGED_PURESIGNAL'],'1');self.assertEqual(e['SATURN_BRIDGE_REVIEWED_WDSP_DIR'],'/var/lib/reviewed-native')
    def test_explicit_environment_wins_over_profile(self):
        r,e,_=self.run_fixture('SATURN_BRIDGE_MANAGED_PURESIGNAL=1\n',override={'SATURN_BRIDGE_MANAGED_PURESIGNAL':'0'});self.assertEqual(r.returncode,0,r.stderr);self.assertEqual(e['SATURN_BRIDGE_MANAGED_PURESIGNAL'],'0')
    def test_writable_profile_or_unknown_setting_is_rejected(self):
        for content,mode in [('SATURN_BRIDGE_MANAGED_PURESIGNAL=1\n','666'),('PATH=/bad\n','644')]:
            r,_,_=self.run_fixture(content,mode);self.assertNotEqual(r.returncode,0)

if __name__=='__main__':unittest.main()
