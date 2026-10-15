"""Actual Saturn Remote UI against explicit offline owner fixtures; no RF/G2."""
import argparse
import datetime
import hashlib
import http.server
import json
import os
import re
import select
import socket
import struct
from pathlib import Path
import threading
import numpy as np
from scipy.io import wavfile
from urllib.parse import urlparse, parse_qs
from playwright.sync_api import sync_playwright
from run_ps_browser import make_input, write_json
from real_saturn_go_proxy import RealSaturnGoProxy


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--speech', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--bridge', required=True)
    parser.add_argument('--lifecycle-pcm-only', action='store_true')
    args = parser.parse_args()
    bridge = urlparse(args.bridge)
    if bridge.scheme != 'ws' or bridge.hostname != '127.0.0.1':
        parser.error('explicit loopback ws fixture required')
    template = Path(os.environ['PS_REMOTE_TEMPLATE']).resolve()
    bundle = Path(os.environ['PS_REMOTE_BUNDLE']).resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    samples = make_input(args.speech.resolve(), output)
    # Chrome's file-backed fake microphone consumes PCM16 WAV, not the float32
    # analyzer WAV. Keep both inputs and record the conversion explicitly.
    wavfile.write(output / 'microphone-pcm16.wav', 48000,
                  np.rint(np.clip(samples, -1, 1) * 32767).astype(np.int16))
    # Freeze the exact shipped page and generated bundle served to this run.
    (output / 'saturn-remote-next.html').write_bytes(template.read_bytes())
    (output / 'saturn-remote-next.js').write_bytes(bundle.read_bytes())
    write_json(output / 'run-metadata.json', {
        'startedAtIso': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'bridge': args.bridge, 'offline': True, 'physicalRf': False,
        'templateSha256': hashlib.sha256(template.read_bytes()).hexdigest(),
        'bundleSha256': hashlib.sha256(bundle.read_bytes()).hexdigest(),
        'microphone': 'real getUserMedia and AudioWorklet; Chrome fake-device WAV input',
        'microphoneFileSha256': hashlib.sha256((output / 'microphone-pcm16.wav').read_bytes()).hexdigest(),
    })

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass
        def do_GET(self):
            path = urlparse(self.path).path
            if self.headers.get('Upgrade', '').lower() == 'websocket':
                # Preserve the actual UI's same-origin proxy policy. Tunnel
                # bytes only to the explicitly validated loopback fixture.
                upstream = socket.create_connection(('127.0.0.1', bridge.port), timeout=3)
                try:
                    # Mirror remote_tls.rs bridge_dial_url / split_proxy_lane_message:
                    # public /saturn/{control,media} -> upstream /{control,media},
                    # then a session_lane initialization before browser traffic.
                    lane = {'/saturn/control': 'control', '/saturn/media': 'media'}.get(path)
                    target = '/' + lane if lane else '/tci'
                    request = f'GET {target} HTTP/1.1\r\n' + ''.join(f'{k}: {v}\r\n' for k, v in self.headers.items()) + '\r\n'
                    upstream.sendall(request.encode())
                    response = b''
                    while b'\r\n\r\n' not in response:
                        response += upstream.recv(8192)
                    if lane:
                        session = parse_qs(urlparse(self.path).query)['session'][0]
                        if not session or len(session) > 64 or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-' for c in session):
                            raise ValueError('invalid offline split session')
                        payload = f'session_lane:{session},{lane};'.encode()
                        mask = os.urandom(4)
                        header = bytes([0x81, 0x80 | len(payload)]) if len(payload) < 126 else bytes([0x81, 0xfe]) + struct.pack('!H', len(payload))
                        upstream.sendall(header + mask + bytes(v ^ mask[i % 4] for i, v in enumerate(payload)))
                    self.connection.sendall(response)
                    upstream.settimeout(None)
                    while True:
                        readable, _, _ = select.select([self.connection, upstream], [], [], 5)
                        for source in readable:
                            data = source.recv(65536)
                            if not data:
                                return
                            (upstream if source is self.connection else self.connection).sendall(data)
                except (ConnectionError, OSError):
                    return
                finally:
                    upstream.close()
            if path == '/fixture/disconnect':
                (output / 'request-disconnect').touch(exist_ok=False)
                self.send_response(200); self.end_headers(); return
            if path == '/remote-next':
                data, kind = template.read_bytes(), 'text/html'
            elif path == '/remote-assets/remote-next.js':
                data, kind = bundle.read_bytes(), 'text/javascript'
            else:
                self.send_response(404); self.end_headers(); return
            self.send_response(200); self.send_header('content-type', kind)
            self.send_header('content-length', str(len(data))); self.end_headers(); self.wfile.write(data)
        def do_POST(self):
            self.send_response(404); self.end_headers()

    executable = os.environ.get('PS_SATURN_GO_BIN')
    proxy = RealSaturnGoProxy(executable, args.bridge, template, bundle, output) if executable else None
    server = None if proxy else http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = None if proxy else threading.Thread(target=server.serve_forever, daemon=True)
    if thread:
        thread.start()
    events, errors, frames, snapshots, console, ui_trace = [], [], [], [], [], []
    sockets, requests = [], []
    page = None
    try:
        if proxy:
            proxy.start()
        origin = proxy.origin if proxy else f'http://127.0.0.1:{server.server_port}'
        with sync_playwright() as p:
            browser = p.chromium.launch(executable_path='/usr/bin/google-chrome', headless=True, args=[
                '--no-sandbox', '--autoplay-policy=no-user-gesture-required',
                '--use-fake-device-for-media-stream', '--use-fake-ui-for-media-stream',
                f'--use-file-for-fake-audio-capture={output / "microphone-pcm16.wav"}',
                '--disable-background-timer-throttling',
            ])
            cookies = None
            if proxy:
                # Obtain a real remember-device cookie from the actual Basic
                # login path, then use a fresh context WITHOUT Basic credentials
                # for every UI/WSS lifecycle action. Never archive cookie values.
                login = browser.new_context(ignore_https_errors=True,
                    http_credentials={'username': proxy.username, 'password': proxy.password})
                login.new_page().goto(f'{origin}/remote-next')
                cookies = login.cookies(origin)
                proxy.check_browser_cookie(cookies)
                login.close()
            context = browser.new_context(viewport={'width': 1440, 'height': 1000},
                ignore_https_errors=bool(proxy))
            if cookies:
                context.add_cookies(cookies)
            # Never let the replay browser contact a radio/external service.
            def loopback_only(route):
                url = urlparse(route.request.url)
                if url.scheme in ['http', 'https', 'ws', 'wss'] and url.hostname != '127.0.0.1':
                    errors.append('non-loopback request refused: ' + route.request.url)
                    route.abort()
                else:
                    route.continue_()
            context.route('**/*', loopback_only)
            page = context.new_page()
            page.on('request', lambda request: requests.append({'url': request.url, 'method': request.method}))
            page.set_default_timeout(7000)
            page.on('pageerror', lambda e: errors.append(str(e)))
            page.on('console', lambda e: console.append({'type': e.type, 'text': e.text}))
            page.on('dialog', lambda d: d.accept())
            cdp = page.context.new_cdp_session(page); cdp.send('Network.enable')
            cdp.on('Network.webSocketFrameReceived', lambda e: events.append(e))
            cdp.on('Network.webSocketFrameSent', lambda e: frames.append(e))
            cdp.on('Network.webSocketCreated', lambda e: sockets.append({'requestId': e['requestId'], 'url': e['url']}))
            page.expose_function('recordManagedUiTrace', lambda value: ui_trace.append(value))
            write_json(output / 'browser-identity.json', {'version': browser.version,
                'path': 'actual template + IIFE + actual UI buttons + real microphone/worklet APIs'})
            page.goto(f'{origin}/remote-next' if proxy else f'{origin}/remote-next?transport=split')
            if proxy:
                assert parse_qs(urlparse(page.url).query) == {
                    'transport': ['split'], 'tx_opus': ['1'], 'tx_cfc': ['1']}
            print('Actual UI loaded', flush=True)
            page.wait_for_function('Boolean(window.SaturnRemoteNext) && Boolean(document.querySelector("#managed-pcm-select"))')
            page.wait_for_timeout(300)
            page.evaluate('''() => {
                window.managedUiTrace=[];
                new MutationObserver(() => window.recordManagedUiTrace({
                    at:performance.now(), status:document.querySelector('#managed-pcm-status').textContent,
                    result:document.querySelector('#managed-pcm-transaction').textContent,
                    tx:document.querySelector('#mox-btn').textContent
                })).observe(document.querySelector('#managed-pcm-status'),{childList:true});
            }''')
            assert page.locator('#managed-pcm-select').input_value() == 'legacy'
            assert not frames, 'default page must not open or arm a managed radio'
            page.evaluate('''() => {
                const select = document.querySelector('#managed-pcm-select');
                select.value = 'managed'; select.dispatchEvent(new Event('change', {bubbles:true}));
            }''')
            page.evaluate("url => { document.querySelector('#ws-url').value = url; }", proxy.ws_url if proxy else args.bridge + '/tci')
            for session in range(3):
                print(f'Actual UI connecting session {session}', flush=True)
                page.evaluate("document.querySelector('#go-live-btn').click()")
                page.wait_for_function("document.querySelector('#managed-pcm-status').dataset.phase === 'ready'", timeout=7000)
                if proxy:
                    pair = sockets[-2:]
                    sessions = {parse_qs(urlparse(s['url']).query)['session'][0] for s in pair}
                    assert len(sessions) == 1 and {urlparse(s['url']).path for s in pair} == {'/saturn/control', '/saturn/media'}
                    assert all(urlparse(s['url']).scheme == 'wss' and urlparse(s['url']).netloc == urlparse(origin).netloc for s in pair)
                    proxy.paired(sessions.pop(), session)
                page.evaluate('''() => {
                    const gain = document.querySelector('#tx-mic-gain'); gain.value='-3';
                    gain.dispatchEvent(new Event('input',{bubbles:true})); gain.dispatchEvent(new Event('change',{bubbles:true}));
                }''')
                # EQ is a separate real UI message, not a batched parser input.
                page.evaluate("document.querySelector('#tx-eq-enable-btn').click()")
                page.wait_for_function("document.querySelector('#managed-pcm-status').dataset.phase === 'ready' && document.querySelector('#managed-pcm-transaction').textContent.includes('applied')", timeout=5000)
                for action in range(2):
                    page.evaluate("document.querySelector('#tx-arm-btn').click()")
                    page.wait_for_function("!document.querySelector('#mox-btn').disabled", timeout=2000)
                    page.evaluate("document.querySelector('#mox-btn').click()")
                    page.wait_for_function("document.querySelector('#managed-pcm-status').dataset.phase === 'streaming'", timeout=5000)
                    page.wait_for_function("document.querySelector('#mox-btn').textContent.includes('ON')", timeout=5000)
                    page.wait_for_timeout(300)
                    snapshots.append({'session': session, 'action': action,
                        'status': page.locator('#managed-pcm-status').text_content(),
                        'transaction': page.locator('#managed-pcm-transaction').text_content(),
                        'tx': page.locator('#mox-btn').text_content()})
                    if session == 1 and action == 1:
                        # Actual socket loss while keyed. UI disconnect path is
                        # not replaced by a direct controller call.
                        if proxy:
                            (output / 'request-disconnect').touch(exist_ok=False)
                        else:
                            page.request.get(f'http://127.0.0.1:{server.server_port}/fixture/disconnect')
                        page.wait_for_function("document.querySelector('#managed-pcm-status').dataset.phase === 'disconnected'", timeout=5000)
                        page.evaluate("document.querySelector('#go-live-btn').click()") # cancel automatic retry
                    else:
                        page.evaluate("document.querySelector('#mox-btn').click()")
                        page.wait_for_function("document.querySelector('#managed-pcm-status').dataset.phase === 'ready'", timeout=5000)
                if session != 1:
                    if session == 2 and os.environ.get('PS_UI_PREQUEUE_REJECTION') == '1':
                        # Exercise the real UI -> real Bridge configure_batch
                        # validation path. The offline owner is capped at 5 W;
                        # this 6 W proposal is rejected BEFORE an owner envelope
                        # exists. No RF or device is involved in this fixture.
                        first_reply = len(events)
                        page.evaluate('''() => {
                            const drive = document.querySelector('#tx-drive'); drive.value='6';
                            drive.dispatchEvent(new Event('input',{bubbles:true}));
                            drive.dispatchEvent(new Event('change',{bubbles:true}));
                        }''')
                        page.wait_for_function("document.querySelector('#managed-pcm-transaction').textContent.includes('rejected (InvalidConfig)') && document.querySelector('#managed-pcm-transaction').textContent.includes('0 pending')", timeout=5000)
                        replies = [e['response']['payloadData'] for e in events[first_reply:]
                                   if e['response']['opcode'] == 1]
                        rejected = next(re.search(r'tx_config:0,([1-9][0-9]*),rejected,InvalidConfig,0;', text)
                                        for text in replies if re.search(r'tx_config:0,([1-9][0-9]*),rejected,InvalidConfig,0;', text))
                        transaction_id = rejected.group(1)
                        assert not any(f'tx_config:0,{transaction_id},queued,' in text for text in replies)
                        immediate = page.locator('#managed-pcm-transaction').text_content()
                        # An orphan awaiting-queued request would otherwise
                        # overwrite the reason at the five-second deadline.
                        page.wait_for_timeout(5200)
                        after_deadline = page.locator('#managed-pcm-transaction').text_content()
                        assert after_deadline == immediate
                        assert page.locator('#managed-pcm-status').get_attribute('data-phase') == 'fault'
                        assert 'InvalidConfig' in page.locator('#managed-pcm-status').text_content()
                        page.screenshot(path=str(output / 'prequeue-rejection.png'), full_page=True)
                        write_json(output / 'prequeue-rejection.json', {
                            'request': 'actual tx-drive control: 6 W (offline cap 5 W)',
                            'reply': rejected.group(0), 'queuedBeforeTerminal': False,
                            'immediate': immediate, 'afterAcknowledgmentDeadline': after_deadline,
                            'status': page.locator('#managed-pcm-status').text_content(),
                            'physicalRf': False,
                        })
                    page.evaluate("document.querySelector('#go-live-btn').click()")
                    page.wait_for_function("document.querySelector('#managed-pcm-status').dataset.phase === 'disconnected'", timeout=5000)
                page.wait_for_timeout(200)
                if proxy:
                    proxy.wait_metrics(0, 0, f'connection {session}: all proxy slots released')
                page.evaluate("window.dispatchEvent(new Event('online'))")
            assert not errors, errors
            assert len(snapshots) == 6
            assert any(e['response']['opcode'] == 2 for e in frames), 'no actual PCM frames'
            assert all('opus' not in e['response']['payloadData'] for e in frames
                       if e['response']['opcode'] == 1 and 'tx_codec_caps:' in e['response']['payloadData'])
            page.evaluate("document.querySelector('#header-setup-btn').click(); document.querySelector('#setup-tab-tx').click()")
            page.locator('#setup-tx-puresignal-section').scroll_into_view_if_needed()
            page.screenshot(path=str(output / 'remote-ui.png'), full_page=True)
            if proxy:
                proxy.check_browser_cookie(context.cookies())
                assert len(sockets) == 6
                lanes = {s['requestId']: urlparse(s['url']).path for s in sockets}
                assert all(lanes[e['requestId']] == '/saturn/media' for e in frames if e['response']['opcode'] == 2)
                assert all(lanes[e['requestId']] == '/saturn/control' for e in frames if e['response']['opcode'] == 1)
            browser.close()
    finally:
        write_json(output / 'ui-snapshots.json', snapshots)
        write_json(output / 'received-frames.json', events)
        write_json(output / 'sent-frames.json', frames)
        write_json(output / 'page-errors.json', errors)
        write_json(output / 'browser-console.json', console)
        write_json(output / 'ui-trace.json', ui_trace)
        write_json(output / 'websocket-paths.json', sockets)
        write_json(output / 'http-requests.json', requests)
        if server:
            server.shutdown(); server.server_close(); thread.join()
        if proxy:
            proxy.close()
    print(f'Actual managed PCM UI retained: {output}', flush=True)


if __name__ == '__main__':
    main()
