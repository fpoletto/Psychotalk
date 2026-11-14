#!/usr/bin/env python3
"""Preserve dependency license/notice files in the distributed APK."""
import os
import pathlib
import shutil
import tomllib

root = pathlib.Path(__file__).resolve().parent.parent
registry = pathlib.Path(os.environ.get("CARGO_HOME", root / ".tools/cargo")) / "registry/src"
destination = root / "android/app/src/main/assets/licenses/dependencies"
if destination.exists():
    shutil.rmtree(destination)
destination.mkdir(parents=True)
lock = tomllib.loads((root / "Cargo.lock").read_text())
for package in lock["package"]:
    name = package["name"] + "-" + package["version"]
    for source in registry.glob("*/" + name):
        for pattern in ("LICENSE*", "COPYING*", "NOTICE*"):
            for license_file in source.rglob(pattern):
                if license_file.is_file():
                    target = destination / name / license_file.relative_to(source)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(license_file, target)
