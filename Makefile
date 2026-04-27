.DEFAULT_GOAL := help
.PHONY: help install-deps install-watch build run watch check test test-nvr test-ffmpeg-bus \
        fmt fmt-check frontend-install frontend-build frontend-dev frontend-lint \
        frontend-typecheck clean clean-frontend

PROJECT_ROOT := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
FFMPEG_DIR ?= $(PROJECT_ROOT)/ffmpeg
ZLM_DIR    ?= $(PROJECT_ROOT)/zlm
RUST_LOG   ?= info
export FFMPEG_DIR ZLM_DIR RUST_LOG
export LD_LIBRARY_PATH := $(FFMPEG_DIR)/lib:$(ZLM_DIR)/lib:$(LD_LIBRARY_PATH)

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
	cargo build --workspace

run:
	cargo run --package nvr

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
	rm -rf $(DASHBOARD_DIR)/node_modules $(DASHBOARD_DIR)/dist
