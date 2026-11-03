"""Versioned UI-only installation. Leaves immutable release and Bridge intact.

Changes the existing activation-owned webroot assignment, so a future normal
release activation replaces it with that release's webroot. No extra overriding
drop-in, firmware operation, radio configuration or transmit command is added.
"""
import base64,fcntl,hashlib,json,os,re,shutil,ssl,subprocess,sys,tempfile,time,urllib.request
from pathlib import Path

STAGE=Path(__file__).resolve().parent
CONF=Path('/etc/systemd/system/saturn-go.service.d/50-saturn-release.conf')
CURRENT=Path('/opt/saturn/current')
LOCK=Path('/run/lock/saturn-maintenance/release.lock')
def command(*args):
    return subprocess.check_output(args,text=True,stderr=subprocess.STDOUT,timeout=40).strip()
def pid(unit):return int(command('systemctl','show',unit,'-p','MainPID','--value'))
def environment(unit):
    return dict(v.split('=',1) for v in Path(f'/proc/{pid(unit)}/environ').read_text().split('\0') if '=' in v)
def sha(path):return hashlib.sha256(Path(path).read_bytes()).hexdigest()
def write_conf(text):
    tmp=CONF.with_name(CONF.name+'.ui-new');tmp.write_text(text);tmp.chmod(0o644);os.replace(tmp,CONF)
def fresh_idle(expected_pid):
    path=Path('/run/saturn-bridge/xdma-ready.json');row=json.loads(path.read_text())
    assert time.time()-path.stat().st_mtime<5,'Bridge readiness snapshot expired'
    assert pid('saturn-bridge')==expected_pid==row['metrics']['pid'],'Bridge changed'
    assert row['shared_host']['clean_idle'],'TX owner must be idle for UI reconnect'
    assert row['shared_host']['rx_frames']>100,'Receive readiness missing'
    return row
def web_ready(root,payload):
    end=time.monotonic()+45;last=''
    while time.monotonic()<end:
        try:
            # Always obtain auth from the fresh Go process, never the old PID.
            env=environment('saturn-go');assert env['SATURN_WEBROOT']==str(root)
            auth=env.get('SATURN_REMOTE_BASIC_AUTH','')
            headers={'Authorization':'Basic '+base64.b64encode(auth.encode()).decode()} if auth else {}
            for route,name in [('remote-next','saturn-remote-next.html'),('remote-assets/remote-next.js','saturn-remote-next.js')]:
                req=urllib.request.Request('https://127.0.0.1:8443/'+route,headers=headers)
                with urllib.request.urlopen(req,context=ssl._create_unverified_context(),timeout=5) as response:
                    assert response.status==200
                    assert hashlib.sha256(response.read()).hexdigest()==payload[name],route+' asset mismatch'
            for route in ['telemetry','settings']:
                with urllib.request.urlopen('http://127.0.0.1:8080/'+route,timeout=5) as response:assert response.status==200
            return
        except (OSError,ValueError,KeyError,AssertionError,subprocess.CalledProcessError) as error:last=str(error);time.sleep(.3)
    raise RuntimeError('Authenticated web readiness deadline: '+last)
def save(record,backup):
    text=json.dumps(record,indent=2)+'\n';(backup/'result.json').write_text(text)
    (STAGE/'deploy-result.json').write_text(text);(STAGE/'deploy-result.json').chmod(0o644)
def restore(record,backup):
    assert CURRENT.resolve()==Path(record['release']),'Release changed since UI deployment'
    assert CONF.read_text()==record['newConfig'],'Webroot configuration changed since UI deployment'
    fresh_idle(record['bridgePid']);write_conf(record['oldConfig']);command('systemctl','daemon-reload');command('systemctl','restart','saturn-go')
    web_ready(Path(record['previousWebroot']),record['previousAssets']);fresh_idle(record['bridgePid'])
    record['rolledBack']=True;save(record,backup)
def main():
    assert os.geteuid()==0;os.umask(0o077)
    descriptor=os.open(LOCK,os.O_RDWR|os.O_NOFOLLOW)
    fcntl.flock(descriptor,fcntl.LOCK_EX|fcntl.LOCK_NB)
    try:
        if len(sys.argv)==3 and sys.argv[1]=='--rollback':
            backup=Path(sys.argv[2]);assert backup.is_absolute() and not backup.is_symlink() and backup.stat().st_uid==0
            restore(json.loads((backup/'result.json').read_text()),backup);print('UI rollback verified');return
        assert sys.argv[1:]==['--apply']
        identity=json.loads((STAGE/'identity.json').read_text());release=CURRENT.resolve()
        assert release.name==identity['baseCommit'];assert not CONF.is_symlink()
        for name,digest in identity['assets'].items():assert sha(STAGE/name)==digest,name
        bridge=pid('saturn-bridge');assert bridge>0 and pid('p2app')==0;before=fresh_idle(bridge)
        env=environment('saturn-go');oldroot=Path(env['SATURN_WEBROOT']);assert oldroot.resolve()==release/'webroot'
        old=CONF.read_text();assert old.count('Environment=SATURN_WEBROOT=')==1
        root=Path('/opt/saturn/ui')/identity['commit']/'webroot';assert not root.parent.is_symlink()
        if root.exists():
            assert root.stat().st_uid==0 and not root.is_symlink()
            for original in oldroot.rglob('*'):
                if original.is_file() and original.name not in [*identity['assets'],'saturn-remote-next.js.sha256']:
                    assert sha(root/original.relative_to(oldroot))==sha(original)
            for name,digest in identity['assets'].items():assert sha(root/name)==digest
        previous_assets={name:sha(oldroot/name) for name in identity['assets']}
        unchanged=[release/'SHA256SUMS',release/'release-manifest.json',release/'bin/saturn-bridge',release/'bin/saturn-go',
            Path('/etc/default/saturn-puresignal'),Path('/var/lib/saturn-state/remote_settings.json'),
            Path('/var/lib/saturn-bridge/puresignal/receive-state/rx-state-v1.txt')]
        hashes={str(p):sha(p) for p in unchanged if p.is_file()}
        backup=Path(tempfile.mkdtemp(prefix='ps-monitor-ui-before-',dir='/var/lib/saturn-state/deployments'))
        shutil.copy2(__file__,backup/'install-ui.py');shutil.copy2(CONF,backup/CONF.name)
        new=re.sub(r'^Environment=SATURN_WEBROOT=.*$', 'Environment=SATURN_WEBROOT='+str(root),old,flags=re.M)
        record={'format':'saturn-ui-install-v1','commit':identity['commit'],'release':str(release),'backup':str(backup),
            'bridgePid':bridge,'oldConfig':old,'newConfig':new,'previousWebroot':str(oldroot),'previousAssets':previous_assets,
            'webroot':str(root),'assets':identity['assets'],'unchangedHashes':hashes,'pttCommandsSent':0,
            'bridgeRestarted':False,'phase':'prepared'};save(record,backup)
        (backup/'rollback-command.txt').write_text('sudo python3 '+str(backup/'install-ui.py')+' --rollback '+str(backup)+'\n')
        root.parent.mkdir(parents=True,mode=0o755,exist_ok=True);root.parent.chmod(0o755);root.parent.parent.chmod(0o755)

        if not root.exists():shutil.copytree(oldroot,root)
        root.chmod(0o755)
        for name in identity['assets']:
            shutil.copyfile(STAGE/name,root/name);(root/name).chmod(0o644)
        checksum=root/'saturn-remote-next.js.sha256';checksum.write_text(identity['assets']['saturn-remote-next.js']+'  saturn-remote-next.js\n');checksum.chmod(0o644)
        changed=False
        try:
            fresh_idle(bridge);changed=True;write_conf(new);command('systemctl','daemon-reload');command('systemctl','restart','saturn-go')
            web_ready(root,identity['assets']);after=fresh_idle(bridge)
            assert after['shared_host']['rx_frames']>before['shared_host']['rx_frames']
            assert all(sha(path)==digest for path,digest in hashes.items()),'Installed release or radio settings changed'
            assert CURRENT.resolve()==release
            record.update(phase='installed',authenticatedAssetsVerified=True,receiveContinues=True,configurationPreserved=True);save(record,backup)
        except BaseException as error:
            record['error']=str(error);record['phase']='failed';save(record,backup)
            if changed:restore(record,backup)
            raise
        print(json.dumps({k:record[k] for k in ['phase','commit','release','webroot','backup','bridgePid','pttCommandsSent','bridgeRestarted','authenticatedAssetsVerified','receiveContinues','configurationPreserved']}))
    finally:os.close(descriptor)
if __name__=='__main__':main()
