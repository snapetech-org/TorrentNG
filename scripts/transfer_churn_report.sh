#!/usr/bin/env bash

report_write_error() {
  # shellcheck disable=SC2034 # The caller checks both variables before exiting.
  REPORT_WRITE_FAILED="1"
  # shellcheck disable=SC2034 # The caller carries failure into the final report.
  status="FAIL"
  printf 'ERROR: unable to write transfer-churn QA report: %s\n' "$OUT" >&2
}

report_create() {
  local content="$1"
  if ! printf '%s' "$content" > "$OUT"; then
    report_write_error
    return 1
  fi
}

report_append() {
  local content="$1"
  if ! printf '%s' "$content" >> "$OUT"; then
    report_write_error
    return 1
  fi
}
