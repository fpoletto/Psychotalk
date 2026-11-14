#!/bin/sh
set -eu
. scripts/env.sh
ndk="$ANDROID_HOME/ndk/28.2.13676358/toolchains/llvm/prebuilt/darwin-x86_64/bin"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ndk/aarch64-linux-android29-clang"
export CC_aarch64_linux_android="aarch64-linux-android29-clang"
export AR_aarch64_linux_android="$ndk/llvm-ar"
export RANLIB="$ndk/llvm-ranlib"
export RANLIB_aarch64_linux_android="$ndk/llvm-ranlib"
export PATH="$ndk:$PATH"
python3 scripts/private-build.py cargo build --locked --release -p psychotalk-core --target aarch64-linux-android --lib
mkdir -p android/app/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/release/libpsychotalk_core.so android/app/src/main/jniLibs/arm64-v8a/
