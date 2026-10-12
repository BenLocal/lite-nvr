#!/usr/bin/env python3
"""Real-board tests. Every assertion reports independently; no mocked success."""

import json
import os
import pathlib
import re
import shlex
import subprocess
import sys
import time
import urllib.error
import urllib.request

if not __debug__:
    raise SystemExit('E2E assertions require Python without -O/PYTHONOPTIMIZE')

HERE = pathlib.Path(__file__).resolve().parent
CONTRACT = json.loads((HERE.parent / 'expected/contracts.json').read_text())


class Skip(Exception):
    pass


class EnvironmentFailure(Exception):
    pass


class Restart(EnvironmentFailure):
    pass


class BoardUnavailable(EnvironmentFailure):
    pass


class CommandFailure(AssertionError):
    def __init__(self, returncode, message):
        super().__init__(message)
        self.returncode = returncode


class Suite:
    def __init__(self, selected=None):
        self.base = os.environ['BASE']
        self.ssh = json.loads(os.environ['E2E_SSH_JSON'])
        self.root = os.environ['INSTALL_DIR']
        self.remote = os.environ['REMOTE_ROOT']
        self.artifacts = pathlib.Path(os.environ['ARTIFACTS'])
        self.token = ''
        self.last = ''
        self.selected = selected
        self.results = []
        self.devices = []
        self.compositors = []
        self.unavailable = False
        self.restarts = 0
        self.uptime_start = None
        self.uptime_end = None
        self.boot = self.board('cat /proc/sys/kernel/random/boot_id').strip()
        self.uptime_start = self.board('cat /proc/uptime').split()[0]
        self.prefix = 'e2e-' + os.environ['RUN_ID']
        self.pattern = None
        self.source_started = False

    def execute(self, args, timeout=30):
        env = dict(os.environ)
        if args[0] in ('ffprobe', 'ffmpeg'):
            for key in (
                'http_proxy',
                'https_proxy',
                'all_proxy',
                'HTTP_PROXY',
                'HTTPS_PROXY',
                'ALL_PROXY',
            ):
                env.pop(key, None)
        result = subprocess.run(args, capture_output=True, text=True, timeout=timeout, env=env)
        self.last = result.stdout + result.stderr
        if result.returncode:
            raise CommandFailure(
                result.returncode,
                f'command exit={result.returncode}: {self.redact(self.last)[-2000:]}',
            )
        return result.stdout

    def board(self, command, timeout=30):
        try:
            return self.execute(self.ssh + [command], timeout)
        except CommandFailure as error:
            if error.returncode == 255:
                self.unavailable = True
                raise BoardUnavailable('SSH disconnected; remaining board tests aborted') from error
            raise

    def script(self, name, *args, timeout=60):
        command = [
            'timeout',
            str(timeout - 5),
            'sh',
            f'{self.remote}/board/{name}.sh',
            self.root,
            *map(str, args),
        ]
        return self.board(shlex.join(command) + ' 2>&1', timeout)

    def redact(self, text):
        if self.token:
            text = text.replace(self.token, '<redacted>')
        return re.sub(r'("(?:token|password)"\s*:\s*")[^"]*', r'\1<redacted>', text)

    def api(self, method, path, payload=None, expected=200, raw=False, timeout=15):
        headers = {'Content-Type': 'application/json'}
        if self.token:
            headers['Authorization'] = 'Bearer ' + self.token
        data = json.dumps(payload).encode() if payload is not None else None
        request = urllib.request.Request(
            self.base + path, data=data, headers=headers, method=method
        )
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            response = opener.open(request, timeout=timeout)
        except urllib.error.HTTPError as error:
            response = error
        except urllib.error.URLError as error:
            raise AssertionError('HTTP request failed: ' + str(error.reason)) from error
        with response:
            body, code = response.read().decode(), response.code
        self.last = body + '\nHTTP ' + str(code)
        assert code == expected, (
            f'expected HTTP {expected}, actual {code}: {self.redact(body)[:500]}'
        )
        if raw:
            return body
        response = json.loads(body)
        assert response['code'] == (0 if expected == 200 else expected), self.redact(body)[:500]
        return response.get('data') if expected == 200 else response

    def login(self):
        username = os.environ.get('RK_API_USER', 'admin')
        password = os.environ.get('RK_API_PASSWORD', 'admin')
        data = self.api('POST', '/api/user/login', {'username': username, 'password': password})
        self.token = data['token']
        assert self.token, 'empty login token'

    def payload(self, suffix, **overrides):
        value = dict(
            id=self.prefix + '-' + suffix,
            name=self.prefix + '-' + suffix,
            input_type='lavfi',
            input_value='testsrc2=size=1920x1080:rate=25,realtime',
            description='Rockchip e2e fixture',
            record=False,
            include_audio=False,
            config={},
        )
        value.update(overrides)
        return value

    def add(self, payload):
        self.devices.append(payload['id'])  # Cleanup even when the request times out.
        return self.api('POST', '/api/device/add', payload)

    def update(self, payload):
        return self.api('POST', '/api/device/update/' + payload['id'], payload)

    def remove(self, id_):
        self.api('POST', '/api/device/remove/' + id_)
        if id_ in self.devices:
            self.devices.remove(id_)

    def list(self):
        return self.api('GET', '/api/device/list')

    def get(self, id_):
        return next(d for d in self.list() if d['id'] == id_)

    def stored(self, id_):
        return {
            key: value for key, value in self.get(id_).items() if key not in ('state', 'flv_url')
        }

    def wait_running(self, id_, seconds=45):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            metric = self.api('GET', '/api/device/metrics/' + id_)
            if (
                metric['state'] == 'running'
                and metric['distribution']
                and metric['distribution']['schema'] == 'rtsp'
            ):
                return
            self.check_boot()
            time.sleep(1)
        state = self.api('GET', '/api/device/metrics/' + id_)['state']
        raise AssertionError(f'{id_}: expected running within {seconds}s, actual {state}')

    def rtsp(self, id_, app='device', board=False):
        host = '127.0.0.1' if board else os.environ['MEDIA_HOST']
        port = os.environ['RK_RTSP_PORT'] if board else os.environ['MEDIA_PORT']
        return f'rtsp://{host}:{port}/{app}/{id_}'

    def probe(self, url, rtsp=True, seconds=10):
        args = ['ffprobe', '-v', 'error']
        if rtsp:
            args += ['-rtsp_transport', 'tcp']
        args += [
            '-read_intervals',
            f'%+{seconds}',
            '-count_frames',
            '-show_streams',
            '-of',
            'json',
            url,
        ]
        data = json.loads(self.execute(args, timeout=seconds + 30))
        self.last_probe_streams = data['streams']
        video = next((s for s in data['streams'] if s['codec_type'] == 'video'), None)
        assert video is not None, f'expected decoded video track, actual streams={data["streams"]}'
        assert int(video.get('nb_read_frames', '0')) >= max(1, seconds * 20), (
            f'expected decoded frames >= {seconds * 20}, actual {video}'
        )
        return video

    def ensure_pattern(self):
        if self.pattern is None:
            self.pattern = self.payload('pattern')
            self.add(self.pattern)
        self.wait_running(self.pattern['id'])
        return dict(self.pattern)

    def check_boot(self):
        boot = self.board('cat /proc/sys/kernel/random/boot_id').strip()
        if boot != self.boot:
            self.unavailable = True
            self.restarts += 1
            raise Restart(f'board restarted: boot_id {self.boot} -> {boot}')

    def test(self, id_, commit, function, check_board=True):
        if self.selected and id_.startswith(('T1.', 'T2.', 'T3.')) and id_ != self.selected:
            return True
        self.last = ''
        start = time.monotonic()
        status, detail = 'PASS', ''
        fatal = None
        try:
            if check_board:
                self.check_boot()
            function()
            if check_board:
                self.check_boot()
        except Skip as error:
            status, detail = 'SKIP', str(error)
        except Exception as error:
            status, detail = (
                'FAIL',
                str(error) or f'{id_}: assertion failed; see actual response below',
            )
            if isinstance(error, EnvironmentFailure):
                fatal = error
                detail = (
                    'BOARD RESTART: ' if isinstance(error, Restart) else 'BOARD UNAVAILABLE: '
                ) + detail
        result = dict(
            id=id_,
            commit=commit,
            status=status,
            detail=self.redact(detail),
            duration_s=round(time.monotonic() - start, 3),
        )
        if status == 'FAIL':
            response = self.redact(self.last)[:500]
            try:
                tail = self.redact(self.board(f'tail -n 20 {shlex.quote(self.root + "/nvr.log")}'))
            except Exception as error:
                tail = 'log unavailable: ' + str(error)
            result.update(response=response, log_tail=tail)
        self.results.append(result)
        number = len(self.results)
        print(
            f'{"not ok" if status == "FAIL" else "ok"} {number} - {id_} commit={commit}'
            + (f' # SKIP {detail}' if status == 'SKIP' else ''),
            flush=True,
        )
        if status == 'FAIL':
            for line in (
                detail + '\nresponse: ' + result['response'] + '\nnvr.log:\n' + result['log_tail']
            ).splitlines():
                print('# ' + line, flush=True)
        self.write_report()
        if fatal is not None:
            raise fatal
        return status != 'FAIL'

    def write_report(self):
        report = dict(
            commit=os.environ['COMMIT'],
            run_id=os.environ.get('RUN_ID'),
            boot_id=self.boot,
            base=self.base,
            selected=self.selected,
            install_dir=self.root,
            remote_root=self.remote,
            uptime_start_s=self.uptime_start,
            uptime_end_s=self.uptime_end,
            board_restarts_observed=self.restarts,
            results=self.results,
            summary={
                s: sum(r['status'] == s for r in self.results) for s in ['PASS', 'FAIL', 'SKIP']
            },
        )
        (self.artifacts / 'report.json').write_text(
            json.dumps(report, indent=2, ensure_ascii=False)
        )

    def baseline(self):
        def health():
            status = self.script('health', 'status')
            assert 'pid=' in status and 'NOT listening' not in status, status
            self.api('GET', '/nvr/', raw=True)

        def libs():
            output = self.script('health', 'libs')
            assert 'not found' not in output, output

        def glibc():
            # Read ELF version requirements on the host; BusyBox strings/sort are not prerequisites.
            local = self.artifacts / 'nvr'
            self.execute(
                json.loads(os.environ['E2E_SCP_JSON'])
                + [os.environ['RK_DEST'] + ':' + self.root + '/bin/nvr', str(local)],
                timeout=120,
            )
            data = self.execute(['readelf', '--version-info', str(local)])
            needed = max((int(a), int(b)) for a, b in re.findall(r'Name: GLIBC_(\d+)\.(\d+)', data))
            version = re.search(r'(\d+)\.(\d+)', self.script('health', 'libc'))
            assert version, 'cannot determine board glibc'
            actual = tuple(map(int, version.groups()))
            assert needed <= actual, f'GLIBC required={needed}, board={actual}'
            self.last = f'GLIBC required={needed}, board={actual}'

        def auth():
            self.login()
            self.api(
                'POST',
                '/api/user/login',
                {
                    'username': os.environ.get('RK_API_USER', 'admin'),
                    'password': 'e2e-deliberately-wrong',
                },
                expected=500,
            )

        def os_info():
            data = self.api('GET', '/api/system/os')
            assert (
                data['arch'] == 'aarch64'
                and 'RK3588' in data['board_model']
                and data['cpu_core_count'] == 8
            ), data
            assert 'Buildroot' in (data.get('os_name') or '') or 'Buildroot' in (
                data.get('long_os_version') or ''
            ), data
            version = re.search(
                r'^version = "([^"]+)"', (HERE.parents[2] / 'nvr/Cargo.toml').read_text(), re.M
            ).group(1)
            assert data['nvr_version'] == version, data

        def codecs():
            output = self.script('health', 'codecs')
            assert output.count('h264_rkmpp') >= 2 and output.count('hevc_rkmpp') >= 2, output

        for id_, fn in [
            ('T0.1', health),
            ('T0.2', libs),
            ('T0.3', glibc),
            ('T0.4', auth),
            ('T0.5', os_info),
            ('T0.6', codecs),
        ]:
            if not self.test(id_, 'f27cbc7', fn):
                return False
        return True

    def x11(self):
        start = time.monotonic()
        data = self.api('GET', '/api/system/list/x11grab/devices')
        elapsed = time.monotonic() - start
        if data['available']:
            raise Skip('X11 is available; unavailable-X11 fixture does not apply')
        assert data['reason'] and data['displays'] == [], data
        candidates = self.board('printf "%s" "${DISPLAY:-}"; ls /tmp/.X11-unix 2>/dev/null || true')
        if not candidates.strip():
            assert elapsed < 2, f'X11 query expected <2s without candidates, actual {elapsed:.3f}s'
        else:
            raise Skip('Stale DISPLAY/socket candidates: 2s fast-path assertion does not apply')

    def x11_reject(self):
        data = self.api('GET', '/api/system/list/x11grab/devices')
        if data['available']:
            raise Skip('X11 unavailable fixture not applicable')
        p = self.payload('x11', input_type='x11grab', input_value=':987')
        self.devices.append(p['id'])
        response = self.api('POST', '/api/device/add', p, expected=500)
        assert 'X11' in response['message'], response
        assert all(d['id'] != p['id'] for d in self.list()), 'rejected X11 device persisted'
        p = self.payload('x11-update')
        self.add(p)
        old = self.stored(p['id'])
        response = self.api(
            'POST',
            '/api/device/update/' + p['id'],
            dict(p, input_type='x11grab', input_value=':987'),
            expected=500,
        )
        assert 'X11' in response['message'], response
        assert self.stored(p['id']) == old, 'X11 rejection changed stored config'
        self.remove(p['id'])

    def api_cases(self):
        def crud():
            p = self.payload('crud')
            self.add(p)
            assert self.get(p['id'])['name'] == p['name']
            p['description'] = 'updated description'
            self.update(p)
            assert self.get(p['id'])['description'] == p['description']
            self.remove(p['id'])
            assert all(d['id'] != p['id'] for d in self.list())

        def state():
            p = self.payload('state')
            self.add(p)
            data = self.get(p['id'])
            assert data['state'] in CONTRACT['states'] and isinstance(data['flv_url'], str), data
            self.remove(p['id'])

        def normalize():
            for i, output in enumerate([{}, {'width': None, 'video_codec': None}]):
                p = self.payload('normalize' + str(i), config={'output': output})
                data = self.add(p)
                assert 'output' not in data['config'], data
                assert 'output' not in self.get(p['id'])['config']
                self.remove(p['id'])

        def invalid():
            for i, (output, message) in enumerate(CONTRACT['errors']):
                p = self.payload('invalid' + str(i), config={'output': output})
                response = self.api('POST', '/api/device/add', p, expected=500)
                assert response['message'] == message, response
                assert all(d['id'] != p['id'] for d in self.list()), 'invalid device persisted'

        def native():
            for type_ in ['gb28181', 'xiaomi']:
                p = self.payload(
                    'native-' + type_, input_type=type_, config={'output': {'fps': 25}}
                )
                response = self.api('POST', '/api/device/add', p, expected=500)
                assert response['message'] == '该设备使用原生转发，暂不支持输出转码参数', response

        def unchanged():
            p = self.payload('unchanged')
            self.add(p)
            old = self.stored(p['id'])
            p['config'] = {'output': {'fps': 0}}
            self.api('POST', '/api/device/update/' + p['id'], p, expected=500)
            assert self.stored(p['id']) == old, 'rejected update changed stored device'
            self.remove(p['id'])

        def metrics():
            p = self.payload('metrics')
            self.add(p)
            self.wait_running(p['id'])
            deadline = time.monotonic() + 10
            while True:
                data = self.api('GET', '/api/device/metrics/' + p['id'])
                if data['pipeline'] is not None and data['distribution'] is not None:
                    break
                assert time.monotonic() < deadline, 'running metrics did not become available'
                time.sleep(1)
            assert set(CONTRACT['metrics_fields']) <= data.keys(), data
            assert data['state'] in CONTRACT['states'] and isinstance(data['sampled_at_ms'], int), (
                data
            )
            assert set(CONTRACT['pipeline_fields']) <= data['pipeline'].keys(), data
            assert data['pipeline']['output'], data
            assert all('traffic' in out['stream'] for out in data['pipeline']['output']), data
            assert 'tracks' in data['distribution'], data
            listed = self.api('GET', '/api/device/metrics')
            assert any(item['id'] == p['id'] for item in listed), listed
            self.remove(p['id'])

        def absent():
            response = self.api(
                'GET', '/api/device/metrics/' + self.prefix + '-absent', expected=500
            )
            assert response['message'] == 'device not found', response

        def rollback():
            if os.environ.get('RK_FAULT_APPLICATION') != '1':
                raise Skip(
                    'Post-persist failure requires --mutate drop-rollback; validation rejection is T1.6'
                )
            p = self.payload('rollback')
            self.add(p)
            self.wait_running(p['id'])
            old = self.stored(p['id'])
            p['description'] = 'e2e-injected-apply-failure'
            response = self.api('POST', '/api/device/update/' + p['id'], p, expected=500)
            assert response['message'] == 'e2e injected apply failure', response
            self.wait_running(p['id'])
            actual = self.stored(p['id'])
            assert actual == old, f'expected restored device {old}, actual {actual}'
            self.probe(self.rtsp(p['id']), seconds=2)
            self.remove(p['id'])

        for i, fn in enumerate(
            [crud, state, normalize, invalid, native, unchanged, metrics, absent], 1
        ):
            self.test(f'T1.{i}', '23217e1', fn)
        self.test('T1.9', '23217e1', rollback)

    def media_cases(self):
        from media import media_cases

        media_cases(self)

    def hardware_cases(self):
        from hardware import hardware_cases

        hardware_cases(self)

    def clear_fixtures(self):
        errors = []
        if self.source_started:
            try:
                self.script('source', self.remote + '/results', os.environ['RK_RTSP_PORT'], 'stop')
                self.source_started = False
            except Exception as error:
                errors.append(str(error))
        for id_ in self.compositors[:]:
            try:
                self.api('POST', '/api/compositor/remove/' + id_)
                self.compositors.remove(id_)
            except Exception as error:
                errors.append(str(error))
        for id_ in self.devices[:]:
            try:
                self.remove(id_)
            except Exception as error:
                errors.append(str(error))
        if errors:
            raise AssertionError('fixture cleanup failed: ' + '; '.join(errors))

    def cleanup(self):
        if self.unavailable:
            raise Skip('Board restarted/disconnected; isolated fixture directory retained')
        self.clear_fixtures()
        if self.token:
            try:
                self.api('POST', '/api/user/logout')
            except Exception as error:
                raise AssertionError('logout failed: ' + str(error)) from error


def main(requested='all'):
    suite = Suite(requested if '.' in requested else None)
    print('TAP version 13', flush=True)
    try:
        if suite.baseline():
            group = requested.split('.')[0]
            if group != '--baseline':
                if suite.test('T0.7', 'b33e7d7', suite.x11) and suite.test(
                    'T0.8', 'b33e7d7', suite.x11_reject
                ):
                    if group in ('all', 'T1'):
                        suite.api_cases()
                    if group in ('all', 'T2'):
                        suite.media_cases()
                    if group in ('all', 'T3'):
                        suite.hardware_cases()
    except EnvironmentFailure as error:
        print('# ' + str(error), flush=True)
    finally:
        if not suite.unavailable:
            try:
                suite.uptime_end = suite.board('cat /proc/uptime').split()[0]
            except EnvironmentFailure:
                pass  # Cleanup records the unavailable board; preserve previous results.
        suite.test('cleanup', 'baseline', suite.cleanup, check_board=False)
        suite.write_report()
        print(f'1..{len(suite.results)}', flush=True)
        summary = {
            s: sum(r['status'] == s for r in suite.results) for s in ['PASS', 'FAIL', 'SKIP']
        }
        print('# ' + ' '.join(f'{s} {n}' for s, n in summary.items()), flush=True)
    return int(any(r['status'] == 'FAIL' for r in suite.results))


if __name__ == '__main__':
    # Helpers import suite.Skip; keep that identity when run as a script.
    sys.modules['suite'] = sys.modules[__name__]
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else 'all'))
