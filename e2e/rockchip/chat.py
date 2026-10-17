#!/usr/bin/env python3
"""Drive the nvr voice chat (Socket.IO `/chat`) without a browser.

Logs in, then for each round either pushes a WAV through push-to-talk
(`start` → `audio` chunks → `stop`) or sends a typed question (`text`), waits
for `reply_done` and prints the transcript, the reply, and how long the first
spoken sentence took after the question ended. Reply audio is saved as WAV.

Needs python-socketio and requests, e.g. in the e2e cache:
    pip install --target .cache/rockchip-e2e-python "python-socketio[client]" requests
    export PYTHONPATH="$PWD/.cache/rockchip-e2e-python"

Usage:
    e2e/rockchip/chat.py --url http://<board>:18080 --wav question.wav
    e2e/rockchip/chat.py --url http://127.0.0.1:18080 --text "你好" --rounds 3
The WAV must be 16 kHz mono 16-bit, e.g.
    ffmpeg -i in.m4a -ar 16000 -ac 1 -sample_fmt s16 question.wav
"""

import argparse
import os
import sys
import threading
import time
import wave

import requests
import socketio

CHUNK_MS = 100


def login(url, username, password):
    r = requests.post(
        f'{url}/api/user/login',
        json={'username': username, 'password': password},
        timeout=10,
    )
    r.raise_for_status()
    body = r.json()
    if body.get('code') != 0:
        sys.exit(f'login failed: {body}')
    return body['data']['token']


def read_pcm(path):
    with wave.open(path, 'rb') as w:
        if (w.getframerate(), w.getnchannels(), w.getsampwidth()) != (16000, 1, 2):
            sys.exit(f'{path}: need 16 kHz mono 16-bit PCM')
        return w.readframes(w.getnframes())


class Round:
    def __init__(self):
        self.done = threading.Event()
        self.turn = None
        self.heard = None
        self.reply = ''
        self.audio = []  # (seq, sample_rate, pcm)
        self.errors = []
        self.asked_at = None
        self.first_audio = None


def main():
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument('--url', default='http://127.0.0.1:18080')
    p.add_argument('--user', default=os.environ.get('RK_API_USER', 'admin'))
    p.add_argument('--password', default=os.environ.get('RK_API_PASSWORD', 'admin'))
    q = p.add_mutually_exclusive_group(required=True)
    q.add_argument('--wav', help='question audio (16 kHz mono s16)')
    q.add_argument('--text', help='typed question')
    p.add_argument('--rounds', type=int, default=1, help='ask this many times')
    p.add_argument('--realtime', action='store_true', help='pace audio in real time')
    p.add_argument('--timeout', type=float, default=120, help='seconds per round')
    p.add_argument('--out', default='chat_reply', help='reply WAV prefix')
    args = p.parse_args()

    url = args.url.rstrip('/')
    token = login(url, args.user, args.password)
    pcm = read_pcm(args.wav) if args.wav else None

    sio = socketio.Client()
    state = {'round': Round()}

    @sio.on('asr_partial', namespace='/chat')
    def on_partial(data):
        print(f'  … {data["text"]}', flush=True)

    @sio.on('asr_final', namespace='/chat')
    def on_final(data):
        r = state['round']
        r.heard = data['text']
        print(f'  asr: {r.heard!r}', flush=True)
        if not r.heard:
            r.done.set()

    @sio.on('reply_delta', namespace='/chat')
    def on_delta(data):
        r = state['round']
        r.turn = data['turn']
        r.reply += data['text']
        print(data['text'], end='', flush=True)

    @sio.on('reply_audio', namespace='/chat')
    def on_audio(data):
        r = state['round']
        if r.first_audio is None:
            r.first_audio = time.monotonic()
        r.audio.append((data['seq'], data['sample_rate'], data['pcm']))

    @sio.on('reply_done', namespace='/chat')
    def on_done(data):
        state['round'].done.set()

    @sio.on('error', namespace='/chat')
    def on_error(data):
        state['round'].errors.append(data.get('message'))
        print(f'\n  error: {data}', flush=True)

    sio.connect(url, namespaces=['/chat'], auth={'token': token}, transports=['websocket'])

    failures = 0
    for i in range(args.rounds):
        r = state['round'] = Round()
        print(f'round {i + 1}:', flush=True)
        if pcm is not None:
            sio.emit('start', namespace='/chat')
            step = 16000 * 2 * CHUNK_MS // 1000
            for off in range(0, len(pcm), step):
                sio.emit('audio', pcm[off : off + step], namespace='/chat')
                if args.realtime:
                    time.sleep(CHUNK_MS / 1000)
            r.asked_at = time.monotonic()
            sio.emit('stop', namespace='/chat')
        else:
            r.asked_at = time.monotonic()
            sio.emit('text', args.text, namespace='/chat')
        if not r.done.wait(args.timeout):
            print(f'\n  timed out after {args.timeout}s')
            failures += 1
            continue
        print()
        first = f'{r.first_audio - r.asked_at:.2f}s' if r.first_audio else 'none'
        print(f'  sentences spoken: {len(r.audio)}, first audio after {first}')
        if r.errors or not r.audio:
            failures += 1
        if r.audio:
            r.audio.sort()
            path = f'{args.out}_{i + 1}.wav'
            with wave.open(path, 'wb') as w:
                w.setnchannels(1)
                w.setsampwidth(2)
                w.setframerate(r.audio[0][1])
                w.writeframes(b''.join(a[2] for a in r.audio))
            print(f'  wrote {path}')
    sio.disconnect()
    print(f'{args.rounds - failures}/{args.rounds} rounds ok')
    sys.exit(1 if failures else 0)


if __name__ == '__main__':
    main()
