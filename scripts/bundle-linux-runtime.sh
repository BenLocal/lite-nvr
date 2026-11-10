#!/usr/bin/env bash
# Resolve ELF dependencies with the target compiler, without executing target code.
set -euo pipefail
root="$(cd "$1" && pwd)"
compiler="${2/-unknown/}-gcc"
if [[ -f /usr/share/alsa/alsa.conf ]]; then
    mkdir -p "$root/share/alsa"
    cp -a /usr/share/alsa/. "$root/share/alsa/"
    if [[ -f /usr/share/doc/libasound2-data/copyright ]]; then
        mkdir -p "$root/licenses"
        cp /usr/share/doc/libasound2-data/copyright "$root/licenses/alsa.txt"
    fi
fi
queue=()
while IFS= read -r -d '' file; do
    if readelf -h "$file" >/dev/null 2>&1; then queue+=("$file"); fi
done < <(find "$root/bin" "$root/lib" "$root/ffmpeg" -type f -print0)
declare -A visited=()
for ((index=0; index<${#queue[@]}; index++)); do
    file="${queue[index]}"
    [[ -z "${visited[$file]:-}" ]] || continue
    visited[$file]=1
    while IFS= read -r name; do
        # glibc and the ELF loader belong to the target operating system.
        case "$name" in
            libc.so.6|libm.so.6|libpthread.so.0|libdl.so.2|librt.so.1|libresolv.so.2|libutil.so.1|ld-linux*.so.*) continue ;;
        esac
        if [[ -f "$root/lib/$name" ]]; then
            queue+=("$root/lib/$name")
        elif [[ -f "$root/ffmpeg/lib/$name" ]]; then
            queue+=("$root/ffmpeg/lib/$name")
        else
            source=$("$compiler" -print-file-name="$name")
            if [[ "$source" == "$name" || ! -f "$source" ]]; then
                echo "Unresolved runtime dependency: $name (needed by $file)" >&2
                exit 1
            fi
            cp -L "$source" "$root/lib/$name"
            queue+=("$root/lib/$name")
        fi
    done < <(readelf -d "$file" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')
done
printf 'Checked runtime dependencies of %s ELF files.\n' "${#visited[@]}"
