#!/bin/sh
# Source from the repository root. Tool installations remain local to this project.
export CARGO_HOME="$PWD/.tools/cargo"
export RUSTUP_HOME="$PWD/.tools/rustup"
export JAVA_HOME="/opt/homebrew/opt/openjdk@17/libexec/openjdk.jdk/Contents/Home"
export ANDROID_HOME="$PWD/.tools/android-sdk"
export DEVELOPER_DIR=/Library/Developer/CommandLineTools
export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk
export PATH="$CARGO_HOME/bin:$JAVA_HOME/bin:$ANDROID_HOME/platform-tools:$PATH"
