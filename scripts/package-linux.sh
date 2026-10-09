#!/usr/bin/env bash
# Build GNU Linux release packages with cross; keep native dependencies per target.
# Usage: [APT_MIRROR=https://mirrors.example/ubuntu] bash scripts/package-linux.sh [amd64|arm64 ...]
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/cross-deps dist
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"
export CROSS_CONTAINER_OPTS="${CROSS_CONTAINER_OPTS:---cpus=2 --memory=4g} --volume \"$PWD/target/cross-deps:/project/target/cross-deps:ro\" --volume \"$PWD/third_party:/project/third_party:ro\""
command -v cross >/dev/null
# An optional mirror layer derives from the official cross image.
if [[ -n "${APT_MIRROR:-}" ]]; then
    case "$APT_MIRROR" in
        http://*|https://*) ;;
        *) echo 'APT_MIRROR must be an http(s) Ubuntu archive URL' >&2; exit 1 ;;
    esac
fi
docker info >/dev/null
(cd nvr-dashboard/app && npm ci && npm run build)
version=$(awk '/^version = / {gsub(/"/, "", $3); print $3; exit}' nvr/Cargo.toml)
sherpa_version=$(awk '/^name = "sherpa-onnx-sys"/ {found=1; next} found && /^version = / {gsub(/"/, "", $3); print $3; exit}' Cargo.lock)
architectures=("$@")
if [[ ${#architectures[@]} -eq 0 ]]; then architectures=(amd64 arm64); fi
for arch in "${architectures[@]}"; do
    case "$arch" in amd64|arm64) ;; *) echo "Unsupported architecture: $arch" >&2; exit 1 ;; esac
    if [[ "$arch" == amd64 ]]; then
        rust_arch=x86_64
        ffmpeg_arch=64
        image_var=CROSS_TARGET_X86_64_UNKNOWN_LINUX_GNU_IMAGE
    else
        rust_arch=aarch64
        ffmpeg_arch=arm64
        image_var=CROSS_TARGET_AARCH64_UNKNOWN_LINUX_GNU_IMAGE
    fi
    target="${rust_arch}-unknown-linux-gnu"
    base_image="${!image_var:-ghcr.io/cross-rs/$target:main}"
    export "$image_var=$base_image"
    deps="target/cross-deps/$arch"
    mkdir -p "$deps/ffmpeg" "$deps/zlm"
    if [[ ! -f "$deps/ffmpeg/include/libavcodec/avcodec.h" ]]; then
        curl -fL --retry 3 -o "$deps/ffmpeg.archive" "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-linux${ffmpeg_arch}-gpl-shared-8.1.tar.xz"
        tar -xJf "$deps/ffmpeg.archive" -C "$deps/ffmpeg" --strip-components=1
    fi
    if [[ ! -f "$deps/zlm/lib/libmk_api.so" ]]; then
        curl -fL --retry 3 -o "$deps/zlm.archive" "https://github.com/BenLocal/ZLMediaKit-Build/releases/download/autobuild-2026-06-24/zlmediakit_master_linux_${arch}_latest.tar.gz"
        tar -xzf "$deps/zlm.archive" -C "$deps/zlm" --strip-components=1
    fi
    if [[ ! -f "$deps/onnxruntime/lib/libonnxruntime.so" ]]; then
        ort_arch=$([[ "$arch" == amd64 ]] && echo x64 || echo aarch64)
        mkdir -p "$deps/onnxruntime"
        curl -fL --retry 3 -o "$deps/onnxruntime.archive" "https://github.com/microsoft/onnxruntime/releases/download/v1.22.0/onnxruntime-linux-${ort_arch}-1.22.0.tgz"
        tar -xzf "$deps/onnxruntime.archive" -C "$deps/onnxruntime" --strip-components=1
    fi
    bash scripts/download_sherpa_onnx_libs.sh --os linux --arch "$rust_arch"
    export FFMPEG_DIR="/project/$deps/ffmpeg" ZLM_DIR="/project/$deps/zlm"
    export SHERPA_ONNX_LIB_DIR="/project/third_party/sherpa-onnx/sherpa-onnx-v${sherpa_version}-linux-$([[ "$arch" == amd64 ]] && echo x64 || echo aarch64)-static-lib/lib"
    export PKG_CONFIG_ALLOW_CROSS=1
    export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
    export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
    export BINDGEN_EXTRA_CLANG_ARGS="--target=$target"
    if [[ "$arch" == arm64 ]]; then
        export BINDGEN_EXTRA_CLANG_ARGS="--target=aarch64-linux-gnu --sysroot=/usr/aarch64-linux-gnu"
    fi
    flags="-C link-arg=-Wl,-rpath-link,$FFMPEG_DIR/lib -C link-arg=-Wl,-rpath,\$ORIGIN/../ffmpeg/lib -C link-arg=-Wl,-rpath,\$ORIGIN/../lib"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="$flags"
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="$flags"
    if [[ -n "${APT_MIRROR:-}" ]]; then
        docker build --platform linux/amd64 \
            --build-arg "CROSS_BASE_IMAGE=$base_image" \
            --build-arg "APT_MIRROR=$APT_MIRROR" \
            -t "lite-nvr-cross:$arch" -f scripts/Dockerfile.cross scripts
        if [[ "$arch" == amd64 ]]; then
            export CROSS_TARGET_X86_64_UNKNOWN_LINUX_GNU_IMAGE=lite-nvr-cross:amd64
        else
            export CROSS_TARGET_AARCH64_UNKNOWN_LINUX_GNU_IMAGE=lite-nvr-cross:arm64
        fi
    fi
    cross build --release --locked -p nvr --target "$target"
    package="lite-nvr-${version}-linux-${arch}-gnu"
    staging="target/cross-deps/$package"
    mkdir -p "$staging/bin" "$staging/lib"
    cp "target/$target/release/nvr" "$staging/bin/nvr"
    mkdir -p "$staging/ffmpeg"
    cp -R "$deps/ffmpeg/." "$staging/ffmpeg/"
    cp -P "$deps/zlm/lib/"*.so* "$staging/lib/"
    cp -P "$deps/onnxruntime/lib/"*.so* "$staging/lib/"
    # Bundle the target C++/OpenMP runtimes; glibc comes from the deployment OS.
    docker run --rm --platform linux/amd64 --cpus=1 --memory=256m \
        -v "$PWD/$staging/lib:/package" "$base_image" \
        bash -c 'for lib in libstdc++.so.6 libgcc_s.so.1 libgomp.so.1; do
            path=$("$1" -print-file-name="$lib")
            cp -L "$path" "/package/$lib"
        done' _ "${rust_arch}-linux-gnu-g++"
    cp "$deps/onnxruntime/LICENSE" "$staging/ONNXRUNTIME-LICENSE"
    if [[ -f LICENSE ]]; then cp LICENSE "$staging/"; fi
    cat > "$staging/start.sh" <<'START'
#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")" && pwd)"
export LD_LIBRARY_PATH="$root/lib:$root/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$root/ffmpeg/bin:$PATH"
export ORT_DYLIB_PATH="${ORT_DYLIB_PATH:-$root/lib/libonnxruntime.so}"
cd "$root"
exec "$root/bin/nvr" "$@"
START
    cat > "$staging/README.txt" <<'README'
Run ./start.sh on GNU Linux (Ubuntu 24.04 or a compatible glibc 2.39+ system).
Dashboard: http://localhost:18080/nvr/
FFmpeg, ZLMediaKit and ONNX Runtime CPU libraries are included.
ASR/detection model files are configured separately and are not included.
SMB transport is disabled (the default Cargo feature set).
README
    chmod +x "$staging/start.sh"
    tar -czf "dist/$package.tar.gz" -C target/cross-deps "$package"
done
(cd dist && shasum -a 256 lite-nvr-*-linux-*-gnu.tar.gz > SHA256SUMS)
