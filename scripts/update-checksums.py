#!/usr/bin/env python3
"""Refresh only the checksums of the local UFLinux package's declared sources."""
import hashlib
from pathlib import Path
import re
import shlex
import sys

path = Path(__file__).resolve().parents[1] / "aports/uflinux/APKBUILD"
text = path.read_text()
sources = shlex.split(re.search(r'^source="([^"]+)"', text, re.M).group(1))
sums = "\n" + "\n".join(hashlib.sha512((path.parent / name).read_bytes()).hexdigest() + "  " + name for name in sources) + "\n"
updated = re.sub(r'^sha512sums="[^"]*"', lambda _: 'sha512sums="' + sums + '"', text, flags=re.M)
if "--check" in sys.argv:
    if text != updated:
        sys.exit("Local package checksums changed; run python3 scripts/update-checksums.py")
else:
    path.write_text(updated)
