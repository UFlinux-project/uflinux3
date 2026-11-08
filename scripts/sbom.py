#!/usr/bin/env python3
"""SPDX inventory of the signed APK archives actually bundled in an ISO."""
from datetime import datetime, timezone
import hashlib
import io
import json
from pathlib import Path
import tarfile
import uuid

from builder import gzip_file_members, sha


def apk_metadata(path):
    for _, raw in gzip_file_members(path):
        with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
            try:
                data = archive.extractfile(".PKGINFO").read().decode()
            except KeyError:
                continue
        return dict(line.split(" = ", 1) for line in data.splitlines() if " = " in line)
    raise ValueError("Missing APK control metadata: " + str(path))


def generate(directory, destination, version, arch, epoch):
    packages = []
    for path in sorted(directory.glob("*.apk")):
        info = apk_metadata(path)
        packages.append({"SPDXID": "SPDXRef-" + hashlib.sha256(path.name.encode()).hexdigest()[:24],
                         "name": info["pkgname"], "versionInfo": info["pkgver"],
                         "downloadLocation": "NOASSERTION", "filesAnalyzed": False,
                         "licenseConcluded": "NOASSERTION", "licenseDeclared": "NOASSERTION",
                         "licenseComments": "Alpine APK license declaration: " + info.get("license", "unspecified"),
                         "copyrightText": "NOASSERTION",
                         "checksums": [{"algorithm": "SHA256", "checksumValue": sha(path)}],
                         "externalRefs": [{"referenceCategory": "PACKAGE-MANAGER", "referenceType": "purl",
                                           "referenceLocator": f"pkg:apk/alpine/{info['pkgname']}@{info['pkgver']}?arch={info.get('arch', arch)}"}]})
    identity = hashlib.sha256(json.dumps(packages, sort_keys=True).encode()).hexdigest()
    document = {"spdxVersion": "SPDX-2.3", "dataLicense": "CC0-1.0", "SPDXID": "SPDXRef-DOCUMENT",
                "name": f"UFLinux-{version}-{arch}-ISO-APK-inventory",
                "documentNamespace": "https://spdx.org/spdxdocs/" + str(uuid.uuid5(uuid.NAMESPACE_URL, identity)),
                "creationInfo": {"creators": ["Tool: UFLinux sbom.py"],
                                 "created": datetime.fromtimestamp(int(epoch), timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")},
                "documentDescribes": [p["SPDXID"] for p in packages], "packages": packages}
    destination.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n")
