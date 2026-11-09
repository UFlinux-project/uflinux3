#!/usr/bin/env python3
"""Refresh only the checksums of the local UFLinux package's declared sources."""
import hashlib
import json
from pathlib import Path
import re
import shlex
import sys
from urllib.parse import urlsplit

path = Path(__file__).resolve().parents[1] / "aports/uflinux/APKBUILD"
text = path.read_text()
sources = shlex.split(re.search(r'^source="([^"]+)"', text, re.M).group(1))
lock = json.loads((path.parents[2] / "config/sources.lock.json").read_text())
checksums = []
for source in sources:
    alias, separator, url = source.partition("::")
    if not separator:
        url = source
    if url.startswith("https://"):
        spec = next((spec for spec in lock["sources"].values() if spec["url"] == url), None)
        if not spec or "sha512" not in spec:
            raise SystemExit("Remote package sources require a locked SHA-512: " + source)
        name, digest = (alias if separator else Path(urlsplit(url).path).name), spec["sha512"]
    else:
        name = source
        digest = hashlib.sha512((path.parent / name).read_bytes()).hexdigest()
    checksums.append(digest + "  " + name)
sums = "\n" + "\n".join(checksums) + "\n"
updated = re.sub(r'^sha512sums="[^"]*"', lambda _: 'sha512sums="' + sums + '"', text, flags=re.M)
if "--check" in sys.argv:
    if text != updated:
        sys.exit("Local package checksums changed; run python3 scripts/update-checksums.py")
else:
    path.write_text(updated)
