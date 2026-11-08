#!/usr/bin/env python3
"""Download the last published repository snapshot; only HTTP 404 means first release."""
import json
import os
from pathlib import Path
import subprocess

repository = os.environ["GITHUB_REPOSITORY"]
result = subprocess.run(["gh", "api", f"repos/{repository}/releases/latest"], text=True, capture_output=True)
if result.returncode:
    if "HTTP 404" not in result.stderr:
        raise RuntimeError(result.stderr)
else:
    release = json.loads(result.stdout)
    names = {asset["name"] for asset in release["assets"]}
    if "uflinux-apk-repository.tar.gz" not in names:
        raise RuntimeError("The latest release has no repository snapshot; refusing to discard update history")
    Path("out/previous").mkdir(parents=True, exist_ok=True)
    subprocess.run(["gh", "release", "download", release["tag_name"], "--repo", repository,
                    "--pattern", "uflinux-apk-repository.tar.gz", "--dir", "out/previous"], check=True)
