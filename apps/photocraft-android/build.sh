#!/usr/bin/env sh
# Builds the PhotoCraft APK: the Rust library with cargo-ndk, then the Java shell with Gradle.
#
# Needs: Rust with the aarch64-linux-android target, cargo-ndk, the Android SDK and NDK
# (ANDROID_HOME, ANDROID_NDK_HOME), JDK 17 and Gradle 8.9+. The image in ../../../android-build
# (or any machine with those) works.
#
#   apps/photocraft-android/build.sh            # release APK
#   CARGO_BUILD_JOBS=2 apps/photocraft-android/build.sh   # less RAM, slower
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)

cd "$root"
cargo ndk --target arm64-v8a --platform 26 \
    --output-dir "$here/android/app/src/main/jniLibs" \
    build --profile android -p photocraft-android

cd "$here/android"
gradle --no-daemon assembleRelease

apk="$here/android/app/build/outputs/apk/release/app-release.apk"
echo "APK: $apk"
