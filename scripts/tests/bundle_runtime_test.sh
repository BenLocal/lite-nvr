#!/usr/bin/env bash
# Exercise recursive dependency collection with real ELF fixtures on Linux.
set -euo pipefail
scripts="$(cd "$(dirname "$0")/.." && pwd)"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
mkdir -p "$temporary/package/bin" "$temporary/package/lib" "$temporary/package/ffmpeg" "$temporary/sdk"
printf 'int leaf(void) { return 0; }\n' > "$temporary/leaf.c"
printf 'extern int leaf(void); int parent(void) { return leaf(); }\n' > "$temporary/parent.c"
printf 'extern int parent(void); int main(void) { return parent(); }\n' > "$temporary/main.c"
cc -shared -fPIC "$temporary/leaf.c" -Wl,-soname,libleaf.so -o "$temporary/sdk/libleaf.so"
cc -shared -fPIC "$temporary/parent.c" -L"$temporary/sdk" -lleaf \
    -Wl,-soname,libparent.so -o "$temporary/package/lib/libparent.so"
cc "$temporary/main.c" -L"$temporary/package/lib" -lparent \
    -Wl,-rpath-link,"$temporary/sdk" -o "$temporary/package/bin/nvr"
export LIBRARY_PATH="$temporary/sdk"
target="$(uname -m)-unknown-linux-gnu"
bash "$scripts/bundle-linux-runtime.sh" "$temporary/package" "$target"
cmp "$temporary/sdk/libleaf.so" "$temporary/package/lib/libleaf.so"
LD_LIBRARY_PATH="$temporary/package/lib" "$temporary/package/bin/nvr"
rm "$temporary/sdk/libleaf.so" "$temporary/package/lib/libleaf.so"
if bash "$scripts/bundle-linux-runtime.sh" "$temporary/package" "$target" >"$temporary/output" 2>&1; then
    echo 'Expected an unresolved dependency to fail packaging.' >&2
    exit 1
fi
grep -q 'Unresolved runtime dependency: libleaf.so' "$temporary/output"
echo 'Recursive runtime collection and missing-dependency rejection passed.'
