#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
. scripts/env.sh
python3 scripts/extract-tor.py
sh scripts/build-core.sh
python3 scripts/package-licenses.py
export GRADLE_USER_HOME="$PWD/.tools/gradle-home"
android/gradlew -p android assembleDebug assembleDebugAndroidTest --no-daemon
mkdir -p dist
cp android/app/build/outputs/apk/debug/app-debug.apk dist/psychotalk-0.3.1-experimental-arm64.apk
shasum -a 256 dist/psychotalk-0.3.1-experimental-arm64.apk | tee dist/psychotalk-0.3.1-experimental-arm64.apk.sha256

python3 scripts/check-runtime-privacy.py dist/psychotalk-0.3.1-experimental-arm64.apk
