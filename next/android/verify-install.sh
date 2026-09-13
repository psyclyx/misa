#!/usr/bin/env bash
set -euo pipefail
: "${MISA_APK:?APK path is required}"
: "${ANDROID_HOME:?SDK path is required}"
android_check_dir=$(mktemp -d)
android_emulator_pid=
android_daemon_pid=
cleanup() {
  if [[ -n "$android_daemon_pid" ]]; then
    kill "$android_daemon_pid" 2>/dev/null || true
    wait "$android_daemon_pid" 2>/dev/null || true
  fi
  if [[ -n "$android_emulator_pid" ]]; then
    kill "$android_emulator_pid" 2>/dev/null || true
    wait "$android_emulator_pid" 2>/dev/null || true
  fi
  adb -P 5038 kill-server >/dev/null 2>&1 || true
  rm -rf "$android_check_dir"
}
trap cleanup EXIT
if [[ -z "${MISA_TICKET:-}" ]]; then
  : "${MISA_DAEMON:?Packaged daemon path is required}"
  MISA_CREDENTIALS="$android_check_dir/credentials.json" "$MISA_DAEMON" --open --no-relay \
    --session android-check >"$android_check_dir/daemon.log" 2>&1 &
  android_daemon_pid=$!
  for _attempt in $(seq 1 30); do
    MISA_TICKET=$(head -n1 "$android_check_dir/daemon.log")
    if [[ "$MISA_TICKET" == misa:* ]]; then break; fi
    if ! kill -0 "$android_daemon_pid" 2>/dev/null; then
      cat "$android_check_dir/daemon.log"
      exit 1
    fi
    sleep 1
  done
  [[ "$MISA_TICKET" == misa:* ]]
  MISA_TICKET=${MISA_TICKET/127.0.0.1/10.0.2.2}
fi
export ANDROID_AVD_HOME="$android_check_dir/avd"
export ANDROID_USER_HOME="$android_check_dir/android"
mkdir -p "$ANDROID_AVD_HOME" "$ANDROID_USER_HOME"
printf 'no\n' | "$ANDROID_HOME/cmdline-tools/22.0/bin/avdmanager" create avd \
  --name misa-check --package 'system-images;android-35;google_apis;x86_64' --device pixel
"$ANDROID_HOME/emulator/emulator" -avd misa-check -port 5580 -no-window -no-audio \
  -no-boot-anim -no-snapshot -gpu swiftshader_indirect >"$android_check_dir/emulator.log" 2>&1 &
android_emulator_pid=$!
adb -P 5038 start-server
android_serial=localhost:5581
for _attempt in $(seq 1 180); do
  adb -P 5038 connect "$android_serial" >/dev/null 2>&1 || true
  if [[ "$(adb -P 5038 -s "$android_serial" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == 1 ]]; then
    break
  fi
  if ! kill -0 "$android_emulator_pid" 2>/dev/null; then
    cat "$android_check_dir/emulator.log"
    exit 1
  fi
  sleep 1
done
[[ "$(adb -P 5038 -s "$android_serial" shell getprop sys.boot_completed | tr -d '\r')" == 1 ]]
adb -P 5038 -s "$android_serial" install -r "$MISA_APK"
adb -P 5038 -s "$android_serial" shell am start -W -n org.misa.app/.MainActivity
adb -P 5038 -s "$android_serial" shell pidof org.misa.app
printf 'APK installed and MainActivity launched successfully\n'

# The primary APK and its instrumentation APK were signed together.
: "${MISA_TEST_APK:?Instrumentation APK path is required}"
adb -P 5038 -s "$android_serial" install -r "$MISA_TEST_APK"
adb -P 5038 -s "$android_serial" shell am instrument -w -r -e ticket "$MISA_TICKET" \
  org.misa.app.test/org.misa.app.ClientInstrumentation | tee "$android_check_dir/tests.log"
grep -F 'INSTRUMENTATION_CODE: -1' "$android_check_dir/tests.log"
grep -F 'PASS' "$android_check_dir/tests.log"
