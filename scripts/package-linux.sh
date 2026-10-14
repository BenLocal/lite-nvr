#!/usr/bin/env bash
# Update GNU Linux release directories with cross; optionally create archives.
# Usage: [APT_MIRROR=https://mirrors.example/ubuntu] bash scripts/package-linux.sh [amd64|arm64|rockchip ...]
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p target/cross-deps dist
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
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
case "$(uname -m)" in
    arm64|aarch64) build_arch=arm64; build_toolchain=aarch64-unknown-linux-gnu ;;
    x86_64|amd64) build_arch=amd64; build_toolchain=x86_64-unknown-linux-gnu ;;
    *) echo "Unsupported build host: $(uname -m)" >&2; exit 1 ;;
esac
build_platform="linux/$build_arch"
native_image="lite-nvr-cross:host-native-$build_arch"
# Loopback proxies on macOS are reachable through Docker's host alias.
build_http_proxy="${HTTP_PROXY:-}"
build_https_proxy="${HTTPS_PROXY:-}"
if [[ "$(uname -s)" == Darwin ]]; then
    build_http_proxy="${build_http_proxy//127.0.0.1/host.docker.internal}"
    build_http_proxy="${build_http_proxy//localhost/host.docker.internal}"
    build_https_proxy="${build_https_proxy//127.0.0.1/host.docker.internal}"
    build_https_proxy="${build_https_proxy//localhost/host.docker.internal}"
fi
HTTP_PROXY="$build_http_proxy" HTTPS_PROXY="$build_https_proxy" \
docker build --platform "$build_platform" --build-arg HTTP_PROXY --build-arg HTTPS_PROXY \
    --build-arg "APT_MIRROR=${APT_MIRROR:-}" \
    -t "$native_image" -t lite-nvr-cross:host-native -f scripts/Dockerfile.cross-native scripts
echo "Build container platform: $build_platform"
python3 scripts/package-cache.py frontend
version=$(awk '/^version = / {gsub(/"/, "", $3); print $3; exit}' nvr/Cargo.toml)
sherpa_version=$(awk '/^name = "sherpa-onnx-sys"/ {found=1; next} found && /^version = / {gsub(/"/, "", $3); print $3; exit}' Cargo.lock)
architectures=("$@")
if [[ ${#architectures[@]} -eq 0 ]]; then architectures=(amd64 arm64 rockchip); fi
for arch in "${architectures[@]}"; do
    case "$arch" in amd64|arm64|rockchip) ;; *) echo "Unsupported architecture: $arch" >&2; exit 1 ;; esac
    cargo_features=()
    dependency_arch="$arch"
    if [[ "$arch" == rockchip ]]; then
        dependency_arch=arm64
        cargo_features=(--features rockchip)
    fi
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
    base_image="${!image_var:-$native_image}"
    export "$image_var=$base_image"
    export "${image_var}_TOOLCHAIN=$build_toolchain"
    deps="target/cross-deps/$arch"
    mkdir -p "$deps/ffmpeg" "$deps/zlm"
    if [[ "$arch" == rockchip ]]; then
        bash scripts/download-rockchip-ffmpeg.sh >/dev/null
    fi
    if [[ ! -f "$deps/ffmpeg/include/libavcodec/avcodec.h" ]]; then
        curl -fL --retry 3 -o "$deps/ffmpeg.archive" "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-linux${ffmpeg_arch}-gpl-shared-8.1.tar.xz"
        tar -xJf "$deps/ffmpeg.archive" -C "$deps/ffmpeg" --strip-components=1
    fi
    if [[ ! -f "$deps/zlm/lib/libmk_api.so" ]]; then
        curl -fL --retry 3 -o "$deps/zlm.archive" "https://github.com/BenLocal/ZLMediaKit-Build/releases/download/autobuild-2026-06-24/zlmediakit_master_linux_${dependency_arch}_latest.tar.gz"
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
    SHERPA_ONNX_LIB_DIR="/project/third_party/sherpa-onnx/sherpa-onnx-v${sherpa_version}-linux-$([[ "$arch" == amd64 ]] && echo x64 || echo aarch64)-static-lib/lib"
    export SHERPA_ONNX_LIB_DIR
    export PKG_CONFIG_ALLOW_CROSS=1
    export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
    export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
    export BINDGEN_EXTRA_CLANG_ARGS="--target=$target"
    sysroot=/
    if [[ "$dependency_arch" != "$build_arch" ]]; then sysroot="/usr/${rust_arch}-linux-gnu"; fi
    export BINDGEN_EXTRA_CLANG_ARGS="--target=$target --sysroot=$sysroot"
    flags="-C link-arg=-Wl,-rpath-link,/project/target/cross-deps/$dependency_arch/ffmpeg/lib -C link-arg=-Wl,-rpath,\$ORIGIN/../ffmpeg/lib -C link-arg=-Wl,-rpath,\$ORIGIN/../lib"
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="$flags"
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="$flags"
    if [[ -n "${APT_MIRROR:-}" ]]; then
        HTTP_PROXY="$build_http_proxy" HTTPS_PROXY="$build_https_proxy" \
        docker build --platform "$build_platform" --build-arg HTTP_PROXY --build-arg HTTPS_PROXY \
            --build-arg "CROSS_BASE_IMAGE=$base_image" \
            --build-arg "APT_MIRROR=$APT_MIRROR" \
            -t "lite-nvr-cross:$arch-$build_arch" -f scripts/Dockerfile.cross scripts
        if [[ "$arch" == amd64 ]]; then
            export CROSS_TARGET_X86_64_UNKNOWN_LINUX_GNU_IMAGE="lite-nvr-cross:amd64-$build_arch"
        else
            export CROSS_TARGET_AARCH64_UNKNOWN_LINUX_GNU_IMAGE="lite-nvr-cross:$arch-$build_arch"
        fi
    fi
    # Native SDK paths differ even when the Rust target triple is identical.
    build_target_dir="target/package/$arch"
    if [[ "$arch" == rockchip ]]; then
        bash scripts/with-rockchip-patch.sh cross build --release --locked -p nvr --target "$target" --target-dir "$build_target_dir" "${cargo_features[@]}"
    else
        cross build --release --locked -p nvr --target "$target" --target-dir "$build_target_dir"
    fi
    package="lite-nvr-${version}-linux-${arch}-gnu"
    if [[ "$arch" == rockchip ]]; then package="lite-nvr-${version}-linux-arm64-rockchip"; fi
    staging="dist/$package"
    metadata="target/cross-deps/package-metadata/$arch"
    mkdir -p "$metadata"
    archive="dist/$package.tar.gz"
    stamp="target/cross-deps/$package.sha256"
    binary="$build_target_dir/$target/release/nvr"
    # Include the image ID: its C++ runtimes are part of the shipped package.
    image_id=$(docker image inspect --format '{{.Id}}' "$base_image")
    signature=""
    if [[ "${PACKAGE_ARCHIVE:-0}" == 1 ]]; then
        signature=$( {
            printf '%s\n' "$image_id"
            python3 scripts/package-cache.py "$binary" "$deps" LICENSE \
                scripts/package-linux.sh scripts/package-cache.py scripts/rockchip/start.sh
        } | shasum -a 256 | awk '{print $1}')
    fi
    # Keep image-provided runtimes separately; a Rust-only update need not start Docker.
    runtime="target/cross-deps/runtime-$arch"
    if [[ ! -f "$runtime/.image-id" || "$(cat "$runtime/.image-id")" != "$image_id" ||
          ! -f "$runtime/libstdc++.so.6" || ! -f "$runtime/libgcc_s.so.1" || ! -f "$runtime/libgomp.so.1" ]]; then
        mkdir -p "$runtime"
        docker run --rm --platform "$build_platform" --cpus=1 --memory=256m \
            -v "$PWD/$runtime:/package" "$base_image" \
            bash -ec 'for lib in libstdc++.so.6 libgcc_s.so.1 libgomp.so.1; do
                path=$("$1" -print-file-name="$lib")
                cp -L "$path" "/package/$lib"
            done' _ "${rust_arch}-linux-gnu-g++"
        printf '%s\n' "$image_id" > "$runtime/.image-id"
    fi
    # Synchronize contents, retaining unchanged files and removing obsolete SDK files.
    mkdir -p "$staging/bin" "$staging/lib"
    python3 scripts/package-cache.py copy "$binary" "$staging/bin/nvr"
    python3 scripts/package-cache.py sync "$deps/ffmpeg" "$staging/ffmpeg"
    python3 scripts/package-cache.py sync-files "$staging/lib" \
        "$deps/zlm/lib/"*.so* "$deps/onnxruntime/lib/"*.so* "$runtime/"*.so*
    python3 scripts/package-cache.py copy "$deps/onnxruntime/LICENSE" "$staging/ONNXRUNTIME-LICENSE"
    if [[ -f LICENSE ]]; then python3 scripts/package-cache.py copy LICENSE "$staging/LICENSE"; fi
    if [[ "$arch" == rockchip ]]; then
        # Board launcher with run/start/stop/restart/status (POSIX sh for BusyBox).
        python3 scripts/package-cache.py copy scripts/rockchip/start.sh "$staging/start.sh"
    else
        cat > "$metadata/start.sh" <<'START'
#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")" && pwd)"
export LD_LIBRARY_PATH="$root/lib:$root/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$root/ffmpeg/bin:$PATH"
export ORT_DYLIB_PATH="${ORT_DYLIB_PATH:-$root/lib/libonnxruntime.so}"
cd "$root"
exec "$root/bin/nvr" "$@"
START
        chmod +x "$metadata/start.sh"
        python3 scripts/package-cache.py copy "$metadata/start.sh" "$staging/start.sh"
    fi
    cat > "$metadata/README.txt" <<'README'
Run ./start.sh on GNU Linux with a glibc at least as new as the build image's
(check what the binary needs: strings bin/nvr | grep -o 'GLIBC_2\.[0-9]*' | sort -V | tail -1).
Dashboard: http://localhost:18080/nvr/
FFmpeg, ZLMediaKit and ONNX Runtime CPU libraries are included.
ASR/detection model files are configured separately and are not included.
SMB transport is disabled (the default Cargo feature set).
README
    if [[ "$arch" == rockchip ]]; then
        printf '\nRockchip feature enabled; RK FFmpeg and MPP/RGA libraries included.\nRKMPP hardware requires matching Rockchip kernel drivers and device access.\n./start.sh [run|start|stop|restart|status]: run in the foreground (default) or manage a background nvr.\n' >> "$metadata/README.txt"
    fi
    python3 scripts/package-cache.py copy "$metadata/README.txt" "$staging/README.txt"
    chmod +x "$staging/start.sh"
    echo "Package directory updated: $staging"
    if [[ "${PACKAGE_ARCHIVE:-0}" == 1 ]]; then
        if [[ ! -f "$archive" || ! -f "$stamp" || "$(cat "$stamp")" != "$signature" ]]; then
            tar -czf "$archive.tmp" -C dist "$package"
            mv "$archive.tmp" "$archive"
            printf '%s\n' "$signature" > "$stamp"
        else
            echo "Archive unchanged: $archive"
        fi
    fi
done
if [[ "${PACKAGE_ARCHIVE:-0}" == 1 ]]; then
    (cd dist && shasum -a 256 lite-nvr-*-linux-*.tar.gz > SHA256SUMS)
fi
