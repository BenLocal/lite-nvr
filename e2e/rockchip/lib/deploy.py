"""Host-side build, isolated deployment, transport and lifecycle ownership."""

import datetime
import hashlib
import json
import os
import pathlib
import re
import shlex
import shutil
import signal
import socket
import stat
import subprocess
import time
import urllib.error
import urllib.request
import uuid

from provenance import source_digest


class Runner:
    def __init__(self, here):
        self.here = pathlib.Path(here)
        self.repo = self.here.parents[1]
        self.host = os.environ.get('RK_HOST')
        self.user = os.environ.get('RK_USER')
        if not self.host or not self.user:
            raise ValueError('Set RK_HOST and RK_USER (see ignored dev-env.local.md)')
        self.target = f'{self.user}@{self.host}'
        self.rtsp_port = self.port('RK_RTSP_PORT', 8554)
        self.rtmp_port = self.port('RK_RTMP_PORT', 8555)
        if self.rtsp_port == self.rtmp_port:
            raise ValueError('RTSP and RTMP ports must differ')
        self.run_id = (
            datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
            + '-'
            + uuid.uuid4().hex[:8]
        )
        self.artifacts = pathlib.Path(
            os.environ.get('RK_ARTIFACT_DIR', self.repo / 'third_party/rockchip-e2e' / self.run_id)
        ).resolve()
        self.parent_artifacts = self.artifacts
        self.cache = self.repo / '.cache'
        self.lock = self.cache / 'rockchip-e2e-python.lock'
        self.lock_owned = False
        self.install = None
        self.remote = None
        self.tunnel = None
        self.mutation_file = None
        self.mutation_backup = None
        self.mutation_original = None
        self.mutation_expected = None
        self.phase = 'T0.setup'
        self.commit = 'unknown'
        self.test_digest = self.suite_digest()
        prefix = []
        password = os.environ.get('RK_PASSWORD_FILE')
        if password:
            password = pathlib.Path(password).resolve()
            if not password.is_file():
                raise ValueError('SSH password file is unreadable')
            if os.name == 'posix' and stat.S_IMODE(password.stat().st_mode) & 0o077:
                raise ValueError('SSH password file must have permissions 0600')
            prefix = ['sshpass', '-f', str(password)]
        options = [
            '-F',
            os.environ.get('RK_SSH_CONFIG', os.devnull),
            '-o',
            'ConnectTimeout=5',
            '-o',
            'ServerAliveInterval=5',
            '-o',
            'ServerAliveCountMax=3',
            '-o',
            'StrictHostKeyChecking=accept-new',
            '-o',
            f'UserKnownHostsFile={self.cache / "rockchip-e2e-known_hosts"}',
        ]
        self.ssh = prefix + ['ssh', *options, self.target]
        self.scp = prefix + ['scp', '-O', *options]

    def suite_digest(self):
        digest = hashlib.sha256()
        for path in sorted(self.here.rglob('*')):
            if path.is_file() and path.suffix in ('.py', '.sh', '.json'):
                digest.update(str(path.relative_to(self.here)).encode() + b'\0')
                digest.update(path.read_bytes())
        return digest.hexdigest()

    def check_suite_unchanged(self):
        if self.suite_digest() != self.test_digest:
            raise RuntimeError(
                'E2E code changed during the run; results are from mixed versions and cannot be accepted'
            )

    @staticmethod
    def port(name, default):
        text = os.environ.get(name, str(default))
        if not re.fullmatch(r'[1-9][0-9]{0,4}', text):
            raise ValueError(f'Invalid {name}')
        value = int(text)
        if value > 65535 or value in (18080, 8553):
            raise ValueError(f'Invalid or reserved {name}')
        return value

    def command(self, args, log=None, timeout=30, env=None):
        output = pathlib.Path(log).open('w') if log else None
        process = None
        try:
            process = subprocess.Popen(
                args,
                stdout=output if output else subprocess.PIPE,
                stderr=subprocess.STDOUT if output else subprocess.PIPE,
                text=True,
                env=env,
                start_new_session=os.name == 'posix',
            )
            try:
                stdout, stderr = process.communicate(timeout=timeout)
            except BaseException:
                # Kill only this command's process group, giving the Rockchip
                # wrapper time to restore its temporary Cargo files.
                if process.poll() is None:
                    if os.name == 'posix':
                        os.killpg(process.pid, signal.SIGTERM)
                    else:
                        process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        if os.name == 'posix':
                            os.killpg(process.pid, signal.SIGKILL)
                        else:
                            process.kill()
                        process.wait()
                raise
            if process.returncode:
                detail = f'inspect {log}' if log else (stderr or '')[-500:]
                raise RuntimeError(f'Command exit={process.returncode}: {detail}')
            return stdout or ''
        finally:
            if process:
                for stream in (process.stdout, process.stderr):
                    if stream:
                        stream.close()
            if output:
                output.close()

    def board(self, command, **kwargs):
        return self.command(self.ssh + [command], **kwargs)

    def copy(self, local, destination):
        self.command(
            self.scp + [str(local), f'{self.target}:{destination}'],
            log=self.artifacts / ('upload-' + pathlib.Path(local).name + '.log'),
            timeout=180,
        )

    def setup(self):
        self.cache.mkdir(exist_ok=True)
        if self.artifacts.exists() and any(self.artifacts.iterdir()):
            raise ValueError('RK_ARTIFACT_DIR must be empty; preserve previous evidence')
        self.artifacts.mkdir(parents=True, exist_ok=True)
        for tool in ['git', 'make', 'ffprobe', 'ffmpeg', 'readelf', 'ssh', 'scp']:
            if not shutil.which(tool):
                raise RuntimeError('Missing host tool: ' + tool)
        if os.environ.get('RK_PASSWORD_FILE') and not shutil.which('sshpass'):
            raise RuntimeError('Missing sshpass for RK_PASSWORD_FILE authentication')
        try:
            self.lock.mkdir()
        except FileExistsError as error:
            raise RuntimeError(f'Another e2e owns {self.lock}; inspect its owner.json') from error
        self.lock_owned = True
        (self.lock / 'owner.json').write_text(json.dumps(dict(pid=os.getpid(), run_id=self.run_id)))
        self.commit = self.command(['git', '-C', str(self.repo), 'rev-parse', 'HEAD']).strip()
        (self.artifacts / 'suite.json').write_text(
            json.dumps(dict(suite_sha256=self.test_digest, run_id=self.run_id), indent=2)
        )
        print(f'# Rockchip e2e commit={self.commit} artifacts={self.artifacts}', flush=True)
        self.board(
            'cat /proc/sys/kernel/random/boot_id; cat /proc/uptime; df -h /userdata',
            log=self.artifacts / 'preflight.txt',
        )

    def build(self):
        self.phase = 'T0.build'
        if (self.cache / 'rockchip-build.lock').exists():
            raise RuntimeError('Another Rockchip Cargo build is running')
        before = source_digest(self.repo)
        print('# Building fresh Rockchip package', flush=True)
        env = dict(os.environ, npm_config_cache=str(self.cache / 'npm'))
        self.command(
            ['make', '-C', str(self.repo), 'package', 'PACKAGE_ARCHS=rockchip'],
            log=self.artifacts / 'build.log',
            timeout=int(os.environ.get('RK_BUILD_TIMEOUT', '7200')),
            env=env,
        )
        if (
            self.command(['git', '-C', str(self.repo), 'rev-parse', 'HEAD']).strip() != self.commit
            or source_digest(self.repo) != before
        ):
            raise RuntimeError('HEAD/source changed during build; provenance cannot be claimed')
        version = re.search(
            r'^version = "([^"]+)"', (self.repo / 'nvr/Cargo.toml').read_text(), re.M
        ).group(1)
        self.package = self.repo / f'dist/lite-nvr-{version}-linux-arm64-rockchip.tar.gz'
        self.provenance('write')

    def provenance(self, action):
        self.command(
            [
                os.sys.executable,
                str(self.here / 'lib/provenance.py'),
                action,
                str(self.package),
                str(self.repo),
                self.commit,
            ]
        )
        shutil.copyfile(str(self.package) + '.build.json', self.artifacts / 'build.json')

    def deploy(self):
        self.phase = 'T0.deploy'
        ports = self.board('netstat -ltn')
        (self.artifacts / 'ports-before.txt').write_text(ports)
        for line in ports.splitlines()[2:]:
            fields = line.split()
            if len(fields) > 3 and int(fields[3].rsplit(':', 1)[-1]) in (
                18080,
                8553,
                self.rtsp_port,
                self.rtmp_port,
            ):
                raise RuntimeError('Refusing deployment: occupied port ' + fields[3])
        self.remote = '/userdata/lite-nvr-rk-e2e-' + self.run_id
        self.board('mkdir ' + shlex.quote(self.remote))
        self.copy(self.package, self.remote + '/package.tar.gz')
        self.board(
            f'gzip -dc {shlex.quote(self.remote + "/package.tar.gz")} | tar -x -C {shlex.quote(self.remote)}',
            timeout=120,
        )
        self.install = self.remote + '/' + self.package.name.removesuffix('.tar.gz')
        self.board(
            'mkdir '
            + shlex.quote(self.remote + '/board')
            + ' '
            + shlex.quote(self.remote + '/results')
        )
        for script in (self.here / 'board').glob('*.sh'):
            self.copy(script, self.remote + '/board/')
        self.board(
            f'cd {shlex.quote(self.install)} && NVR_ZLM_RTSP_PORT={self.rtsp_port} NVR_ZLM_RTMP_PORT={self.rtmp_port} ./start.sh start',
            log=self.artifacts / 'start.log',
        )
        for _ in range(15):
            try:
                self.board('wget -q -T 2 -O /dev/null http://127.0.0.1:18080/nvr/')
                return
            except RuntimeError:
                time.sleep(1)
        raise RuntimeError('Board service did not become ready within 15s')

    @staticmethod
    def http_ready(url):
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        try:
            with opener.open(url, timeout=3) as response:
                return response.status == 200
        except (OSError, urllib.error.URLError):
            return False

    def connect(self):
        self.phase = 'T0.connect'
        url_host = f'[{self.host}]' if ':' in self.host else self.host
        direct = self.http_ready(f'http://{url_host}:18080/nvr/')
        if direct:
            try:
                with socket.create_connection((self.host, self.rtsp_port), timeout=3):
                    pass
            except OSError:
                direct = False
        if direct:
            self.base, self.media_host, self.media_port = (
                f'http://{url_host}:18080',
                self.host,
                self.rtsp_port,
            )
            print('# HTTP and RTSP reachable directly', flush=True)
        else:
            with socket.socket() as http, socket.socket() as rtsp:
                http.bind(('127.0.0.1', 0))
                rtsp.bind(('127.0.0.1', 0))
                http_port, rtsp_port = http.getsockname()[1], rtsp.getsockname()[1]
            with (self.artifacts / 'tunnel.log').open('w') as log:
                self.tunnel = subprocess.Popen(
                    self.ssh[:-1]
                    + [
                        '-o',
                        'ExitOnForwardFailure=yes',
                        '-N',
                        '-L',
                        f'127.0.0.1:{http_port}:127.0.0.1:18080',
                        '-L',
                        f'127.0.0.1:{rtsp_port}:127.0.0.1:{self.rtsp_port}',
                        self.target,
                    ],
                    stdin=subprocess.DEVNULL,
                    stdout=log,
                    stderr=log,
                )
            self.base, self.media_host, self.media_port = (
                f'http://127.0.0.1:{http_port}',
                '127.0.0.1',
                rtsp_port,
            )
            for _ in range(10):
                if self.tunnel.poll() is not None:
                    raise RuntimeError('SSH forwarding failed; inspect tunnel.log')
                if self.http_ready(self.base + '/nvr/'):
                    break
                time.sleep(1)
            else:
                raise RuntimeError('SSH tunnel did not become ready')
            print('# Using loopback SSH tunnel', flush=True)
        os.environ.update(
            BASE=self.base,
            MEDIA_HOST=self.media_host,
            MEDIA_PORT=str(self.media_port),
            INSTALL_DIR=self.install,
            REMOTE_ROOT=self.remote,
            E2E_SSH_JSON=json.dumps(self.ssh),
            E2E_SCP_JSON=json.dumps(self.scp),
            RK_DEST=self.target,
            COMMIT=self.commit,
            ARTIFACTS=str(self.artifacts),
            RUN_ID=self.run_id,
            RK_RTSP_PORT=str(self.rtsp_port),
            RK_RTMP_PORT=str(self.rtmp_port),
        )

    def stop(self):
        try:
            if self.install:
                self.board(
                    'cd ' + shlex.quote(self.install) + ' && ./start.sh stop',
                    log=self.artifacts / 'cleanup.log',
                    timeout=35,
                )
                self.install = None
        finally:
            if self.tunnel:
                self.tunnel.terminate()
                try:
                    self.tunnel.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.tunnel.kill()
                    self.tunnel.wait()
                self.tunnel = None

    def backup_source(self, path):
        self.mutation_file = pathlib.Path(path)
        self.mutation_original = self.mutation_file.read_bytes()
        self.mutation_expected = self.mutation_original
        self.mutation_backup = self.cache / ('e2e-mutation-' + self.run_id + '.backup')
        self.mutation_backup.write_bytes(self.mutation_original)

    def change_source(self, text):
        if self.mutation_file.read_bytes() != self.mutation_expected:
            raise RuntimeError('Source changed outside this mutation; backup retained')
        self.mutation_file.write_text(text)
        self.mutation_expected = self.mutation_file.read_bytes()

    def restore_source(self):
        if self.mutation_file:
            if self.mutation_file.read_bytes() != self.mutation_expected:
                raise RuntimeError(
                    f'Source changed outside this mutation; preserved backup: {self.mutation_backup}'
                )
            self.mutation_file.write_bytes(self.mutation_original)
            self.mutation_backup.unlink()
            self.mutation_file = None

    def unlock(self):
        if self.lock_owned:
            (self.lock / 'owner.json').unlink(missing_ok=True)
            self.lock.rmdir()
            self.lock_owned = False
