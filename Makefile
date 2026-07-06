.DEFAULT_GOAL := help
.PHONY: help install-deps download-asr-libs download-asr-models asr-demo xvfb install-watch build run watch dummy check test test-nvr test-nvr-asr test-ffmpeg-bus \
        fmt fmt-check frontend-install frontend-build frontend-dev frontend-lint \
        frontend-typecheck clean clean-frontend

PROJECT_ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))

ifneq (,$(wildcard $(PROJECT_ROOT)/.env))
include $(PROJECT_ROOT)/.env
export
endif

FFMPEG_DIR ?= $(PROJECT_ROOT)/ffmpeg
RUST_LOG   ?= info
export FFMPEG_DIR RUST_LOG

LD_LIBRARY_PATH := $(FFMPEG_DIR)/lib:$(LD_LIBRARY_PATH)

ifneq ($(strip $(ZLM_DIR)),)
export ZLM_DIR
LD_LIBRARY_PATH := $(LD_LIBRARY_PATH):$(ZLM_DIR)/lib
endif

export LD_LIBRARY_PATH

# Mirror uppercase proxy vars (from .env) to lowercase so curl/wget honor them
# for both http and https (curl ignores uppercase HTTP_PROXY for http URLs).
ifneq ($(strip $(HTTPS_PROXY)),)
https_proxy ?= $(HTTPS_PROXY)
export https_proxy
endif
ifneq ($(strip $(HTTP_PROXY)),)
http_proxy ?= $(HTTP_PROXY)
export http_proxy
endif

# Prebuilt sherpa-onnx libs for crates/nvr-asr. When fetched via
# `make download-asr-libs`, auto-wire SHERPA_ONNX_LIB_DIR so the workspace
# build links them offline (static libs; no runtime LD_LIBRARY_PATH needed).
SHERPA_ONNX_LIB_DIR ?= $(firstword $(wildcard $(PROJECT_ROOT)/third_party/sherpa-onnx/*/lib))
ifneq ($(strip $(SHERPA_ONNX_LIB_DIR)),)
export SHERPA_ONNX_LIB_DIR
endif

DASHBOARD_DIR := nvr-dashboard/app

help:
	@echo "Usage: make <target>"
	@echo ""
	@echo "Setup:"
	@echo "  install-deps       Install FFmpeg & ZLMediaKit prerequisites"
	@echo "  download-asr-libs  Download prebuilt sherpa-onnx libs for nvr-asr (via HTTP(S)_PROXY)"
	@echo "  download-asr-models Download silero_vad.onnx + SenseVoice model (via HTTP(S)_PROXY)"
	@echo "  install-watch      cargo install cargo-watch"
	@echo "  frontend-install   npm ci in $(DASHBOARD_DIR)"
	@echo ""
	@echo "Build / Run:"
	@echo "  build              cargo build --workspace"
	@echo "  run                cargo run --package nvr"
	@echo "  asr-demo           Run nvr-asr streaming demo on a WAV (see ASR_* vars)"
	@echo "  dummy              Run GB28181 dummy-camera (emulated IPC) vs local NVR"
	@echo "  watch              Auto-rebuild & restart nvr on .rs changes"
	@echo "  check              cargo check --workspace"
	@echo "  frontend-build     Build dashboard SPA"
	@echo "  frontend-dev       Run dashboard dev server"
	@echo "  xvfb               Start Xvfb virtual display :99 (screen-capture tests)"
	@echo ""
	@echo "Quality:"
	@echo "  test               Run all workspace tests"
	@echo "  test-nvr           Run nvr crate tests"
	@echo "  test-nvr-asr       Run nvr-asr crate tests"
	@echo "  test-ffmpeg-bus    Run ffmpeg-bus crate tests"
	@echo "  fmt                cargo fmt"
	@echo "  fmt-check          cargo fmt --check"
	@echo "  frontend-lint      Dashboard ESLint"
	@echo "  frontend-typecheck Dashboard type check"
	@echo ""
	@echo "Cleanup:"
	@echo "  clean              cargo clean"
	@echo "  clean-frontend     Remove dashboard node_modules and dist"

install-deps:
	bash scripts/pre_install_deps.sh

# Download the prebuilt sherpa-onnx native libs (crates/nvr-asr) via the proxy
# from .env (HTTP_PROXY/HTTPS_PROXY, mirrored to lowercase above). Pass script
# flags with ASR_LIBS_ARGS, e.g.:
#   make download-asr-libs ASR_LIBS_ARGS="--shared --arch aarch64"
ASR_LIBS_ARGS ?=
download-asr-libs:
	bash scripts/download_sherpa_onnx_libs.sh $(ASR_LIBS_ARGS)

# Download the ASR models (silero_vad.onnx + SenseVoice) via the .env proxy.
# Pass script flags with ASR_MODELS_ARGS, e.g.
#   make download-asr-models ASR_MODELS_ARGS="--dest /data/models"
ASR_MODELS_ARGS ?=
download-asr-models:
	bash scripts/download_asr_models.sh $(ASR_MODELS_ARGS)

# Start the Xvfb virtual display used by screen-capture tests/pipelines.
xvfb:
	bash scripts/run_xvfb.sh

install-watch:
	cargo install cargo-watch

build:
	cargo build --workspace -vv

run:
	cargo run --package nvr

# GB28181 dummy-camera (emulated IPC). Defaults target the local NVR's GB SIP
# (NVR_GB_SIP_ID / NVR_GB_PORT). Override any DUMMY_* var, and pass extra flags
# via DUMMY_ARGS, e.g.:
#   make dummy DUMMY_PASSWORD=12345678
#   make dummy DUMMY_ARGS="--source-file clip.mp4 --media-ip 172.17.0.1"
DUMMY_SERVER_ADDR ?= 127.0.0.1:5060
DUMMY_SERVER_ID   ?= 34020000002000000001
DUMMY_DEVICE_ID   ?= 34020000001320000001
DUMMY_PASSWORD    ?=
DUMMY_ARGS        ?=

dummy:
	cargo run -p dummy-camera -- \
		--server-addr $(DUMMY_SERVER_ADDR) \
		--server-id   $(DUMMY_SERVER_ID) \
		--device-id   $(DUMMY_DEVICE_ID) \
		$(if $(strip $(DUMMY_PASSWORD)),--password $(DUMMY_PASSWORD)) \
		$(DUMMY_ARGS)

watch:
	@command -v cargo-watch >/dev/null 2>&1 || { \
		echo "cargo-watch not found. Run: make install-watch"; exit 1; }
	cargo watch -w nvr -w crates/ffmpeg-bus -w nvr-db -x 'run --package nvr'

# Run the nvr-asr streaming-correction demo. Model paths are auto-discovered
# under third_party/asr-models (from `make download-asr-models`); the sherpa
# static libs come from SHERPA_ONNX_LIB_DIR (see `make download-asr-libs`).
# Override any ASR_* var, and pass extra demo flags via ASR_ARGS, e.g.:
#   make asr-demo ASR_WAV=/path/speech-16k.wav
#   make asr-demo ASR_ARGS="--language zh --partial-ms 200"
#   make asr-demo ASR_PUNCT=          # disable punctuation (raw recognizer output)
ASR_MODELS_DIR ?= $(PROJECT_ROOT)/third_party/asr-models
ASR_SENSEVOICE ?= $(firstword $(wildcard $(ASR_MODELS_DIR)/sherpa-onnx-sense-voice-*))
ASR_PUNCT_DIR  ?= $(firstword $(wildcard $(ASR_MODELS_DIR)/sherpa-onnx-punct-*))
ASR_MODEL      ?= $(ASR_SENSEVOICE)/model.int8.onnx
ASR_TOKENS     ?= $(ASR_SENSEVOICE)/tokens.txt
ASR_VAD        ?= $(ASR_MODELS_DIR)/silero_vad.onnx
ASR_PUNCT      ?= $(if $(ASR_PUNCT_DIR),$(ASR_PUNCT_DIR)/model.onnx)
ASR_WAV        ?= $(ASR_SENSEVOICE)/test_wavs/zh.wav
ASR_ARGS       ?= --realtime

asr-demo:
	@test -n "$(strip $(ASR_SENSEVOICE))" || { \
		echo "SenseVoice model not found under $(ASR_MODELS_DIR). Run: make download-asr-models"; exit 1; }
	@test -n "$(strip $(SHERPA_ONNX_LIB_DIR))" || { \
		echo "SHERPA_ONNX_LIB_DIR unset (no sherpa libs). Run: make download-asr-libs"; exit 1; }
	cargo run -p nvr-asr --bin nvr-asr-demo -- \
		--model  $(ASR_MODEL) \
		--tokens $(ASR_TOKENS) \
		--vad    $(ASR_VAD) \
		$(if $(strip $(ASR_PUNCT)),--punct $(ASR_PUNCT)) \
		--wav    $(ASR_WAV) \
		$(ASR_ARGS)

check:
	cargo check --workspace

test:
	cargo test --workspace --lib --tests --no-fail-fast

test-nvr:
	cargo test -p nvr

test-nvr-asr:
	cargo test -p nvr-asr

test-ffmpeg-bus:
	cargo test -p ffmpeg-bus

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

frontend-install:
	cd $(DASHBOARD_DIR) && npm ci

frontend-build:
	cd $(DASHBOARD_DIR) && npm ci && npm run build

frontend-dev:
	cd $(DASHBOARD_DIR) && npm run dev

frontend-lint:
	cd $(DASHBOARD_DIR) && npm run lint

frontend-typecheck:
	cd $(DASHBOARD_DIR) && npm run type-check

clean:
	cargo clean

clean-frontend:
	rm -rf $(DASHBOARD_DIR)/dist
