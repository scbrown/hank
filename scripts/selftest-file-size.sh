#!/usr/bin/env bash
# selftest-file-size.sh — prove the size ratchet still BITES after grandfathering.
#
# The danger in fixing an always-red check is making it always-green instead. That would be
# strictly worse: the old state at least annoyed someone. Every case below is one the hook
# must still fail, plus the two it must not. aegis-1gy64.
set -uo pipefail
SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOOK="$SELF_DIR/check-file-size.sh"
TMP="$(mktemp -d -t fsz-XXXXXX)"; trap 'rm -rf "$TMP"' EXIT
pass=0; fail=0
ok(){ printf '  \033[32mok\033[0m   %s\n' "$*"; pass=$((pass+1)); }
bad(){ printf '  \033[31mFAIL\033[0m %s\n' "$*"; fail=$((fail+1)); }
check(){ if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (expected '$3', got '$2')"; fi; }

mkfile(){ : > "$TMP/$1"; for _ in $(seq "$2"); do echo "// line" >> "$TMP/$1"; done; }
run(){ ( cd "$TMP" && FILE_SIZE_BASELINE="$TMP/base.txt" "$HOOK" "$@" >"$TMP/out" 2>&1; echo $? ); }

printf 'grandfathered.rs\t600\n' > "$TMP/base.txt"

echo "== a NEW file over the hard limit must FAIL"
mkfile new.rs 501
check "exit 1"            "$(run new.rs)" "1"
check "names the limit"   "$(grep -c 'limit: 500' "$TMP/out")" "1"

echo "== a NEW file under the limit passes"
mkfile small.rs 100
check "exit 0"            "$(run small.rs)" "0"

echo "== a GRANDFATHERED file that GREW must FAIL"
mkfile grandfathered.rs 601
check "exit 1"            "$(run grandfathered.rs)" "1"
check "says it grew"      "$(grep -c 'may shrink, never grow' "$TMP/out")" "1"

echo "== a GRANDFATHERED file at exactly its frozen size passes"
mkfile grandfathered.rs 600
check "exit 0"            "$(run grandfathered.rs)" "0"

echo "== a GRANDFATHERED file that SHRANK (still over limit) passes, silently"
mkfile grandfathered.rs 550
check "exit 0"            "$(run grandfathered.rs)" "0"
check "no NOTICE yet"     "$(grep -c NOTICE "$TMP/out")" "0"

echo "== a GRANDFATHERED file now UNDER the limit passes and asks to be delisted"
mkfile grandfathered.rs 400
check "exit 0"            "$(run grandfathered.rs)" "0"
check "NOTICE to delist"  "$(grep -c 'drop it from' "$TMP/out")" "1"

echo "== tests are still exempt"
mkfile foo_test.rs 900
check "exit 0"            "$(run foo_test.rs)" "0"

echo "== the REAL tree is green (the whole point) "
check "repo passes"       "$(cd "$SELF_DIR/.." && ./scripts/check-file-size.sh $(git ls-files '*.rs') >/dev/null 2>&1; echo $?)" "0"

echo
if [ "$fail" -eq 0 ]; then printf 'selftest: %d passed, 0 failed\n' "$pass"; exit 0; fi
printf 'selftest: %d passed, \033[31m%d failed\033[0m\n' "$pass" "$fail"; exit 1
