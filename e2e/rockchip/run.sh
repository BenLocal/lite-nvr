#!/bin/sh
# Compatibility entry; host-side implementation is Python.
exec "${RK_PYTHON:-python3}" "$(dirname "$0")/run.py" "$@"
