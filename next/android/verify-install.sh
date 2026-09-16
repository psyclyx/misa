#!/usr/bin/env bash
set -euo pipefail
: "${MISA_APK:?APK path is required}"
: "${ANDROID_HOME:?SDK path is required}"
android_check_dir=$(mktemp -d)
mkdir -m 700 "$android_check_dir/runtime" "$android_check_dir/state"
export XDG_RUNTIME_DIR="$android_check_dir/runtime" XDG_STATE_HOME="$android_check_dir/state" MISA_PREFS="$android_check_dir/preferences.json"
android_emulator_pid=
android_daemon_pid=
android_second_pid=
cleanup() {
  local result=$?
  if [[ -n "$android_second_pid" ]]; then
    kill "$android_second_pid" 2>/dev/null || true
    wait "$android_second_pid" 2>/dev/null || true
  fi
  if [[ -n "$android_daemon_pid" ]]; then
    kill "$android_daemon_pid" 2>/dev/null || true
    wait "$android_daemon_pid" 2>/dev/null || true
  fi
  if [[ -n "$android_emulator_pid" ]]; then
    kill "$android_emulator_pid" 2>/dev/null || true
    wait "$android_emulator_pid" 2>/dev/null || true
  fi
  adb -P 5038 kill-server >/dev/null 2>&1 || true
  if [[ "$result" != 0 ]]; then
    rm -rf "$android_check_dir/avd" "$android_check_dir/android"
    printf 'Failure artifacts retained: %s\n' "$android_check_dir"
  else
    rm -rf "$android_check_dir"
  fi
}
trap cleanup EXIT
if [[ -z "${MISA_TICKET:-}" ]]; then
  : "${MISA_DAEMON:?Packaged daemon path is required}"
  android_plugin_args=()
  if [[ -n "${MISA_PLUGIN:-}" ]]; then android_plugin_args=(--plugin "$MISA_PLUGIN"); fi
  MISA_CREDENTIALS="$android_check_dir/credentials.json" "$MISA_DAEMON" --open --no-relay \
    --session android-check "${android_plugin_args[@]}" >"$android_check_dir/daemon.log" 2>&1 &
  android_daemon_pid=$!
  for _attempt in $(seq 1 30); do
    MISA_TICKET=$(sed -n '/^misa:/{p;q;}' "$android_check_dir/daemon.log")
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
if [[ -z "${MISA_TICKET2:-}" && -n "${MISA_DAEMON:-}" ]]; then
  "$MISA_DAEMON" --open --no-relay --tool-approval ask --session android-check >"$android_check_dir/second.log" 2>&1 &
  android_second_pid=$!
  for _attempt in $(seq 1 30); do
    MISA_TICKET2=$(sed -n '/^misa:/{p;q;}' "$android_check_dir/second.log")
    if [[ "$MISA_TICKET2" == misa:* ]]; then break; fi
    sleep 1
  done
  [[ "$MISA_TICKET2" == misa:* ]]
  MISA_TICKET2=${MISA_TICKET2/127.0.0.1/10.0.2.2}
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
adb -P 5038 -s "$android_serial" shell am instrument -w -r -e ticket "$MISA_TICKET" -e ticket2 "${MISA_TICKET2:-}" -e plugin "${MISA_PLUGIN:+true}" \
  org.misa.app.test/org.misa.app.ClientInstrumentation | tee "$android_check_dir/tests.log"
grep -F 'INSTRUMENTATION_CODE: -1' "$android_check_dir/tests.log"
grep -F 'PASS' "$android_check_dir/tests.log"
if [[ -n "${MISA_CHECK_ARTIFACTS:-}" ]]; then
  mkdir -p "$MISA_CHECK_ARTIFACTS"
  adb -P 5038 -s "$android_serial" shell am start -W -n org.misa.app/.MainActivity --es ticket "$MISA_TICKET"
  sleep 5
  adb -P 5038 -s "$android_serial" exec-out screencap -p >"$MISA_CHECK_ARTIFACTS/session.png"
  adb -P 5038 -s "$android_serial" shell uiautomator dump /sdcard/misa-window.xml
  adb -P 5038 -s "$android_serial" pull /sdcard/misa-window.xml "$MISA_CHECK_ARTIFACTS/window.xml"
  cp "$android_check_dir/tests.log" "$MISA_CHECK_ARTIFACTS/tests.log"
fi
