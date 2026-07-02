# dummy-camera — a GB28181 device (IPC) emulator

A runnable "dummy camera" that behaves like a real Hikvision IPC (下级设备) toward
an NVR / GB28181 platform. It:

- **REGISTERs** to the platform with digest auth and keeps the registration alive
  (periodic Keepalive), retrying until the platform is up.
- Answers **Catalog** (channel list), **DeviceInfo** (manufacturer / model /
  firmware), and **RecordInfo** (录像文件查询 — advertised recordings) queries.
- On **INVITE (Play)**, pushes H.264 as **PS-over-RTP** to the address the platform
  asked to receive on — over **UDP**, **TCP-passive**, or **TCP-active**, whichever
  the SDP offer selects. The video is either the bundled clip or, with
  `--source-file`, any local file streamed live via **ffmpeg** (looped).
- On **INVITE (Playback / Download)**, streams a recorded file (`--record-file`)
  seeked to the requested time window (设备历史视频回放).
- Logs **DeviceControl (PTZ)** commands (a real camera would move its motors).
- **Unregisters** (设备注销, REGISTER `Expires: 0`) on Ctrl-C, and BYEs / stops
  streaming cleanly when the platform tears a session down.

All SIP signaling is handled by the workspace's `gb28181` crate (`GbClient`); this
binary adds the media plane: an Annex-B H.264 parser (`h264.rs`, with an
incremental `AnnexBParser` for ffmpeg's byte stream), a GB28181 MPEG-2 Program
Stream muxer (`ps.rs`), and an RTP packetizer + transport with an ffmpeg
file-source streamer (`rtp.rs`).

## Run

```bash
cargo run -p dummy-camera -- \
  --server-addr 127.0.0.1:5060 \
  --server-id   34020000002000000001 \
  --device-id   34020000001320000001 \
  --password    12345678            # omit for an open/no-auth platform
```

Key options (`--help` for all):

| Flag | Meaning | Default |
|------|---------|---------|
| `--server-addr` | Platform SIP UDP address | *required* |
| `--server-id` | Platform 20-digit GB code | *required* |
| `--device-id` | Our device 20-digit GB code | *required* |
| `--domain` | SIP domain / digest realm | first 10 digits of `--server-id` |
| `--channel-id` | Advertised channel (the entry the NVR pulls) | `= --device-id` |
| `--source-file` | Local video file to stream live via ffmpeg (else the bundled clip) | *(none)* |
| `--record-file` | File advertised as a recording + served on Playback | `= --source-file` |
| `--record-start` / `--record-end` | Advertised recording window (ISO 8601) — also the Playback seek origin | `2024-01-01T00:00:00` / `…T01:00:00` |
| `--manufacturer` / `--model` / `--firmware` | DeviceInfo fields | `lite-nvr` / `dummy-camera` / `0.1` |
| `--media-ip` | IP advertised as our media source | `127.0.0.1` |
| `--fps` | Playback frame rate (paces RTP + 90 kHz clock) | `25` |
| `--listen` | Local SIP listen address | `0.0.0.0:5061` |

Set `RUST_LOG=debug` for verbose SIP/media logs.

## End-to-end against this NVR

Start the NVR with GB support enabled (see the repo README for `FFMPEG_DIR` /
`ZLM_DIR` / `LD_LIBRARY_PATH`):

```bash
NVR_GB_ENABLE=1 \
NVR_GB_SIP_ID=34020000002000000001 \
NVR_GB_DOMAIN=3402000000 \
NVR_GB_PORT=5060 \
NVR_GB_MEDIA_IP=127.0.0.1 \
cargo run --package nvr
```

Add a gb device so the NVR knows the channel to pull (matching the dummy's
`--device-id` / `--channel-id`):

```bash
curl -s localhost:8080/api/device/add -H 'content-type: application/json' -d '{
  "id": "cam-dummy",
  "name": "Dummy Cam",
  "input_type": "gb28181",
  "input_value": "{\"device_id\":\"34020000001320000001\",\"channel_id\":\"34020000001320000001\"}"
}'
```

Then:

1. **Run the dummy** (command above). The NVR logs `Registered … 34020000001320000001`.
2. **Trigger a pull:** `curl -s localhost:8080/api/gb/play -d '{"device_id":"cam-dummy"}'`
   (add `,"transport":"tcp_passive"` / `"tcp_active"` to exercise TCP). ZLM sends
   an INVITE; the dummy answers and starts pushing PS/RTP.
3. **Confirm it's live:** `curl -s localhost:8080/api/gb/streams` shows the stream
   with `"live":true` and an `rtp` block (peer/ssrc/port). The dashboard device
   row shows **拉流中**.
4. **Watch the video:** open `http://127.0.0.1:8553/rtp/cam-dummy.live.flv` (the
   looping test pattern).
5. **Teardown:** stop viewing → the NVR BYEs → the dummy logs the session close
   and stops streaming.
6. **Unregister:** press Ctrl-C → the dummy sends `REGISTER Expires: 0` (设备注销)
   and exits; the platform drops the device.

> This flow was validated end-to-end against this NVR with `--source-file`: the
> pulled `.live.flv` decoded cleanly as 640×360 H.264 (the ffmpeg source), and
> Ctrl-C logged `unregistering (设备注销) → unregistered`.

## Live file source & history playback

`--source-file <path>` makes the camera stream a **real local video** (any format
ffmpeg reads) instead of the bundled clip. ffmpeg transcodes it to baseline H.264
and loops it in real time (`-re -stream_loop -1`); the incremental `AnnexBParser`
turns ffmpeg's byte stream into access units fed to the same PS/RTP path.

```bash
cargo run -p dummy-camera -- \
  --server-addr 172.17.0.1:5060 --server-id 34020000002000000001 \
  --device-id 34020000001320000001 --password 12345678 \
  --source-file /path/to/clip.mp4
```

The same file is advertised as a **recording** (RecordInfo) and served on a
**Playback / Download INVITE** (设备历史视频回放): ffmpeg seeks it to the requested
time window (offset from `--record-start`) and plays that segment once. Pass an
explicit `--record-file` to serve a different file for playback than the live
source. `--record-start` / `--record-end` set the advertised recording window and
the seek origin. (This NVR pulls live only; drive Playback from a platform such as
WVP, or exercise it with the crate's unit tests.)

## Diagnostics

Inspect the exact Program Stream the camera would send, without any SIP:

```bash
cargo run -p dummy-camera -- --server-addr 0:0 --server-id x --device-id x \
  --dump-ps /tmp/dummy.ps
ffprobe /tmp/dummy.ps        # -> format=mpeg, codec=h264, 480x270
ffmpeg -i /tmp/dummy.ps -f null -   # decodes every frame with no errors
```

## The sample clip

`assets/sample.h264` is a 4-second 480×270 baseline-H.264 test pattern (embedded
via `include_bytes!`, looped). Regenerate with:

```bash
ffmpeg -f lavfi -i testsrc2=size=480x270:rate=20 -t 4 \
  -c:v libx264 -profile:v baseline -pix_fmt yuv420p -crf 30 \
  -x264-params "keyint=20:min-keyint=20:scenecut=0:repeat-headers=1" \
  -an assets/sample.h264
```

`repeat-headers=1` puts SPS/PPS before every IDR, so a viewer that joins
mid-stream (or on a loop boundary) always gets a decodable keyframe.

## Notes / limits

- Video only (no audio) — sufficient for the NVR to register and display the stream.
- One `--media-port` is shared across pulls; use the default `0` (ephemeral) so
  concurrent pulls don't collide.
- TCP-active binds a listener and waits up to 10 s for the platform to connect.
