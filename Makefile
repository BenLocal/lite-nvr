.DEFAULT_GOAL := help
.PHONY: help install-deps install-watch build run watch dummy check test test-nvr test-ffmpeg-bus \
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

DASHBOARD_DIR := nvr-dashboard/app

help:
	@echo "Usage: make <target>"
	@echo ""
	@echo "Setup:"
	@echo "  install-deps       Install FFmpeg & ZLMediaKit prerequisites"
	@echo "  install-watch      cargo install cargo-watch"
	@echo "  frontend-install   npm ci in $(DASHBOARD_DIR)"
	@echo ""
	@echo "Build / Run:"
	@echo "  build              cargo build --workspace"
	@echo "  run                cargo run --package nvr"
	@echo "  dummy              Run GB28181 dummy-camera (emulated IPC) vs local NVR"
	@echo "  watch              Auto-rebuild & restart nvr on .rs changes"
	@echo "  check              cargo check --workspace"
	@echo "  frontend-build     Build dashboard SPA"
	@echo "  frontend-dev       Run dashboard dev server"
	@echo ""
	@echo "Quality:"
	@echo "  test               Run all workspace tests"
	@echo "  test-nvr           Run nvr crate tests"
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
	cargo watch -w nvr -w ffmpeg-bus -w nvr-db -x 'run --package nvr'

check:
	cargo check --workspace

test:
	cargo test --workspace --lib --tests --no-fail-fast

test-nvr:
	cargo test -p nvr

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
