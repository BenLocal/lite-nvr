"""Hardware checks never accept software fallback as hardware success."""
import json
import os
import re
import shlex
import subprocess
import time

from suite import Skip


def hardware_cases(s):
    out = s.remote + '/results'
    card = os.environ.get('RK_DRM_CARD', '/dev/dri/card0')
    connector = os.environ.get('RK_HDMI_CONNECTOR', 'HDMI-A-2')

    def encode():
        output = s.script('hwcodec', out, 'encode', timeout=90)
        assert 'h264_rkmpp' in output and 'libx264' not in output, output

    def decode():
        if s.selected == 'T3.2':
            encode()
        s.board('test -s ' + shlex.quote(out + '/hardware.mp4'))
        output = s.script('hwcodec', out, 'decode', timeout=90)
        assert 'h264_rkmpp' in output and 'frame=' in output, output

    def hdmi():
        from hdmi import require_mode
        require_mode(s, card, connector)
        s.script('hdmi', out, card, connector, 'pattern', timeout=40)
        if os.environ.get('RK_HDMI_LOOPBACK', '1') != '1':
            raise Skip('DRM output presented; HDMI OUT->IN physical loopback not declared (RK_HDMI_LOOPBACK=1)')
        from hdmi import loopback_signal
        with loopback_signal(s) as mode:
            s.script('hdmi', out, card, connector, 'grab', timeout=25)
        expected = mode
        local = s.artifacts / 'loop.mp4'
        s.execute(json.loads(os.environ['E2E_SCP_JSON']) + [os.environ['RK_DEST'] + ':' + out + '/loop.mp4', str(local)], timeout=120)
        video = s.probe(str(local), rtsp=False, seconds=2)
        assert (video['width'], video['height']) == expected[:2], video
        numerator, denominator = map(int, video['r_frame_rate'].split('/'))
        assert abs(numerator / denominator - expected[2]) <= 1, video
        # Content must match the generated upper-half color bars, not an unrelated signal.
        pixels = subprocess.run(['ffmpeg', '-v', 'error', '-i', str(local), '-frames:v', '1',
                                 '-vf', 'crop=iw:ih/2:0:0,scale=8:1', '-pix_fmt', 'rgb24', '-f', 'rawvideo', '-'],
                                capture_output=True, timeout=30, check=True).stdout
        expected_rgb = [(255,255,255), (255,255,0), (0,255,255), (0,255,0), (255,0,255), (255,0,0), (0,0,255), (0,0,0)]
        assert len(pixels) == 24
        assert all(abs(pixels[i * 3 + channel] - rgb[channel]) < 65 for i, rgb in enumerate(expected_rgb) for channel in range(3)), f'loopback does not match color bars: {list(pixels)}'

    def dual():
        if card != '/dev/dri/card0':
            raise Skip('dual_hdmiout example uses card0; configured card differs')
        from hdmi import build_examples
        build_examples(s)
        for name in ['HDMI-A-1', 'HDMI-A-2']:
            condition = s.script('hdmi', out, card, name, 'conditions').splitlines()
            if condition[0] != 'connected' or int(condition[2]) == 0:
                raise Skip(f'{name} has no connected sink with a usable mode')
        text = s.script('hdmi', out, card, connector, 'dual')
        assert 'Presented' in text, text

    def resources():
        # Remove prior cases' encoders so 1/2/4 means exactly these fixtures.
        s.clear_fixtures()
        seconds = int(os.environ.get('RK_RESOURCE_SECONDS', '20'))
        assert seconds >= 10, 'Resource sampling duration must be >=10s'
        for count in [1, 2, 4]:
            fixtures = [s.payload(f'resource-{count}-{i}') for i in range(count)]
            for p in fixtures:
                s.add(p)
                s.wait_running(p['id'])
            before = s.script('health', 'resources')
            time.sleep(seconds)
            after = s.script('health', 'resources')
            (s.artifacts / f'resources-{count}.txt').write_text(before + '\n---\n' + after)
            assert 'VmRSS:' in after, 'NVR process disappeared during resource sampling'
            for p in fixtures:
                assert s.get(p['id'])['state'] == 'running', 'resource test lost a stream'
                s.probe(s.rtsp(p['id']), seconds=2)
                s.remove(p['id'])
            log = s.board('tail -n 200 ' + shlex.quote(s.root + '/nvr.log'))
            assert 'Cannot allocate memory' not in log and 'out of memory' not in log.lower(), 'OOM in NVR log'

    for i, fn in enumerate([encode, decode, hdmi, dual, resources], 1):
        s.test(f'T3.{i}', '23217e1' if i in (3,4) else 'baseline', fn)
