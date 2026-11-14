#!/usr/bin/env python3
"""Run a Rust build without embedding the workstation's source/home paths."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

if len(sys.argv) < 2:
    raise SystemExit("Usage: private-build.py COMMAND [ARG ...]")

env = os.environ.copy()
encoded = env.get("CARGO_ENCODED_RUSTFLAGS")
flags = encoded.split("\x1f") if encoded else env.get("RUSTFLAGS", "").split()
# The project mapping comes last so it takes precedence over its parent home.
flags += [
    f"--remap-path-prefix={Path.home()}=/home/builder",
    f"--remap-path-prefix={Path.cwd()}=/workspace/Psychotalk",
]
env["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(flags)
# Vendored OpenSSL also embeds its installation prefix as ordinary C strings,
# outside Rust's path remapping. Build it in a private, anonymous scratch root.
output = Path(env.get("CARGO_TARGET_DIR", "target")).resolve()
with tempfile.TemporaryDirectory(prefix="psychotalk-build-", dir="/tmp") as scratch:
    env["CARGO_TARGET_DIR"] = scratch
    result = subprocess.run(sys.argv[1:], env=env)
    if result.returncode:
        raise SystemExit(result.returncode)
    root = Path(scratch)
    profiles = [root / "release", *root.glob("*/release")]
    for profile in profiles:
        if not profile.exists():
            continue
        destination = output / profile.relative_to(root)
        destination.mkdir(parents=True, exist_ok=True)
        for artifact in profile.iterdir():
            if artifact.is_file() and not artifact.name.startswith(".") and (
                artifact.suffix in {".so", ".dylib", ".a", ".rlib", ".exe", ".dll"}
                or not artifact.suffix
            ):
                shutil.copy2(artifact, destination / artifact.name)
