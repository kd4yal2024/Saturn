import { describe, it, expect } from 'vitest';
import { mkdtempSync, writeFileSync, chmodSync, readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { startDisplayBridge, readProcessCpu } from '../scripts/lib/display-bridge-replay.mjs';
import { pause } from '../scripts/lib/chrome-session.mjs';

describe.skipIf(process.platform!=='linux')('offline fixture exit evidence',()=>{
  it('preserves logs and exit status when a ready fixture dies before CPU sampling',async()=>{
    const root=mkdtempSync(join(tmpdir(),'saturn-fixture-exit-'));
    const binary=join(root,'fake-fixture');
    writeFileSync(binary,`#!${process.execPath}
const fs=require('fs');const path=require('path');
fs.writeFileSync(path.join(process.env.SATURN_DISPLAY_REPLAY_DIR,'ready.json'),JSON.stringify({port:Number(process.env.SATURN_DISPLAY_REPLAY_PORT),hardwareAccess:false,synthetic:true}));
console.log('retained fixture failure');setInterval(()=>{if(fs.existsSync(path.join(process.env.SATURN_DISPLAY_REPLAY_DIR,'exit-now')))process.exit(17)},10);
`);
    chmodSync(binary,0o755);
    let fixture;
    try {
      fixture=await startDisplayBridge(binary,join(root,'run'));
      expect(readProcessCpu(fixture.pid)).not.toBeNull();
      writeFileSync(join(root,'run/exit-now'),'');
      for(let i=0;i<100&&readProcessCpu(fixture.pid);i++)await pause(20);
      expect(readProcessCpu(fixture.pid)).toBeNull();
      let failure;
      try {await fixture.stop()} catch(error){failure=error}
      expect(failure?.fixtureEvidence.exit.code).toBe(17);
      expect(failure?.fixtureEvidence.result).toBeNull();
      expect(readFileSync(join(root,'run/bridge.log'),'utf8')).toContain('retained fixture failure');
    }finally{if(fixture)await fixture.stop().catch(()=>{});rmSync(root,{recursive:true,force:true})}
  });
});
