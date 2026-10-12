"""Hardware checks never accept software fallback as hardware success."""

import os
import shlex
import subprocess
import time

from suite import CommandFailure, Skip


def hardware_cases(s):
    out = s.remote + '/results'

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
        from hdmi import RGB_SAMPLE_FILTER, matches_color_bars, output_config, require_mode

        card, connector = output_config()
        if os.environ.get('RK_HDMI_LOOPBACK', '1') == '1' and not os.environ.get(
            'RK_HDMI_CAPTURE_DEVICE'
        ):
            raise Skip('Set RK_HDMI_CAPTURE_DEVICE from the actual loopback wiring')
        require_mode(s, card, connector)
        s.script('hdmi', out, card, connector, 'pattern', timeout=40)
        if os.environ.get('RK_HDMI_LOOPBACK', '1') != '1':
            raise Skip(
                'DRM output presented; HDMI OUT->IN physical loopback not declared (RK_HDMI_LOOPBACK=1)'
            )
        from hdmi import loopback_signal

        local = s.artifacts / 'loop.mp4'
        with loopback_signal(s) as mode:
            p = s.payload(
                'hdmi-capture', input_type='v4l2', input_value=os.environ['RK_HDMI_CAPTURE_DEVICE']
            )
            s.add(p)
            try:
                s.wait_running(p['id'], seconds=15)
                s.execute(
                    [
                        'ffmpeg',
                        '-nostdin',
                        '-v',
                        'error',
                        '-rtsp_transport',
                        'tcp',
                        '-i',
                        s.rtsp(p['id']),
                        '-frames:v',
                        '50',
                        '-an',
                        '-c:v',
                        'copy',
                        '-y',
                        str(local),
                    ],
                    timeout=20,
                )
            finally:
                s.remove(p['id'])
        expected = mode
        video = s.probe(str(local), rtsp=False, seconds=2)
        assert (video['width'], video['height']) == expected[:2], video
        numerator, denominator = map(int, video['r_frame_rate'].split('/'))
        assert abs(numerator / denominator - expected[2]) <= 1, video
        # Content must match the generated upper-half color bars, not an unrelated signal.
        pixels = subprocess.run(
            [
                'ffmpeg',
                '-v',
                'error',
                '-i',
                str(local),
                '-frames:v',
                '1',
                '-vf',
                RGB_SAMPLE_FILTER,
                '-pix_fmt',
                'rgb24',
                '-f',
                'rawvideo',
                '-',
            ],
            capture_output=True,
            timeout=30,
            check=True,
        ).stdout
        assert matches_color_bars(pixels), f'loopback does not match color bars: {list(pixels)}'

    def dual():
        from hdmi import output_config, require_modes

        card, connector = output_config()
        second = os.environ.get('RK_HDMI_SECOND_CONNECTOR')
        if not second:
            raise Skip('Set RK_HDMI_SECOND_CONNECTOR from the actual second output wiring')
        assert second != connector, 'Two distinct HDMI output connectors are required'
        attempts = []
        for attempt in range(3):
            require_modes(s, card, [connector, second])
            try:
                text = s.script('hdmi', out, card, connector, 'dual', second)
                break
            except CommandFailure as error:
                attempts.append(str(error))
                (s.artifacts / 'dual-startup-attempts.log').write_text('\n'.join(attempts))
                # Both connectors are opened before the example presents any
                # frames. Wait only for an HPD race at that startup boundary.
                missing = any(
                    f'NoConnector(Some("{name}"))' in str(error) for name in [connector, second]
                )
                if not missing or attempt == 2:
                    raise
                time.sleep(2)
        assert (
            'Presented' in text and 'Second output presented after first output closed' in text
        ), text

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
            assert 'Cannot allocate memory' not in log and 'out of memory' not in log.lower(), (
                'OOM in NVR log'
            )

    for i, fn in enumerate([encode, decode, hdmi, dual, resources], 1):
        s.test(f'T3.{i}', '23217e1' if i in (3, 4) else 'baseline', fn)
