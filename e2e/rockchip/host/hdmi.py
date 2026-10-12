"""The same Rust pattern supplies the HDMI capture and output loopback tests."""
import contextlib
import json
import os
import pathlib
import re
import shlex
import subprocess
import time

from suite import Skip


def build_examples(s):
    if getattr(s, 'hdmi_built', False):
        return
    repo = pathlib.Path(__file__).resolve().parents[3]
    s.execute(['make', '-C', str(repo), 'rockchip-e2e-hdmi'], timeout=1800)
    for name in ['hdmiout', 'dual_hdmiout']:
        s.execute(json.loads(os.environ['E2E_SCP_JSON']) +
                  [str(repo / 'target/aarch64-unknown-linux-gnu/release/examples' / name), os.environ['RK_DEST'] + ':' + s.remote + '/results/'], timeout=120)
    s.hdmi_built = True


def require_mode(s, card, connector):
    build_examples(s)
    s.script('hdmi', s.remote + '/results', card, connector, 'list')
    condition = s.script('hdmi', s.remote + '/results', card, connector, 'conditions').splitlines()
    assert condition[0] in ['connected', 'disconnected', 'unknown'], condition
    if condition[0] != 'connected' or int(condition[2]) == 0:
        raise Skip(f'{connector}: status={condition[0]}, EDID bytes={condition[1]}, modes={condition[2]}; HDMI OUT cannot provide a loopback signal without a usable mode')


@contextlib.contextmanager
def loopback_signal(s):
    card = os.environ.get('RK_DRM_CARD', '/dev/dri/card0')
    connector = os.environ.get('RK_HDMI_CONNECTOR', 'HDMI-A-2')
    require_mode(s, card, connector)
    out = s.remote + '/results'
    log_path = s.artifacts / 'capture-pattern.log'
    # A finite pattern ends normally and restores the previous CRTC. Do not
    # kill the Rust example early, which would bypass its close/drop path.
    with log_path.open('w') as log:
        args = ['timeout', '40', 'sh', s.remote + '/board/hdmi.sh', s.root, out, card, connector, 'pattern', '30']
        process = subprocess.Popen(s.ssh + [shlex.join(args)], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 8
            stable = 0
            while time.monotonic() < deadline:
                assert process.poll() is None, 'HDMI pattern exited before capture: ' + log_path.read_text()
                signal = s.board('cat /sys/kernel/debug/hdmirx/status')
                expected = re.search(r'Output: (\d+)x(\d+) @ (\d+) Hz', log_path.read_text())
                locked = all(re.search(name + r':Lock\b', signal) for name in ['Clk-Ch', 'Ch0', 'Ch1', 'Ch2'])
                timing = expected and f'Timing: {expected[1]}x{expected[2]}' in signal
                stable = stable + 1 if locked and timing else 0
                if stable >= 3:
                    break
                time.sleep(1)
            else:
                raise Skip('HDMI OUT pattern presented, but IN never stably locked all channels and matching timing: ' + signal.replace('\n', '; '))
            mode = re.search(r'Output: (\d+)x(\d+) @ (\d+) Hz', log_path.read_text())
            assert mode, 'HDMI pattern did not report its output mode'
            yield tuple(map(int, mode.groups()))
        finally:
            try:
                code = process.wait(timeout=45)
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
                raise AssertionError('HDMI pattern exceeded its finite runtime')
            s.last = log_path.read_text()
            assert code == 0, 'HDMI pattern failed: ' + s.last
