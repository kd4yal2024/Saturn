import { describe, expect, it, vi } from 'vitest';
import { HardwarePttSession } from '../src/transport/hardware-ptt';

function fixture() {
  const start = vi.fn(async (_token: string) => true);
  const stop = vi.fn();
  const options = { start, stop, canStart: () => true, transmitting: () => false, ready: () => true };
  const session = new HardwarePttSession(options);
  return { session, options, start, stop };
}
describe('rear PTT', () => {
  it('starts once per press and stops on its matching release', () => {
    const f = fixture(); f.session.text('saturn_ptt:0,true,1;saturn_ptt:0,true,1;');
    expect(f.start).toHaveBeenCalledTimes(1); expect(f.start).toHaveBeenCalledWith('1');
    f.session.text('saturn_ptt:0,false,1;saturn_ptt:0,false,1;'); expect(f.stop).toHaveBeenCalledTimes(1);
    f.session.text('saturn_ptt:0,true,1;'); expect(f.start).toHaveBeenCalledTimes(1);
  });
  it('does not steal existing MOX or act as a viewer', () => {
    for (const reason of ['viewer', 'mox']) {
      const f = fixture(); f.options.canStart = () => reason !== 'viewer'; f.options.transmitting = () => reason === 'mox';
      f.session.text('saturn_ptt:0,true,1;saturn_ptt:0,false,1;');
      expect(f.start).not.toHaveBeenCalled(); expect(f.stop).not.toHaveBeenCalled();
    }
  });
  it('release cancels an unresolved start and late completion cannot restore it', async () => {
    const f = fixture(); let complete!: (v: boolean) => void;
    f.start.mockImplementation(() => new Promise(resolve => { complete = resolve; }));
    f.session.text('saturn_ptt:0,true,1;'); f.session.text('saturn_ptt:0,false,1;'); expect(f.stop).toHaveBeenCalledTimes(1);
    complete(true); await Promise.resolve();
    f.session.text('saturn_ptt:0,false,1;'); expect(f.stop).toHaveBeenCalledTimes(1);
  });
  it('old start completion cannot cancel a new connection using the same number', async () => {
    const f = fixture(); let complete!: (v: boolean) => void;
    f.start.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
    f.session.text('saturn_ptt:0,true,1;'); f.session.reset(); f.session.text('saturn_ptt:0,true,1;');
    complete(false); await Promise.resolve(); f.session.text('saturn_ptt:0,false,1;');
    expect(f.start).toHaveBeenCalledTimes(2); expect(f.stop).toHaveBeenCalledTimes(2);
  });
  it('old releases and malformed or oversized numbers cannot affect a fresh press', () => {
    const f = fixture(); f.session.text('saturn_ptt:0,true,1;'); f.session.text('saturn_ptt:0,false,1;'); f.session.text('saturn_ptt:0,true,2;');
    f.session.text('saturn_ptt:0,false,1;saturn_ptt:0,true,0;saturn_ptt:0,true,18446744073709551616;');
    expect(f.start).toHaveBeenCalledTimes(2); expect(f.stop).toHaveBeenCalledTimes(1);
    f.session.text('saturn_ptt:0,false,2;'); expect(f.stop).toHaveBeenCalledTimes(2);
  });
  it('rejection cancels pending mic acquisition but never stops unrelated MOX', () => {
    const f = fixture(); f.session.text('saturn_ptt:0,true,1;'); f.session.text('saturn_ptt_rejected:0,1,Expired;');
    expect(f.stop).toHaveBeenCalledTimes(1); f.session.disown();
    f.session.text('saturn_ptt:0,false,1;'); expect(f.stop).toHaveBeenCalledTimes(1);
  });
  it('a delayed rejection for an old press cannot stop the current press', () => {
    const f = fixture(); f.session.text('saturn_ptt:0,true,1;'); f.session.text('saturn_ptt:0,false,1;'); f.session.text('saturn_ptt:0,true,2;saturn_ptt_rejected:0,1,Expired;');
    expect(f.stop).toHaveBeenCalledTimes(1);
    f.session.text('saturn_ptt:0,false,2;'); expect(f.stop).toHaveBeenCalledTimes(2);
  });
  it('waits for delivery of host cleanup with the original press token', () => {
    const f=fixture(); f.options.ready=()=>false;
    f.session.text('saturn_ptt:0,true,19;'); expect(f.start).not.toHaveBeenCalled();
    f.options.ready=()=>true; f.session.text('saturn_tx_owner:shared,2,ReadyOff;');
    expect(f.start).toHaveBeenCalledWith('19');
    f.session.text('saturn_ptt:0,false,19;'); expect(f.stop).toHaveBeenCalledWith('19');
  });
  it('release before readiness discards the pending press without touching another request', () => {
    const f=fixture(); f.options.ready=()=>false;
    f.session.text('saturn_ptt:0,true,19;saturn_ptt:0,false,19;');
    f.options.ready=()=>true; f.session.text('ready;');
    expect(f.start).not.toHaveBeenCalled(); expect(f.stop).not.toHaveBeenCalled();
  });
});
