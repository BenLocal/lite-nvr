#!/usr/bin/env bash
set -euo pipefail

# FFmpeg download URLs for different platforms
# Using BtbN/FFmpeg-Builds for Linux/Windows
# For macOS, using homebrew or manual installation is recommended

FFMPEG_VERSION="7.1"
FFMPEG_BASE_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest"
ZLM_VERSION="autobuild-2026-02-02"
ZLM_BASE_URL="https://github.com/BenLocal/ZLMediaKit-Build/releases/download"

# Detect OS and Architecture
detect_platform() {
    local os arch

    # Detect OS
    case "$(uname -s)" in
        Linux*)     os="linux" ;;
        Darwin*)    os="macos" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *)          os="unknown" ;;
    esac

    # Detect Architecture
    case "$(uname -m)" in
        x86_64|amd64)   arch="x64" ;;
        aarch64|arm64)  arch="arm64" ;;
        *)              arch="unknown" ;;
    esac

    echo "${os}-${arch}"
}

# Get FFmpeg download URL for the platform
get_ffmpeg_url() {
    local platform="$1"

    case "${platform}" in
        linux-x64)
            echo "${FFMPEG_BASE_URL}/ffmpeg-n${FFMPEG_VERSION}-latest-linux64-gpl-shared-${FFMPEG_VERSION}.tar.xz"
            ;;
        linux-arm64)
            echo "${FFMPEG_BASE_URL}/ffmpeg-n${FFMPEG_VERSION}-latest-linuxarm64-gpl-shared-${FFMPEG_VERSION}.tar.xz"
            ;;
        windows-x64)
            echo "${FFMPEG_BASE_URL}/ffmpeg-n${FFMPEG_VERSION}-latest-win64-gpl-shared-${FFMPEG_VERSION}.zip"
            ;;
        windows-arm64)
            echo "${FFMPEG_BASE_URL}/ffmpeg-n${FFMPEG_VERSION}-latest-winarm64-gpl-shared-${FFMPEG_VERSION}.zip"
            ;;
        macos-*)
            # BtbN doesn't provide macOS builds, return empty
            echo ""
            ;;
        *)
            echo ""
            ;;
    esac
}

get_zlm_url() {
    local platform="$1"

    case "${platform}" in
        linux-x64)
            echo "${ZLM_BASE_URL}/${ZLM_VERSION}/zlmediakit_master_linux_amd64_latest.tar.gz"
            ;;
        linux-arm64)
            echo "${ZLM_BASE_URL}/${ZLM_VERSION}/zlmediakit_master_linux_arm64_latest.tar.gz"
            ;;
    esac
}

# Download file using curl or wget
download_file() {
    local url="$1"
    local output="$2"

    if command -v curl >/dev/null 2>&1; then
        curl -L "${url}" -o "${output}"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "${output}" "${url}"
    else
        echo "Error: curl or wget is required to download FFmpeg." >&2
        exit 1
    fi
}

# Extract archive based on file extension
extract_archive() {
    local archive="$1"
    local dest="$2"

    case "${archive}" in
        *.tar.xz)
            tar -xJf "${archive}" -C "${dest}" --strip-components=1
            ;;
        *.tar.gz)
            tar -xzf "${archive}" -C "${dest}" --strip-components=1
            ;;
        *.zip)
            # For Windows zip files
            if command -v unzip >/dev/null 2>&1; then
                local temp_dir="${dest}/_temp"
                mkdir -p "${temp_dir}"
                unzip -q "${archive}" -d "${temp_dir}"
                # Move contents from nested directory
                mv "${temp_dir}"/*/* "${dest}/" 2>/dev/null || mv "${temp_dir}"/* "${dest}/"
                rm -rf "${temp_dir}"
            else
                echo "Error: unzip is required to extract zip files." >&2
                exit 1
            fi
            ;;
        *)
            echo "Error: Unknown archive format: ${archive}" >&2
            exit 1
            ;;
    esac
}

# Setup FFmpeg for macOS using Homebrew
setup_macos_ffmpeg() {
    local ffmpeg_dir="$1"
    local ffmpeg_formula="ffmpeg@7"

    echo "=== macOS FFmpeg Setup ==="

    # Check if Homebrew is installed
    if ! command -v brew >/dev/null 2>&1; then
        echo "Error: Homebrew is not installed." >&2
        echo "Please install Homebrew first: https://brew.sh" >&2
        echo "Then run: brew install ${ffmpeg_formula}" >&2
        exit 1
    fi

    # Check if FFmpeg@7 is installed via Homebrew
    if ! brew list "${ffmpeg_formula}" >/dev/null 2>&1; then
        echo "Installing ${ffmpeg_formula} via Homebrew..."
        brew install "${ffmpeg_formula}"
    else
        echo "${ffmpeg_formula} is already installed via Homebrew."
    fi

    # Get FFmpeg prefix from Homebrew
    local ffmpeg_prefix
    ffmpeg_prefix="$(brew --prefix "${ffmpeg_formula}")"

    # Create symlinks in ffmpeg directory
    mkdir -p "${ffmpeg_dir}"

    echo "Creating symlinks from Homebrew FFmpeg..."

    # Create symlinks to Homebrew FFmpeg
    if [[ -d "${ffmpeg_prefix}/include" ]]; then
        ln -sfn "${ffmpeg_prefix}/include" "${ffmpeg_dir}/include"
    fi

    if [[ -d "${ffmpeg_prefix}/lib" ]]; then
        ln -sfn "${ffmpeg_prefix}/lib" "${ffmpeg_dir}/lib"
    fi

    if [[ -d "${ffmpeg_prefix}/bin" ]]; then
        ln -sfn "${ffmpeg_prefix}/bin" "${ffmpeg_dir}/bin"
    fi

    echo "FFmpeg setup complete. Symlinks created in: ${ffmpeg_dir}"
    echo "Homebrew FFmpeg location: ${ffmpeg_prefix}"
}

# Main function
main() {
    local root_dir ffmpeg_dir platform ffmpeg_url

    root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    ffmpeg_dir="${root_dir}/ffmpeg"

    # Allow custom URL override
    if [[ -n "${FFMPEG_URL:-}" ]]; then
        ffmpeg_url="${FFMPEG_URL}"
        platform="custom"
    else
        platform="$(detect_platform)"
        ffmpeg_url="$(get_ffmpeg_url "${platform}")"
    fi

    echo "=== FFmpeg Installation Script ==="
    echo "Platform detected: ${platform}"
    echo "FFmpeg directory: ${ffmpeg_dir}"

    # Handle macOS specially
    if [[ "${platform}" == macos-* && -z "${ffmpeg_url}" ]]; then
        setup_macos_ffmpeg "${ffmpeg_dir}"
        return 0
    fi

    # Validate URL
    if [[ -z "${ffmpeg_url}" ]]; then
        echo "Error: Unsupported platform: ${platform}" >&2
        echo "Supported platforms: linux-x64, linux-arm64, windows-x64, windows-arm64, macos-*" >&2
        exit 1
    fi

    echo "Download URL: ${ffmpeg_url}"

    # Determine archive extension
    local archive_ext
    case "${ffmpeg_url}" in
        *.tar.xz) archive_ext="tar.xz" ;;
        *.tar.gz) archive_ext="tar.gz" ;;
        *.zip)    archive_ext="zip" ;;
        *)        archive_ext="tar.xz" ;;
    esac

    local archive_path="${ffmpeg_dir}/ffmpeg-shared.${archive_ext}"

    # Create directory and download
    mkdir -p "${ffmpeg_dir}"

    echo "Downloading FFmpeg..."
    download_file "${ffmpeg_url}" "${archive_path}"

    echo "Extracting FFmpeg..."
    extract_archive "${archive_path}" "${ffmpeg_dir}"

    # Cleanup
    rm -f "${archive_path}"

    echo "=== FFmpeg installation complete ==="
    echo "Installed to: ${ffmpeg_dir}"

    # List installed files
    if [[ -d "${ffmpeg_dir}/lib" ]]; then
        echo "Libraries found:"
        ls -la "${ffmpeg_dir}/lib/" | head -10
    fi
}

# Run main function
main "$@"
