"""T4 feature contracts and T5 live captions against the real NVR.

Every fixture is named with the run prefix and removed by the test or by
Suite.clear_fixtures, so these cases are safe on an existing instance (live.py).
"""

import json
import os
import pathlib
import shlex
import threading
import time
import urllib.error
import urllib.request

from suite import Skip

# SenseVoice's bundled sample says 开放时间早上9点至下午5点。 After AAC transport
# the first word varies (派饭/待饭时间), so match the stable time phrase.
SPEECH_WAV = (
    'third_party/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/test_wavs/zh.wav'
)
SPEECH_TEXT = '早上9点'


def request(s, path, method='GET', token=None, headers=None):
    """Raw status, content type and bytes; some routes are not BaseResponse JSON."""
    request = urllib.request.Request(s.base + path, method=method, headers=headers or {})
    if token is None:
        token = s.token
    if token:
        request.add_header('Authorization', 'Bearer ' + token)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        response = opener.open(request, timeout=30)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read()
        s.last = f'HTTP {response.code} {response.headers.get("Content-Type")} {len(body)} bytes'
        s.last_headers = response.headers
        return response.code, response.headers.get('Content-Type', ''), body


def as_user(s, token, fn):
    """Run fn with another session token, always restoring the admin session."""
    admin = s.token
    s.token = token
    try:
        return fn()
    finally:
        s.token = admin


def login_as(s, username, password, expected=200):
    return as_user(
        s,
        '',
        lambda: s.api(
            'POST', '/api/user/login', {'username': username, 'password': password}, expected
        ),
    )


def second_device(s, suffix):
    p = s.payload(suffix, input_value='testsrc=size=640x360:rate=25,realtime')
    s.add(p)
    s.wait_running(p['id'])
    return p


# Answers only the login exchange FTP targets use for their connectivity test, and
# accepts exactly one password, so a passing test proves which password was stored.
FTP_STUB = r"""
import socket, sys, time
base, password = sys.argv[1], sys.argv[2]
server = socket.socket()
server.bind(('127.0.0.1', 0))
server.listen(4)
server.settimeout(5)
open(base + '.port', 'w').write(str(server.getsockname()[1]))
deadline = time.time() + 180
while time.time() < deadline:
    try:
        conn, _ = server.accept()
    except OSError:
        continue
    conn.settimeout(10)
    stream = conn.makefile('rwb', 0)
    stream.write(b'220 e2e\r\n')
    try:
        for line in stream:
            verb, _, arg = line.decode().strip().partition(' ')
            verb = verb.upper()
            if verb == 'USER':
                stream.write(b'331 password\r\n')
            elif verb == 'PASS':
                stream.write(b'230 ok\r\n' if arg == password else b'530 denied\r\n')
            elif verb == 'QUIT':
                stream.write(b'221 bye\r\n')
                break
            else:
                stream.write(b'502 unsupported\r\n')
    except OSError:
        pass
    conn.close()
"""


def start_ftp_stub(s, password):
    """Start FTP_STUB beside the NVR (host network) and return (port, pid), or None."""
    if s.board('command -v python3 || true').strip() == '':
        return None
    base = f'/tmp/{s.prefix}-ftp'
    pid = s.board(
        f'nohup python3 -c {shlex.quote(FTP_STUB)} {base} {shlex.quote(password)} '
        '>/dev/null 2>&1 & echo $!'
    ).strip()
    for _ in range(20):
        port = s.board(f'cat {base}.port 2>/dev/null || true').strip()
        if port:
            s.board(f'rm -f {base}.port')
            return int(port), pid
        time.sleep(0.5)
    raise AssertionError('FTP stub did not start on the board')


def coverage_cases(s):
    def auth():
        admin = s.token
        as_user(s, '', lambda: s.api('GET', '/api/device/list', expected=401, raw=True))
        as_user(
            s, 'not-a-session', lambda: s.api('GET', '/api/device/list', expected=401, raw=True)
        )
        # hls.js/Safari playback authenticate with ?token= when headers are unavailable.
        devices = as_user(s, '', lambda: s.api('GET', '/api/device/list?token=' + admin))
        assert isinstance(devices, list), devices

    def users():
        username = s.prefix + '-user'
        s.users.append(username)
        s.api('POST', '/api/user/add', {'username': username, 'password': 'e2e-pass-1'})
        response = s.api(
            'POST', '/api/user/add', {'username': username, 'password': 'x'}, expected=500
        )
        assert response['message'] == 'User already exists', response
        session = login_as(s, username, 'e2e-pass-1')
        assert session['username'] == username, session
        info = as_user(s, session['token'], lambda: s.api('GET', '/api/user/info'))
        assert info['username'] == username, info
        response = as_user(
            s,
            session['token'],
            lambda: s.api(
                'POST',
                '/api/user/password',
                {'old_password': 'wrong', 'new_password': 'e2e-pass-2'},
                expected=500,
            ),
        )
        assert response['message'] == 'Old password is incorrect', response
        as_user(
            s,
            session['token'],
            lambda: s.api(
                'POST',
                '/api/user/password',
                {'old_password': 'e2e-pass-1', 'new_password': 'e2e-pass-2'},
            ),
        )
        response = login_as(s, username, 'e2e-pass-1', expected=500)
        assert response['message'] == 'Invalid username or password', response
        login_as(s, username, 'e2e-pass-2')
        me = s.api('GET', '/api/user/info')['username']
        response = s.api('POST', '/api/user/remove/' + me, expected=500)
        assert response['message'] == 'Cannot remove the currently logged-in user', response
        s.api('POST', '/api/user/remove/' + username)
        s.users.remove(username)
        assert all(u['username'] != username for u in s.api('GET', '/api/user/list'))
        # Removing a user revokes all of its sessions.
        as_user(s, session['token'], lambda: s.api('GET', '/api/user/info', expected=401, raw=True))

    def system():
        p = s.ensure_pattern()
        overview = s.api('GET', '/api/system/overview')
        total, online, offline = (
            overview['device_total'],
            overview['device_online'],
            overview['device_offline'],
        )
        assert total == online + offline == len(overview['devices']), overview
        item = next((d for d in overview['devices'] if d['id'] == p['id']), None)
        assert item and item['online'] is True, item
        metrics = s.api('GET', '/api/system/metrics')
        assert metrics['cpu_core_count'] > 0 and 0 < metrics['mem_used'] <= metrics['mem_total'], (
            metrics
        )
        settings = s.api('GET', '/api/system/settings')
        assert settings['player'] in ('mpegts', 'jessibuca', 'auto'), settings
        cleanup = s.api('GET', '/api/system/cleanup')
        assert isinstance(cleanup['enabled'], bool) and cleanup['max_age_days'] >= 0, cleanup

    def snapshot():
        p = s.ensure_pattern()
        code, kind, body = request(s, '/api/device/snapshot/' + p['id'])
        assert code == 200 and kind == 'image/jpeg', s.last
        assert body[:3] == b'\xff\xd8\xff', 'snapshot is not a JPEG'
        path = s.artifacts / 'snapshot.jpg'
        path.write_bytes(body)
        streams = json.loads(
            s.execute(['ffprobe', '-v', 'error', '-show_streams', '-of', 'json', str(path)])
        )['streams']
        assert (streams[0]['width'], streams[0]['height']) == (1920, 1080), streams
        code, _, _ = request(s, '/api/device/snapshot/' + s.prefix + '-absent')
        assert code == 500, s.last

    def program():
        a = s.ensure_pattern()
        b = second_device(s, 'program-b')
        id_ = s.prefix + '-program'
        s.programs.append(id_)
        sources = [
            dict(id='a', url=s.rtsp(a['id'], board=True)),
            dict(id='b', url=s.rtsp(b['id'], board=True)),
        ]
        data = s.api('POST', '/api/program/create', dict(id=id_, sources=sources))
        assert data['active'] == 'a', data
        # The program must publish to the instance's configured RTMP port.
        assert f':{os.environ["RK_RTMP_PORT"]}/' in data['publish_url'], data
        time.sleep(3)
        s.probe(s.rtsp(id_, 'switcher'), seconds=3)
        s.api('POST', '/api/program/switch/' + id_, {'to': 'b'})
        listed = next(p for p in s.api('GET', '/api/program/list') if p['id'] == id_)
        assert listed['active'] == 'b', listed
        s.probe(s.rtsp(id_, 'switcher'), seconds=3)
        response = s.api('POST', '/api/program/switch/' + id_, {'to': 'missing'}, expected=500)
        assert 'missing' in response['message'], response
        s.api('POST', '/api/program/remove/' + id_)
        s.programs.remove(id_)
        assert all(p['id'] != id_ for p in s.api('GET', '/api/program/list'))
        s.remove(b['id'])

    def compositor():
        a = s.ensure_pattern()
        b = second_device(s, 'mosaic-b')
        id_ = s.prefix + '-mosaic'
        s.compositors.append(id_)
        half = dict(y=0, w=320, h=360)
        s.api(
            'POST',
            '/api/compositor/create',
            dict(
                id=id_,
                sources=[
                    dict(id='a', url=s.rtsp(a['id'], board=True)),
                    dict(id='b', url=s.rtsp(b['id'], board=True)),
                ],
                width=640,
                height=360,
                fps=25,
                regions=[dict(source='a', x=0, **half), dict(source='b', x=320, **half)],
            ),
        )

        def current():
            return next(c for c in s.api('GET', '/api/compositor/list') if c['id'] == id_)

        time.sleep(3)
        s.probe(s.rtsp(id_, 'switcher'), seconds=3)
        s.api('POST', '/api/compositor/switch/' + id_, {'region': 0, 'to': 'b'})
        assert [r['source'] for r in current()['regions']] == ['b', 'b'], current()
        s.api(
            'POST',
            '/api/compositor/relayout/' + id_,
            {'regions': [dict(source='a', x=0, y=0, w=640, h=360)]},
        )
        assert [r['source'] for r in current()['regions']] == ['a'], current()
        data = s.api(
            'POST',
            '/api/compositor/add-source/' + id_,
            {'source_id': 'c', 'url': s.rtsp(b['id'], board=True)},
        )
        assert {src['id'] for src in data['sources']} == {'a', 'b', 'c'}, data
        data = s.api('POST', '/api/compositor/remove-source/' + id_, {'source_id': 'c'})
        assert {src['id'] for src in data['sources']} == {'a', 'b'}, data
        s.probe(s.rtsp(id_, 'switcher'), seconds=3)
        s.api('POST', '/api/compositor/remove/' + id_)
        s.compositors.remove(id_)
        s.remove(b['id'])

    def detection():
        capabilities = json.loads(s.api('GET', '/api/detect/capabilities', raw=True))
        for key in ('models', 'supported_input_types', 'max_sample_interval_ms', 'max_model_count'):
            assert key in capabilities, capabilities
        assert capabilities['max_model_count'] > 0, capabilities
        models = json.loads(s.api('GET', '/api/detect/models', raw=True))
        assert models == capabilities['models'], (models, capabilities)

    def llm():
        code, _, body = request(s, '/api/llm/health')
        health = json.loads(body)
        if code == 503:
            raise Skip('LLM not ready: ' + json.dumps(health, ensure_ascii=False))
        assert code == 200 and health['status'] == 'ready', health
        models = json.loads(s.api('GET', '/api/llm/v1/models', raw=True))
        assert [m['id'] for m in models['data']] == [health['model']], models
        error = json.loads(
            s.api('POST', '/api/llm/v1/chat/completions', {'messages': 1}, expected=400, raw=True)
        )
        assert error['error']['type'] == 'invalid_request_error', error
        prompt = {
            'messages': [{'role': 'user', 'content': '1+1 等于几？只回答数字。'}],
            'max_tokens': 64,
        }
        reply = json.loads(
            s.api('POST', '/api/llm/v1/chat/completions', prompt, raw=True, timeout=180)
        )
        message = reply['choices'][0]['message']
        text = (message.get('content') or '') + (message.get('reasoning_content') or '')
        assert text.strip(), reply

    def mixer():
        state = s.api('GET', '/api/audiomixer/list')
        if state['buses']:
            raise Skip('instance already has a mixer bus; the console allows only one')
        bus = s.prefix + '-mix'

        def create(inputs):
            return s.api(
                'POST', '/api/audiomixer/bus/create', dict(id=bus, inputs=inputs), expected=500
            )['message']

        assert 'at least one input' in create([]), s.last
        assert '音量不能超过 400%' in create([dict(source_id='alsa:x', volume=401)]), s.last
        assert '音频源不存在' in create([dict(source_id=s.prefix + '-absent')]), s.last
        for path, body in (
            ('/api/audiomixer/bus/remove', dict(bus_id=bus)),
            ('/api/audiomixer/bus/input/volume', dict(bus_id=bus, source_id='x', volume=50)),
            ('/api/audiomixer/bus/input/mute', dict(bus_id=bus, source_id='x', muted=True)),
        ):
            s.api('POST', path, body, expected=500)
        assert s.api('GET', '/api/audiomixer/list')['buses'] == [], s.last
        sources = s.api('GET', '/api/audiomixer/sources')
        if not sources:
            raise Skip('no ALSA capture input visible to the NVR; only validation was tested')
        source = sources[0]['id']
        s.mixers.append(bus)
        data = s.api(
            'POST', '/api/audiomixer/bus/create', dict(id=bus, inputs=[dict(source_id=source)])
        )
        created = next(b for b in data['buses'] if b['id'] == bus)
        assert f':{os.environ["RK_RTMP_PORT"]}/mixer/{bus}' in created['publish_url'], created
        assert created['flv_url'] == f'/media/mixer/{bus}.live.flv', created
        s.api(
            'POST',
            '/api/audiomixer/bus/input/volume',
            dict(bus_id=bus, source_id=source, volume=50),
        )
        s.api(
            'POST', '/api/audiomixer/bus/input/mute', dict(bus_id=bus, source_id=source, muted=True)
        )
        bus_state = next(b for b in s.api('GET', '/api/audiomixer/list')['buses'] if b['id'] == bus)
        assert [(i['source_id'], i['volume'], i['muted']) for i in bus_state['inputs']] == [
            (source, 50, True)
        ], bus_state
        time.sleep(3)
        streams = json.loads(
            s.execute(
                ['ffprobe', '-v', 'error', '-rtsp_transport', 'tcp', '-show_streams', '-of', 'json']
                + [s.rtsp(bus, 'mixer')],
                timeout=30,
            )
        )['streams']
        assert any(st['codec_type'] == 'audio' for st in streams), streams
        s.api('POST', '/api/audiomixer/bus/remove', dict(bus_id=bus))
        s.mixers.remove(bus)

    def transport():
        capabilities = s.api('GET', '/api/transport/capabilities')
        assert isinstance(capabilities['smb_enabled'], bool), capabilities
        password = 'e2e-secret-' + s.prefix[-8:]
        stub = start_ftp_stub(s, password)
        try:
            config = dict(
                host='127.0.0.1',
                port=stub[0] if stub else 1,
                username='e2e',
                password=password,
                base_path='e2e',
                # Disabled and routed to no real device: the target never uploads recordings.
                stream_ids=[s.prefix + '-none'],
            )
            target = dict(name=s.prefix + '-ftp', kind='ftp', enabled=False, config=config)
            for bad in (
                dict(target, kind='nfs'),
                dict(target, name=' '),
                dict(target, config=dict(config, host='')),
                dict(target, config=dict(config, stream_ids=[' '])),
            ):
                s.api('POST', '/api/transport/target/add', bad, expected=500)
            if not capabilities['smb_enabled']:
                smb = dict(target, kind='smb', enabled=True, config=dict(config, share='e2e'))
                message = s.api('POST', '/api/transport/target/add', smb, expected=500)['message']
                assert 'SMB' in message, message
            added = s.api('POST', '/api/transport/target/add', target)
            id_ = added['id']
            s.transports.append(id_)
            assert added['config']['password'] == '', 'password returned in add response'
            listed = next(t for t in s.api('GET', '/api/transport/targets') if t['id'] == id_)
            assert listed['config']['password'] == '', 'password returned in target list'
            assert (listed['enabled'], listed['done'], listed['failed'], listed['pending']) == (
                False,
                0,
                0,
                0,
            ), listed
            assert s.api('GET', '/api/transport/jobs/' + id_) == [], s.last
            if stub:
                s.api('POST', '/api/transport/target/test/' + id_)
                # A blank password (what the dashboard round-trips) keeps the stored one.
                renamed = dict(target, name=target['name'] + '-2', config=dict(config, password=''))
                updated = s.api('POST', '/api/transport/target/update/' + id_, renamed)
                assert updated['name'] == renamed['name'], updated
                s.api('POST', '/api/transport/target/test/' + id_)
                wrong = dict(target, config=dict(config, password='wrong'))
                s.api('POST', '/api/transport/target/update/' + id_, wrong)
                message = s.api('POST', '/api/transport/target/test/' + id_, expected=500)[
                    'message'
                ]
                assert 'login' in message, message
            else:
                message = s.api('POST', '/api/transport/target/test/' + id_, expected=500)[
                    'message'
                ]
                assert 'connect ftp' in message, message
            s.api('POST', '/api/transport/target/remove/' + id_)
            s.transports.remove(id_)
            assert all(t['id'] != id_ for t in s.api('GET', '/api/transport/targets'))
            s.api('POST', '/api/transport/target/test/' + id_, expected=500)
        finally:
            if stub:
                s.board(f'kill {int(stub[1])} 2>/dev/null || true')
        if not stub:
            raise Skip('python3 missing on the board; FTP login and password retention untested')

    def playback():
        p = s.payload('playback', input_value='testsrc2=size=640x360:rate=25,realtime', record=True)
        s.add(p)
        try:
            s.wait_running(p['id'])
            deadline = time.monotonic() + int(os.environ.get('RK_RECORD_TIMEOUT', '210'))
            segments = []
            while time.monotonic() < deadline:
                data = s.api('GET', '/api/playback/device/' + p['id'] + '/segments?page_size=100')
                segments = sorted(data['items'], key=lambda item: item['start_time'])
                if len(segments) >= 2:
                    break
                s.check_boot()
                time.sleep(3)
            assert len(segments) >= 2, f'expected >=2 recording segments, actual {len(segments)}'
            first, second = segments[:2]
            devices = s.api('GET', '/api/playback/device/list?page_size=100')['items']
            item = next((d for d in devices if d['device_id'] == p['id']), None)
            assert item and item['segment_count'] >= 2, item
            today = {seg['id'] for seg in s.api('GET', f'/api/playback/device/{p["id"]}/today')}
            assert {first['id'], second['id']} <= today, today

            code, kind, body = request(s, f'/api/playback/playlist/{p["id"]}?token={s.token}')
            text = body.decode()
            assert code == 200 and kind == 'application/vnd.apple.mpegurl', s.last
            assert '#EXT-X-DISCONTINUITY' in text and text.rstrip().endswith('#EXT-X-ENDLIST'), text
            for seg in (first, second):
                assert f'/api/playback/segment/{seg["id"]}?token={s.token}' in text, text
            code, _, _ = request(s, f'/api/playback/playlist/{s.prefix}-absent')
            assert code == 500, s.last

            path = '/api/playback/segment/' + first['id']
            code, kind, body = request(s, path, headers={'Range': 'bytes=0-187'})
            assert (code, kind, len(body)) == (206, 'video/mp2t', 188), s.last
            assert body[0] == 0x47, 'segment does not start with a TS sync byte'
            assert s.last_headers['Content-Range'] == f'bytes 0-187/{first["file_size"]}', (
                s.last_headers['Content-Range'],
                first,
            )
            code, _, _ = request(s, path, headers={'Range': f'bytes={first["file_size"]}-'})
            assert code == 416, s.last

            deleted = s.api('POST', f'/api/playback/segment/{first["id"]}/delete')
            assert deleted['deleted'] == 1, deleted
            code, _, _ = request(s, path)
            assert code == 500, s.last
            deleted = s.api(
                'POST', '/api/playback/segments/delete', {'ids': [second['id'], s.prefix + '-x']}
            )
            assert deleted['deleted'] == 1, deleted
            left = s.api('GET', '/api/playback/device/' + p['id'] + '/segments?page_size=100')
            assert not {first['id'], second['id']} & {i['id'] for i in left['items']}, left
        finally:
            if p['id'] in s.devices:
                s.remove(p['id'])
            s.api('POST', f'/api/playback/device/{p["id"]}/segments/delete')

    cases = [auth, users, system, snapshot, program, compositor, detection, llm]
    for i, fn in enumerate(cases + [mixer, transport, playback], 1):
        s.test(f'T4.{i}', 'coverage', fn)


def speech_sample(s):
    """Board path of the speech sample: the install's copy, else upload the local one."""
    installed = s.root + '/' + SPEECH_WAV
    if (
        s.board(s.inside(f'test -f {shlex.quote(installed)} && echo yes || echo no')).strip()
        == 'yes'
    ):
        return installed
    local = pathlib.Path(__file__).resolve().parents[3] / SPEECH_WAV
    if not local.is_file():
        raise Skip(f'ASR sample missing on the board ({installed}) and locally ({local})')
    target = s.remote + '/results/speech.wav'
    scp = json.loads(os.environ['E2E_SCP_JSON'])
    if not s.container:
        s.execute(scp + [str(local), os.environ['RK_DEST'] + ':' + target], timeout=60)
        return target
    staged = f'/tmp/{s.prefix}-speech.wav'
    s.execute(scp + [str(local), os.environ['RK_DEST'] + ':' + staged], timeout=60)
    s.board(
        shlex.join(['docker', 'cp', staged, f'{s.container}:{target}'])
        + f'; status=$?; rm -f {shlex.quote(staged)}; exit $status'
    )
    return target


def asr_cases(s):
    def captions():
        try:
            import requests
            import socketio
        except ImportError as error:
            raise Skip(
                'python-socketio missing; pip install --target .cache/rockchip-e2e-python '
                '"python-socketio[client]"'
            ) from error
        wav = speech_sample(s)
        response = s.api('POST', '/api/asr/' + s.prefix + '-absent/start', expected=404, raw=True)
        assert response == 'pipe not found', response
        out = s.remote + '/results/speech'
        s.script('source', out, os.environ['RK_RTSP_PORT'], 'speech', wav, timeout=90)
        s.speech_started = True
        p = s.payload(
            'asr',
            input_type='rtsp',
            input_value=s.rtsp('speech', 'e2e', board=True),
            include_audio=True,
        )
        s.add(p)
        s.wait_running(p['id'])

        events = []
        received = threading.Event()
        # Like Suite.api, never route board traffic through a developer proxy. websocket-client
        # reads proxy variables on its own, so stay on polling; captions, not transport, are tested.
        session = requests.Session()
        session.trust_env = False
        client = socketio.Client(reconnection=False, http_session=session)

        def on(kind):
            def handler(data):
                events.append((kind, data))
                if kind == 'final' and SPEECH_TEXT in data.get('text', ''):
                    received.set()

            return handler

        client.on('partial', on('partial'), namespace='/asr')
        client.on('final', on('final'), namespace='/asr')
        client.connect(s.base, namespaces=['/asr'], transports=['polling'], wait_timeout=10)
        try:
            client.emit('subscribe', p['id'], namespace='/asr')
            code, _, body = request(s, '/api/asr/' + p['id'] + '/start', 'POST')
            status = body.decode()
            if code == 503 or (code == 500 and status.startswith('model load failed')):
                raise Skip(f'ASR unavailable on this instance: HTTP {code} {status}')
            assert (code, status) == (200, 'started'), (code, status)
            ok = received.wait(60)
            (s.artifacts / 'asr-events.json').write_text(
                json.dumps(events, ensure_ascii=False, indent=2)
            )
            assert ok, f'no final caption containing {SPEECH_TEXT} in 60s; events={events[-5:]}'
            assert any(kind == 'partial' for kind, _ in events), 'no partial caption before final'
            final = next(d for k, d in events if k == 'final' and SPEECH_TEXT in d['text'])
            assert final['pipe'] == p['id'] and final['final'] is True, final
            assert final['duration'] > 0 and final['sentences'], final
            ids = [d['id'] for k, d in events if k == 'final']
            assert ids == sorted(ids), f'final segment ids out of order: {ids}'
        finally:
            stopped = s.api('POST', '/api/asr/' + p['id'] + '/stop', raw=True)
            client.disconnect()
        assert stopped == 'stopped', stopped
        assert s.api('POST', '/api/asr/' + p['id'] + '/stop', raw=True) == 'not running'
        s.remove(p['id'])
        s.script('source', out, os.environ['RK_RTSP_PORT'], 'stop')
        s.speech_started = False

    s.test('T5.1', 'asr', captions)
