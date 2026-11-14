#!/usr/bin/env python3
"""Reject source/home paths in all files inside release APKs and tarballs."""
from pathlib import Path
import sys
import tarfile
import zipfile

if len(sys.argv) < 2:
    raise SystemExit("Usage: check-runtime-privacy.py PACKAGE [PACKAGE ...]")

prefixes = {str(Path.home()).encode(), str(Path.cwd()).encode()}
findings = []


def check(package, name, data):
    if any(prefix in data for prefix in prefixes):
        findings.append(f"{package.name}: {name}")


for argument in sys.argv[1:]:
    package = Path(argument)
    if zipfile.is_zipfile(package):
        with zipfile.ZipFile(package) as archive:
            for name in archive.namelist():
                check(package, name, archive.read(name))
    else:
        with tarfile.open(package) as archive:
            for member in archive:
                if member.isfile():
                    check(package, member.name, archive.extractfile(member).read())

if findings:
    raise SystemExit("Local build paths found in:\n" + "\n".join(findings))
print("PASS: no current workstation source/home prefixes in release contents")
