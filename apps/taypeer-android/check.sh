#!/bin/sh
set -eu
cd "$(dirname "$0")"
ANDROID_PROJECT=$(pwd)
: "${ANDROID_HOME:=$ANDROID_PROJECT/.tools/android-sdk}"
: "${ANDROID_USER_HOME:=$ANDROID_PROJECT/.tools/android-user}"
: "${GRADLE_USER_HOME:=$ANDROID_PROJECT/.tools/gradle-cache}"
export ANDROID_HOME ANDROID_USER_HOME GRADLE_USER_HOME
python3 -c 'import PIL' || {
    echo "Android QA needs Pillow: python3 -m pip install -r apps/taypeer-android/requirements-qa.txt (from repository root)" >&2
    exit 1
}
python3 -m unittest discover -s . -p 'test_qa.py'
./gradlew --no-daemon :app:testDebugUnitTest :app:lintDebug :app:assembleDebug :app:assembleDebugAndroidTest "$@"
python3 verify-apk.py app/build/outputs/apk/debug/app-debug.apk
"$ANDROID_HOME/build-tools/35.0.0/zipalign" -c -P 16 4 app/build/outputs/apk/debug/app-debug.apk
