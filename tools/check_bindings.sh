#!/bin/sh
# ---------------------------------------------------------------------------
# check_bindings.sh - MANEDRS_PLAN.md §6 gate 2: maned-sys/src/bindings.rs
# must not drift from what bindgen derives from the frozen maned_c.h.
#
#   sh tools/check_bindings.sh [path/to/maned_c.h]     # default: ../capi-freeze
#
# The comparison is NORMALIZED, not byte-for-byte: the checked-in file
# deliberately carries doc comments, the `use core::ffi::c_char` alias and
# Copy/Clone derives that raw bindgen output does not. What must never
# differ is the ABI itself - constants, struct fields, function signatures.
# Normalization strips comments/attributes/blank lines and canonicalizes
# the c_char spelling, then diffs. First run (bindgen 0.73.2, 0.4.0 header):
# zero declaration differences.
# ---------------------------------------------------------------------------
set -eu
cd "$(dirname "$0")/.."
HDR=${1:-../capi-freeze/include/maned_c.h}
command -v bindgen >/dev/null || { echo "check_bindings: bindgen not installed (cargo install bindgen-cli)" >&2; exit 2; }

norm() {
  sed -e 's|::core::ffi::c_char|c_char|g' \
      -e 's|//.*||' -e 's|/\*.*\*/||' \
      -e 's/#\[derive(.*)\]/#[derive]/' \
      "$1" \
  | grep -vE '^\s*$|^\s*use |^/\*|^\s*\*' \
  | sed 's/^[[:space:]]*//;s/[[:space:]]*$//' \
  | grep -vE '^(unsafe )?extern "C" \{$|^\}$'
}

bindgen "$HDR" --no-layout-tests --use-core --no-doc-comments \
  --allowlist-item 'mnd_.*|MND_.*' -o /tmp/maned_bindings_gen.rs 2>/dev/null

norm /tmp/maned_bindings_gen.rs > /tmp/maned_bindings_a
norm maned-sys/src/bindings.rs > /tmp/maned_bindings_b
if diff /tmp/maned_bindings_a /tmp/maned_bindings_b > /tmp/maned_bindings_norm.diff; then
  echo "check_bindings: OK - no ABI drift vs $HDR"
else
  echo "check_bindings: DRIFT between bindgen($HDR) and maned-sys/src/bindings.rs:" >&2
  cat /tmp/maned_bindings_norm.diff >&2
  exit 1
fi
