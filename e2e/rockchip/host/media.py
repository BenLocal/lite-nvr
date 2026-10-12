"""Media assertions use decoded frames, not only reachable URLs."""
import json
import os
import re
import time
from urllib.parse import urljoin

from suite import Skip


def media_cases(s):
    def pattern():
        p = s.ensure_pattern()
        video = s.probe(s.rtsp(p['id']))
        assert video['codec_name'] == 'h264', video

    def output():
        p = s.ensure_pattern()
        p['config'] = {'output': dict(video_codec='h264', width=1280, height=720, fps=25, max_bitrate=2000000)}
        s.update(p)
        s.wait_running(p['id'])
        video = s.probe(s.rtsp(p['id']))
        assert (video['width'], video['height']) == (1280, 720), video
        num, den = map(int, video['r_frame_rate'].split('/'))
        assert abs(num / den - 25) <= 1, video

    def passthrough():
        s.script('source', s.remote + '/results', os.environ['RK_RTSP_PORT'], 'start')
        s.source_started = True
        p = s.payload('copy', input_type='rtsp', input_value=s.rtsp('source', 'e2e', board=True), include_audio=True)
        s.add(p)
        s.wait_running(p['id'])
        video = s.probe(s.rtsp(p['id']))
        assert (video['codec_name'], video['width'], video['height']) == ('h264', 640, 360), video
        streams = json.loads(s.execute(['ffprobe', '-v', 'error', '-rtsp_transport', 'tcp', '-show_streams', '-of', 'json', s.rtsp(p['id'])]))['streams']
        audio = next(stream for stream in streams if stream['codec_type'] == 'audio')
        assert audio['codec_name'] == 'aac' and int(audio['sample_rate']) == 48000, audio
        assert 'output' not in s.get(p['id'])['config']
        metrics = s.api('GET', '/api/device/metrics/' + p['id'])
        assert not metrics['pipeline']['encoders'], 'Unconfigured encoded source was unexpectedly re-encoded'
        s.remove(p['id'])

    def hevc():
        p = s.ensure_pattern()
        p['config'] = {'output': {'video_codec': 'h265', 'width': 640, 'height': 360}}
        s.update(p)
        s.wait_running(p['id'])
        video = s.probe(s.rtsp(p['id']))
        assert video['codec_name'] == 'hevc', video

    def flv_hls():
        p = s.payload('hls', input_value='testsrc2=size=640x360:rate=25,realtime', record=True)
        s.add(p)
        s.wait_running(p['id'])
        s.probe(s.base + '/media/device/' + p['id'] + '.live.flv', rtsp=False)
        deadline = time.monotonic() + int(os.environ.get('RK_RECORD_TIMEOUT', '210'))
        path = '/media/device/' + p['id'] + '/hls.m3u8'
        playlist = ''
        while time.monotonic() < deadline:
            try:
                playlist = s.api('GET', path, raw=True)
                if '#EXTINF:' in playlist:
                    break
            except AssertionError:
                pass  # ZLM does not expose the playlist before its first segment.
            s.check_boot()
            time.sleep(3)
        assert '#EXTINF:' in playlist, 'HLS never produced a segment'
        uri = next(line.strip() for line in playlist.splitlines() if line.strip() and not line.startswith('#'))
        url = urljoin(s.base + path, uri)
        s.probe(url, rtsp=False, seconds=2)
        s.remove(p['id'])

    def v4l2():
        node = os.environ.get('RK_VIDEO_DEVICE')
        if node:
            p = s.payload('v4l2', input_type='v4l2', input_value=node)
            s.add(p)
            s.wait_running(p['id'])
            video = s.probe(s.rtsp(p['id']))
            assert video['codec_name'] == 'h264', video
            s.remove(p['id'])
        elif os.environ.get('RK_HDMI_LOOPBACK', '1') == '1':
            from hdmi import loopback_signal
            with loopback_signal(s) as mode:
                p = s.payload('v4l2-loop', input_type='v4l2', input_value='/dev/video11')
                s.add(p)
                try:
                    s.wait_running(p['id'], seconds=15)
                    video = s.probe(s.rtsp(p['id']), seconds=5)
                    assert video['codec_name'] == 'h264', video
                    assert (video['width'], video['height']) == mode[:2], video
                finally:
                    s.remove(p['id'])
        else:
            raise Skip('HDMI loopback disabled and no RK_VIDEO_DEVICE supplied')

    def no_signal():
        node = os.environ.get('RK_NO_SIGNAL_DEVICE')
        if not node:
            raise Skip('Set RK_NO_SIGNAL_DEVICE to a verified free node without signal')
        if node == '/dev/video11':
            signal = s.board('cat /sys/kernel/debug/hdmirx/status')
            if 'Clk-Ch:Unlock' not in signal:
                raise Skip('HDMI input is locked; it is not a no-signal fixture')
        p = s.payload('no-signal', input_type='v4l2', input_value=node)
        s.add(p)
        time.sleep(3)  # Allow the old worker to block in open/probe/read.
        start = time.monotonic()
        p.update(input_type='lavfi', input_value='testsrc2=size=640x360:rate=25,realtime')
        s.update(p)
        assert time.monotonic() - start < 15, 'replacement of no-signal pipeline exceeded 15s'
        s.wait_running(p['id'])
        s.probe(s.rtsp(p['id']), seconds=2)
        p.update(input_type='v4l2', input_value=node)
        s.update(p)
        time.sleep(3)
        start = time.monotonic()
        s.remove(p['id'])
        assert time.monotonic() - start < 15, 'deletion of no-signal pipeline exceeded 15s'

    def recording():
        p = s.payload('record', input_value='testsrc2=size=640x360:rate=25,realtime', record=True)
        s.add(p)
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
        assert len(segments) >= 2, f'expected >=2 real recording segments, actual {len(segments)}'
        # Verify each recorded segment decodes, then the actual dashboard transition.
        for segment in segments[:2]:
            body = s.api('GET', '/api/playback/segment-playlist/' + segment['id'], raw=True)
            assert '#EXTINF:' in body and '#EXT-X-ENDLIST' in body, body
            uri = next(line for line in body.splitlines() if line and not line.startswith('#'))
            s.probe(urljoin(s.base + '/api/playback/segment-playlist/' + segment['id'], uri) + '?token=' + s.token,
                    rtsp=False, seconds=2)
        from playback import verify_playback
        verify_playback(s, p, segments)
        s.remove(p['id'])

    def compositor():
        p = s.ensure_pattern()
        id_ = s.prefix + '-compositor'
        s.compositors.append(id_)
        data = s.api('POST', '/api/compositor/create', dict(id=id_, sources=[dict(id='camera', url=s.rtsp(p['id'], board=True))], width=640, height=360, fps=25))
        assert data['rtsp_port'] == int(os.environ['RK_RTSP_PORT']), data
        assert 0 < data['rtsp_port'] < 65536, data
        time.sleep(3)
        s.probe(s.rtsp(id_, 'switcher'))
        s.api('POST', '/api/compositor/remove/' + id_)
        s.compositors.remove(id_)

    def detection():
        cfg = dict(enabled=False, models=['e2e-disabled-model'], sample_every_ms=1000, min_confidence=0.4)
        p = s.payload('detect', config={'detect': cfg})
        data = s.add(p)
        before = data['config']['detect']
        p['description'] = 'detect preserved'
        s.update(p)
        assert s.get(p['id'])['config']['detect'] == before, 'detect config changed on unrelated update'
        s.remove(p['id'])

    def audio_output():
        if not s.source_started:
            s.script('source', s.remote + '/results', os.environ['RK_RTSP_PORT'], 'start')
            s.source_started = True
        for codec, rate in [('aac', 32000), ('opus', 48000)]:
            p = s.payload('audio-' + codec, input_type='rtsp', input_value=s.rtsp('source', 'e2e', board=True),
                          include_audio=True, config={'output': {'audio_codec': codec, 'sample_rate': rate}})
            s.add(p)
            s.wait_running(p['id'])
            # ffprobe intervals can end on audio before ten seconds of video.
            # FFmpeg's mapped video output measures an actual video window.
            progress = s.execute(['ffmpeg', '-nostdin', '-v', 'error', '-rtsp_transport', 'tcp',
                '-i', s.rtsp(p['id']), '-map', '0:v:0', '-an', '-vf', 'setpts=PTS-STARTPTS',
                '-t', '10', '-vsync', '0',
                '-progress', 'pipe:1', '-nostats', '-f', 'null', '-'], timeout=40)
            frames = [int(n) for n in re.findall(r'^frame=(\d+)$', progress, re.M)]
            video_frames = frames[-1] if frames else 0
            streams = json.loads(s.execute(['ffprobe', '-v', 'error', '-rtsp_transport', 'tcp',
                '-select_streams', 'a', '-read_intervals', '%+10', '-count_frames',
                '-show_streams', '-of', 'json', s.rtsp(p['id'])], timeout=40))['streams']
            (s.artifacts / ('audio-' + codec + '.json')).write_text(json.dumps(dict(video_frames_10s=video_frames, audio_streams=streams), indent=2))
            audio = next(stream for stream in streams if stream['codec_type'] == 'audio')
            assert audio['codec_name'] == codec and int(audio['sample_rate']) == rate, audio
            assert int(audio.get('nb_read_frames', '0')) > 0, 'Audio did not decode any frames'
            assert video_frames >= 200, f'{codec}: expected decoded video frames >=200 in 10s, actual {video_frames}'
            s.remove(p['id'])

    for i, fn in enumerate([pattern, output, passthrough, hevc, flv_hls, v4l2, no_signal, recording, compositor, detection], 1):
        commit = '01f3d31' if i == 8 else 'b33e7d7' if i in (1, 6, 7) else '23217e1'
        s.test(f'T2.{i}', commit, fn)
    s.test('T2.11', '23217e1', audio_output)
