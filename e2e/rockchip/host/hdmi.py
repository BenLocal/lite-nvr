"""The same Rust pattern supplies the HDMI capture and output loopback tests."""

import contextlib
import json
import os
import pathlib
import re
import shlex
import subprocess
import time

from suite import CommandFailure, Skip

COLOR_BARS = [
    (255, 255, 255),
    (255, 255, 0),
    (0, 255, 255),
    (0, 255, 0),
    (255, 0, 255),
    (255, 0, 0),
    (0, 0, 255),
    (0, 0, 0),
]
RGB_SAMPLE_FILTER = 'crop=iw:ih/2:0:0,format=rgb24,scale=8:1'


def output_config():
    card = os.environ.get('RK_DRM_CARD')
    connector = os.environ.get('RK_HDMI_CONNECTOR')
    if not card or not connector:
        raise Skip('Set RK_DRM_CARD and RK_HDMI_CONNECTOR from the actual wiring')
    return card, connector


def matches_color_bars(pixels):
    return len(pixels) == 24 and all(
        abs(pixels[i * 3 + channel] - rgb[channel]) < 65
        for i, rgb in enumerate(COLOR_BARS)
        for channel in range(3)
    )


def build_examples(s):
    if getattr(s, 'hdmi_built', False):
        return
    repo = pathlib.Path(__file__).resolve().parents[3]
    s.execute(['make', '-C', str(repo), 'rockchip-e2e-hdmi'], timeout=1800)
    for name in ['hdmiout', 'dual_hdmiout']:
        s.execute(
            json.loads(os.environ['E2E_SCP_JSON'])
            + [
                str(repo / 'target/aarch64-unknown-linux-gnu/release/examples' / name),
                os.environ['RK_DEST'] + ':' + s.remote + '/results/',
            ],
            timeout=120,
        )
    s.hdmi_built = True


def require_mode(s, card, connector):
    require_modes(s, card, [connector])


def require_modes(s, card, connectors):
    build_examples(s)
    deadline = time.monotonic() + 12
    stable = 0
    while time.monotonic() < deadline:
        # Force the kernel to refresh connector modes after the receiver's HPD
        # transition; sysfs alone may still hold an empty cached mode list.
        s.script('hdmi', s.remote + '/results', card, connectors[0], 'list')
        conditions = []
        for connector in connectors:
            condition = s.script(
                'hdmi', s.remote + '/results', card, connector, 'conditions'
            ).splitlines()
            assert condition[0] in ['connected', 'disconnected', 'unknown'], condition
            conditions.append((connector, condition))
        ready = all(
            condition[0] == 'connected' and int(condition[2]) > 0 for _, condition in conditions
        )
        stable = stable + 1 if ready else 0
        if stable >= 2:
            return
        time.sleep(1)
    detail = '; '.join(
        f'{name}: status={condition[0]}, EDID bytes={condition[1]}, modes={condition[2]}'
        for name, condition in conditions
    )
    raise Skip(detail + '; HDMI OUT requires stable usable modes')


@contextlib.contextmanager
def loopback_signal(s):
    card, connector = output_config()
    capture = os.environ.get('RK_HDMI_CAPTURE_DEVICE')
    if not capture:
        raise Skip('Set RK_HDMI_CAPTURE_DEVICE to the input physically wired to RK_HDMI_CONNECTOR')
    require_mode(s, card, connector)
    out = s.remote + '/results'
    log_path = s.artifacts / 'capture-pattern.log'
    # A finite pattern ends normally and restores the previous CRTC. Do not
    # kill the Rust example early, which would bypass its close/drop path.
    with log_path.open('w') as log:
        args = [
            'timeout',
            '40',
            'sh',
            s.remote + '/board/hdmi.sh',
            s.root,
            out,
            card,
            connector,
            'pattern',
            '30',
        ]
        process = subprocess.Popen(s.ssh + [shlex.join(args)], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 12
            stable = 0
            signal = 'Output mode not yet reported'
            while time.monotonic() < deadline:
                assert process.poll() is None, (
                    'HDMI pattern exited before capture: ' + log_path.read_text()
                )
                expected = re.search(r'Output: (\d+)x(\d+) @ (\d+) Hz', log_path.read_text())
                if expected:
                    try:
                        pixels = list(
                            map(
                                int,
                                s.script(
                                    'hdmi',
                                    out,
                                    card,
                                    connector,
                                    'sample',
                                    capture,
                                    expected[3],
                                    timeout=10,
                                ).split(),
                            )
                        )
                        stable = stable + 1 if matches_color_bars(pixels) else 0
                        signal = f'{capture}: sampled RGB bars {pixels}'
                    except CommandFailure as error:
                        stable = 0
                        signal = str(error)
                if stable >= 3:
                    break
                time.sleep(1)
            else:
                raise Skip(
                    'Configured HDMI input did not stably capture the generated color bars: '
                    + signal.replace('\n', '; ')
                )
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
