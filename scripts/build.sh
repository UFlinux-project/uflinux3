#!/bin/sh
# The same entry point is used locally and on GitHub-hosted runners.
set -eu
exec python3 "$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)/builder.py" "$@"
