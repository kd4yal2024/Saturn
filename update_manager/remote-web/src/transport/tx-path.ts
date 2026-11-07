import { managedSettingsMessage } from './managed-pcm';

export type TxPathPreference = 'auto' | 'legacy' | 'managed';
export type TxPath = 'pending' | 'legacy' | 'managed' | 'blocked';

/** Select a client protocol, never host RF permission or a TX arm.
 * Ordered shared snapshots advertise capabilities before ready. Older Legacy
 * hosts omit them; absence is interpreted only at the ready boundary.
 */
export class TxPathNegotiator {
  path: TxPath = 'pending';
  reason = 'Waiting for host TX owner';
  private shared = false;
  private snapshot: string[] = [];
  private bytes = 0;
  constructor(private readonly preference: TxPathPreference = 'auto') {}
  get ready(): boolean { return this.path === 'legacy' || this.path === 'managed'; }
  observe(text: string): boolean {
    const before = this.path;
    for (const raw of text.split(';')) {
      const c = raw.trim();
      if (!c) continue;
      const shared = c.startsWith('saturn_tx_media_caps:') || c.startsWith('saturn_tx_owner:shared,');
      if (shared && this.path === 'legacy') {
        this.path = 'blocked'; this.reason = 'Host TX owner changed; disconnect and reconnect';
      }
      this.shared ||= shared;
      if (this.path !== 'pending') continue;
      // Retain negotiation only, never large spectra, audio or pairing secrets.
      if (/^(?:ready$|tx_enable:|tx_codec_(?:accept|reject):|saturn_rx_tuning_caps:|saturn_tx_(?:media_caps|owner|arm_policy|drive_limit|committed_drive|fault_ack_caps|fault_ack_state):|saturn_ps_(?:control_caps|mode|measurement_caps|stimulus_caps|two_tone_caps|native_two_tone_caps|manual_attenuation_caps|fault_attenuation_caps|auto_caps):)/.test(c)) {
        this.bytes += c.length + 1;
        if (this.bytes > 16384) {
          this.path = 'blocked'; this.reason = 'Host TX negotiation exceeded its bounded snapshot';
          this.snapshot = []; continue;
        }
        this.snapshot.push(c);
      }
      if (c !== 'ready') continue;
      const advertised = this.shared ? 'managed' : 'legacy';
      if (this.preference !== 'auto' && this.preference !== advertised) {
        this.path = 'blocked';
        this.reason = `Selected ${this.preference} TX conflicts with host ${advertised}; select Auto and reconnect`;
      } else {
        this.path = advertised;
        this.reason = advertised === 'managed' ? 'Host shared owner: Managed tagged PCM / SATP'
          : 'Host Legacy owner: Legacy TX';
      }
    }
    return before !== this.path;
  }
  takeSnapshot(): string {
    const text = this.snapshot.join(';') + ';';
    this.snapshot = []; return text;
  }
  filterControl(text: string): string {
    if (this.ready) return text;
    // Never queue/replay stale TX intent while discovery is unresolved.
    return text.split(';').map(c => c.trim()).filter(c => c && (
      c === 'trx:0,false' || c === 'saturn_ps_measurement:0,stop' || (
        !managedSettingsMessage(c) && !/^(?:tx_|drive:|trx:|saturn_ps_|saturn_tx_|saturn_satp_control:)/.test(c)
      ))).map(c => c + ';').join('');
  }
}
