"""Launch the unmodified Saturn Go executable for loopback-only offline replay.

No fake proxy/router, production credentials, installed services or hardware.
Private runtime state stays outside the review artifacts directory.
"""
import base64
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import ssl
import subprocess
import tempfile
import time
from urllib.parse import urlparse


class RealSaturnGoProxy:
    def __init__(self, executable, bridge_url, template, bundle, output):
        self.executable = Path(executable).resolve(strict=True)
        bridge = urlparse(bridge_url)
        if bridge.scheme != 'ws' or bridge.hostname != '127.0.0.1' or not bridge.port:
            raise ValueError('explicit loopback backend fixture required')
        self.runtime = Path(tempfile.mkdtemp(prefix='saturn-go-ps-private-', dir='/tmp'))
        self.output = output
        self.username, self.password = 'offline-review', secrets.token_hex(24)
        self.auth = 'Basic ' + base64.b64encode(f'{self.username}:{self.password}'.encode()).decode()
        self.port, self.admin_port = self.free_port(), self.free_port()
        if self.port == self.admin_port:
            raise RuntimeError('port allocation collision; retry with a fresh fixture')
        self.origin = f'https://127.0.0.1:{self.port}'
        self.ws_url = f'wss://127.0.0.1:{self.port}/tci'
        self.bridge_url, self.template, self.bundle = bridge_url, template, bundle
        self.process = None
        self.log = None
        self.checks = []
        self.metrics = []
        self.context = ssl._create_unverified_context() # startup only; pin generated cert below

    @staticmethod
    def free_port():
        with socket.socket() as s:
            s.bind(('127.0.0.1', 0))
            return s.getsockname()[1]

    def request(self, path, headers=None):
        conn = http.client.HTTPSConnection('127.0.0.1', self.port, context=self.context, timeout=3)
        try:
            conn.request('GET', path, headers=headers or {})
            response = conn.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            conn.close()

    def expect(self, label, path, expected, headers=None):
        status, response_headers, body = self.request(path, headers)
        self.checks.append({'check': label, 'path': path, 'expected': expected, 'status': status})
        assert status == expected, (label, status, expected)
        return response_headers, body

    def handshake(self, label, path, expected, auth=None, origin=None):
        headers = {'Upgrade': 'websocket', 'Connection': 'Upgrade',
                   'Sec-WebSocket-Version': '13',
                   'Sec-WebSocket-Key': base64.b64encode(secrets.token_bytes(16)).decode(),
                   'Origin': origin or self.origin}
        if auth:
            headers['Authorization'] = auth
        return self.expect(label, path, expected, headers)

    def start(self):
        webroot = self.runtime / 'webroot'
        webroot.mkdir()
        (webroot / 'saturn-remote-next.html').write_bytes(self.template.read_bytes())
        (webroot / 'saturn-remote-next.js').write_bytes(self.bundle.read_bytes())
        # Every startup persistence path is under this new private runtime.
        # No inherited SATURN_* variable may point at appliance state/services.
        env = {k: v for k, v in os.environ.items() if not k.startswith('SATURN_')}
        env.update({
            'SATURN_ADDR': f'127.0.0.1:{self.admin_port}',
            'SATURN_REMOTE_TLS_ADDR': f'127.0.0.1:{self.port}',
            'SATURN_REMOTE_BASIC_AUTH': f'{self.username}:{self.password}',
            'SATURN_STATE_DIR': str(self.runtime / 'state'),
            'SATURN_WEBROOT': str(webroot),
            'SATURN_SCRIPTS_DIR': str(self.runtime / 'scripts'),
            'SATURN_REPO_ROOT': str(self.runtime / 'no-appliance-repository'),
            'SATURN_REMOTE_BRIDGE_WS': self.bridge_url,
        })
        self.log = (self.output / 'saturn-go.log').open('wb')
        self.process = subprocess.Popen([str(self.executable)], env=env, cwd=self.runtime,
                                        stdout=self.log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + 8
        while True:
            if self.process.poll() is not None:
                raise RuntimeError('Saturn Go exited before TLS readiness; see sanitized runtime log')
            try:
                if self.request('/remote-next?transport=split')[0] == 401:
                    break
            except (OSError, http.client.HTTPException):
                pass
            if time.monotonic() > deadline:
                raise TimeoutError('actual Saturn Go TLS listener did not become ready')
            time.sleep(0.025)
        certificate = self.runtime / 'state/remote-tls/saturn-remote.crt'
        self.context = ssl.create_default_context(cafile=str(certificate))
        with socket.create_connection(('127.0.0.1', self.port), timeout=3) as conn:
            with self.context.wrap_socket(conn, server_hostname='127.0.0.1') as tls:
                self.tls = {'protocol': tls.version(), 'cipher': tls.cipher()[0],
                            'peerCertificateSha256': hashlib.sha256(tls.getpeercert(binary_form=True)).hexdigest(),
                            'pythonCertificateAndHostnameVerification': True,
                            'browserTrust': 'isolated self-signed certificate; ignore_https_errors in test context'}
        self.check_access()

    def check_access(self):
        page = '/remote-next?transport=split'
        headers, _ = self.expect('authenticated normal entry redirect', '/remote-next', 307,
                                 {'Authorization': self.auth})
        assert headers.get('location') == '/remote-next?transport=split&tx_opus=1&tx_cfc=1'
        headers, _ = self.expect('anonymous page denied', page, 401)
        assert headers.get('www-authenticate', '').startswith('Basic ')
        self.expect('wrong password denied', page, 401, {'Authorization': 'Basic ' +
                    base64.b64encode(b'offline-review:not-the-generated-password').decode()})
        self.expect('anonymous bundle denied', '/remote-assets/remote-next.js', 401)
        self.expect('anonymous metrics denied', '/remote_metrics', 401)
        for lane in ['control', 'media']:
            self.handshake(f'anonymous {lane} upgrade denied', f'/saturn/{lane}?session=auth-probe', 401)
            self.handshake(f'cross-origin {lane} upgrade denied', f'/saturn/{lane}?session=auth-probe',
                           403, self.auth, 'https://different.invalid')
            self.handshake(f'missing-session {lane} upgrade denied', f'/saturn/{lane}', 400, self.auth)
        headers, body = self.expect('authenticated actual page', page, 200, {'Authorization': self.auth})
        assert hashlib.sha256(body).digest() == hashlib.sha256(self.template.read_bytes()).digest()
        cookie = headers.get('set-cookie', '')
        assert all(v in cookie for v in ['Secure', 'HttpOnly', 'SameSite=Strict'])
        self.expect('cookie-only page access', page, 200, {'Cookie': cookie.split(';')[0]})
        self.expect('forged cookie denied', page, 401, {'Cookie': 'saturn_remote_auth=not-a-token'})
        _, body = self.expect('authenticated actual bundle', '/remote-assets/remote-next.js', 200,
                              {'Authorization': self.auth})
        assert hashlib.sha256(body).digest() == hashlib.sha256(self.bundle.read_bytes()).digest()
        self.wait_metrics(0, 0, 'before browser: rejected upgrades opened no upstream session')

    def wait_metrics(self, clients, connections, label):
        deadline = time.monotonic() + 3
        while True:
            status, _, body = self.request('/remote_metrics', {'Authorization': self.auth})
            assert status == 200
            snapshot = json.loads(body)
            if snapshot['active_clients'] == clients and snapshot['active_connections'] == connections:
                self.metrics.append({'check': label, **snapshot})
                return
            if time.monotonic() > deadline:
                raise AssertionError((label, snapshot))
            time.sleep(0.025)

    def paired(self, session_id, index):
        self.wait_metrics(1, 2, f'connection {index}: paired control/media consume one client')
        for lane in ['control', 'media']:
            self.handshake(f'connection {index}: duplicate {lane} rejected',
                           f'/saturn/{lane}?session={session_id}', 409, self.auth)
        self.wait_metrics(1, 2, f'connection {index}: duplicate upgrades changed no slot/connection')

    def check_browser_cookie(self, cookies):
        cookie = next(c for c in cookies if c['name'] == 'saturn_remote_auth')
        assert cookie['secure'] and cookie['httpOnly'] and cookie['sameSite'] == 'Strict'
        self.expect('actual browser cookie-only authenticated page', '/remote-next?transport=split',
                    200, {'Cookie': f"{cookie['name']}={cookie['value']}"})
        self.browser_cookie = {k: cookie[k] for k in ['name', 'secure', 'httpOnly', 'sameSite']}

    def close(self):
        if self.process is not None:
            self.process.send_signal(signal.SIGTERM)
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
                self.checks.append({'check': 'runtime cleanup', 'forcedKill': True})
            if self.log:
                self.log.close()
            log_text = (self.output / 'saturn-go.log').read_text()
            self.checks.append({'check': 'no insecure auth override',
                                'passed': 'WITHOUT basic auth' not in log_text
                                and 'accepted via disabled auth' not in log_text})
            self.checks.append({'check': 'graceful process shutdown',
                                'passed': self.process.returncode == 0
                                and 'Saturn server shut down' in log_text})
            report = {'executable': str(self.executable),
                      'executableSha256': hashlib.sha256(self.executable.read_bytes()).hexdigest(),
                      'origin': self.origin, 'upstream': self.bridge_url,
                      'tls': getattr(self, 'tls', None), 'checks': self.checks, 'metrics': self.metrics,
                      'browserCookieAttributes': getattr(self, 'browser_cookie', None),
                      'runtimePrivateState': str(self.runtime),
                      'privateKeysCredentialsAndCookieValuesPackaged': False,
                      'exitCode': self.process.returncode,
                      'productionExecutableModified': False, 'physicalRf': False}
            (self.output / 'real-proxy-results.json').write_text(json.dumps(report, indent=2) + '\n')
            assert self.process.returncode == 0, 'actual proxy did not shut down cleanly'
            assert self.checks[-2]['passed'] and self.checks[-1]['passed']
