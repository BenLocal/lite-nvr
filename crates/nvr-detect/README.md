# nvr-detect — multi-model object detection

A backend-agnostic detection component: a `Detector` trait, unified
`Detection` output (label, bbox in pixels, confidence), and a `usls`
(ONNX Runtime) YOLO backend. `DetectorSet` runs one image through N models
and returns each model's timed result.

## Runtime dependency (ONNX Runtime)

`usls` is built with the `ort-load-dynamic` feature (not the bundled/static
ORT), so that its ONNX Runtime symbols don't clash with `nvr-asr`'s own
ONNX Runtime linkage in the same process. This means the `.onnx` weights
alone are not enough at runtime: a `libonnxruntime.so` matching the ORT
version pulled in by `ort = "2.0.0-rc.10"` must be resolvable, either via

```bash
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
```

or by putting its directory on `LD_LIBRARY_PATH`. Without it, loading any
model fails at runtime (not at build/link time). Building the crate and
running its non-inference tests (config/types/set/convert/hub/tap wiring)
do **not** need the `.so` — only actually running a `Detector` against a
real model does.

## Offline comparison

Put ONNX weights + a `models.json` under `third_party/detect-models/`:

```json
[
  { "name": "yolov8n", "model_file": "yolov8n.onnx", "version": 8.0 },
  { "name": "yolo11s", "model_file": "yolo11s.onnx", "version": 11.0 }
]
```

`version` is required — usls needs `Config.version` set to pick the YOLO
head/decoder (`YOLO::new` bails at runtime with "No clear YOLO Version
specified" if it's omitted).

Weights are not committed. Export from Ultralytics, e.g.
`yolo export model=yolov8n.pt format=onnx`, and drop the `.onnx` in that dir.

```bash
cargo run -p nvr-detect --example detect-compare -- \
  --image some.jpg --models third_party/detect-models/models.json \
  --models-dir third_party/detect-models
```

## Real-time (in nvr)

`nvr` taps a running pipe's decoded video, samples (~2fps), fans each frame out
to the configured models, and serves the latest result. The API (port 18080) is
session-auth guarded, so log in first (`admin`/`admin`) and pass the token.

```bash
TOKEN=$(curl -s -X POST localhost:18080/api/user/login \
  -H 'content-type: application/json' -d '{"username":"admin","password":"admin"}' \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["token"])')

curl -s "localhost:18080/api/detect/models?token=$TOKEN"
curl -s -X POST "localhost:18080/api/detect/<pipe>/start?token=$TOKEN" \
  -H 'content-type: application/json' -d '{"models":["yolov8n","yolo11s"]}'
curl -s "localhost:18080/api/detect/<pipe>/latest?token=$TOKEN"
curl -s -X POST "localhost:18080/api/detect/<pipe>/stop?token=$TOKEN"
```

`GET /latest` returns `{ ts, frame_w, frame_h, models: [{ name, infer_ms,
detections: [{ class_id, label, bbox:{x1,y1,x2,y2}, confidence }], error }] }`.
Coordinates are original-frame pixels; scale by `frame_w`/`frame_h`.

Set `DETECT_MODELS_DIR` to point elsewhere (default `third_party/detect-models`).
Remember the ONNX Runtime dependency above — `nvr` will boot and serve
`GET /models` fine without it, but `POST /{pipe}/start` will fail to load
models until a `libonnxruntime.so` is reachable at runtime.
