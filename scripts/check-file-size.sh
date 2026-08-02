#!/usr/bin/env bash
# Keep source files small (CLAUDE.md; docs/hank-spec.md §7.2).
#
# Takes its file list from ARGUMENTS when given, and falls back to the staged
# set otherwise. That fallback used to be the ONLY source, which made the
# control inert in the place it was most often run: `pre-commit run --all-files`
# leaves the index empty, so the loop body never executed and the hook reported
# "Passed" having examined nothing — while four source files sat over the hard
# limit (hank #83). Reading arguments means `--all-files` checks all files and a
# real commit still checks exactly what is being committed, since pre-commit
# passes the staged set.
set -euo pipefail
WARN_LIMIT=400
ERROR_LIMIT=500

if [ "$#" -gt 0 ]; then
    files=("$@")
else
    mapfile -t files < <(git diff --cached --name-only --diff-filter=ACM)
fi

errors=0; warnings=0; checked=0
for file in "${files[@]:-}"; do
    [[ "$file" == *.rs ]] || continue
    [ -f "$file" ] || continue
    # Tests are exempt (CLAUDE.md). The name-suffix forms cover in-crate test
    # modules; the `tests/` prefix covers the integration suite, where a file is
    # named for the BINARY it drives (`tests/cli.rs`) and so matches neither
    # suffix. Without it the exemption missed the largest test files in the repo.
    if [[ "$file" =~ tests\.rs$ ]] || [[ "$file" =~ _test\.rs$ ]] || [[ "$file" == tests/* ]]; then continue; fi
    checked=$((checked + 1))
    lines=$(wc -l < "$file")
    if [ "$lines" -gt "$ERROR_LIMIT" ]; then echo "ERROR: $file has $lines lines (limit: $ERROR_LIMIT)"; errors=$((errors + 1))
    elif [ "$lines" -gt "$WARN_LIMIT" ]; then echo "WARNING: $file has $lines lines (warn: $WARN_LIMIT)"; warnings=$((warnings + 1)); fi
done
# Report the SUBJECT COUNT, always (hank #84). A green check is two claims — "I
# looked" and "it was fine" — and this hook only ever made the second, which is
# how #83 passed over an empty input set. `checked 0 files` is self-diagnosing in
# a CI log; `Passed` is not. Print it even on success: the empty case is the one
# worth seeing, and it is exactly the case a failure-only message never reaches.
echo "check-file-size: examined $checked file(s) in this invocation (pre-commit batches; totals are the sum of these lines)"
if [ "$errors" -gt 0 ]; then echo "$errors file(s) exceed $ERROR_LIMIT lines."; exit 1; fi
if [ "$warnings" -gt 0 ]; then echo "$warnings file(s) approaching limit."; fi
exit 0
