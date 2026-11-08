#!/bin/sh
set -eu
build=${1:?A private build directory is required}
mkdir -p "$build"
source_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
xml=$source_dir/protocols/wlr-virtual-pointer-unstable-v1.xml
wayland-scanner client-header "$xml" "$build/pointer-protocol.h"
wayland-scanner private-code "$xml" "$build/pointer-protocol.c"
cc -O2 -Wall -Wextra -Werror -I"$build" "$source_dir/pointer-driver.c" "$build/pointer-protocol.c" -lwayland-client -o "$build/pointer-driver"
