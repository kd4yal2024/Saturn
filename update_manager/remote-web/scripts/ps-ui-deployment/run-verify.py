"""Private stdin handoff for the receive-only UI check; never prints credentials."""
from pathlib import Path
import subprocess
root=Path(__file__).resolve().parent
reader="""from pathlib import Path
import json,subprocess
p=int(subprocess.check_output(['systemctl','show','saturn-go','-p','MainPID','--value'],text=True))
env=dict(v.split('=',1) for v in Path(f'/proc/{p}/environ').read_text().split('\\0') if '=' in v)
u,pw=env['SATURN_REMOTE_BASIC_AUTH'].split(':',1)
print(json.dumps(dict(username=u,password=pw)))
"""
secret=subprocess.check_output(['ssh','-o','BatchMode=yes','pi@192.168.0.139','python3','-'],input=reader.encode(),timeout=15)
raise SystemExit(subprocess.run(['node',str(root/'verify-ui.mjs')],input=secret,timeout=100).returncode)
