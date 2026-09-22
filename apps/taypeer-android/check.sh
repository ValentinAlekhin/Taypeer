#!/bin/sh
set -eu
cd "$(dirname "$0")"
ANDROID_PROJECT=$(pwd)
: "${ANDROID_HOME:=$ANDROID_PROJECT/.tools/android-sdk}"
: "${GRADLE_USER_HOME:=$ANDROID_PROJECT/.tools/gradle-cache}"
export ANDROID_HOME GRADLE_USER_HOME
./gradlew --no-daemon :app:testDebugUnitTest :app:lintDebug :app:assembleDebug :app:assembleDebugAndroidTest "$@"
python3 verify-apk.py app/build/outputs/apk/debug/app-debug.apk
"$ANDROID_HOME/build-tools/35.0.0/zipalign" -c -P 16 4 app/build/outputs/apk/debug/app-debug.apk
