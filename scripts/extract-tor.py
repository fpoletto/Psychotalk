#!/usr/bin/env python3
"""Fetch the pinned Guardian AAR and package only its native executable."""
import hashlib
import pathlib
import urllib.request
import zipfile

root = pathlib.Path(__file__).resolve().parent.parent
archive = root / ".tools/downloads/tor-android.aar"
archive.parent.mkdir(parents=True, exist_ok=True)
expected = "28ff2bb6d4d42eabe57183dedd7a97fce77e8ce4b6f5af6fa03993b3ca054edd"
if not archive.exists():
    urllib.request.urlretrieve(
        "https://raw.githubusercontent.com/guardianproject/gpmaven/master/"
        "info/guardianproject/tor-android/0.4.9.14/tor-android-0.4.9.14.aar", archive
    )
if hashlib.file_digest(archive.open("rb"), "sha256").hexdigest() != expected:
    raise SystemExit("Guardian AAR checksum mismatch; build stopped")
destination = root / "android/app/src/main/jniLibs/arm64-v8a/libtor.so"
destination.parent.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(archive) as contents:
    destination.write_bytes(contents.read("jni/arm64-v8a/libtor.so"))
