#!/usr/bin/env bash
set -euo pipefail
WARN_LIMIT=400
ERROR_LIMIT=500
errors=0; warnings=0
for file in $(git diff --cached --name-only --diff-filter=ACM | grep '\.rs$'); do
    # Tests are exempt (CLAUDE.md). The name-suffix forms cover in-crate test
    # modules; the `tests/` prefix covers the integration suite, where a file is
    # named for the BINARY it drives (`tests/cli.rs`) and so matches neither
    # suffix. Without it the exemption missed the largest test files in the repo.
    if [[ "$file" =~ tests\.rs$ ]] || [[ "$file" =~ _test\.rs$ ]] || [[ "$file" == tests/* ]]; then continue; fi
    lines=$(wc -l < "$file")
    if [ "$lines" -gt "$ERROR_LIMIT" ]; then echo "ERROR: $file has $lines lines (limit: $ERROR_LIMIT)"; errors=$((errors + 1))
    elif [ "$lines" -gt "$WARN_LIMIT" ]; then echo "WARNING: $file has $lines lines (warn: $WARN_LIMIT)"; warnings=$((warnings + 1)); fi
done
if [ "$errors" -gt 0 ]; then echo "$errors file(s) exceed $ERROR_LIMIT lines."; exit 1; fi
if [ "$warnings" -gt 0 ]; then echo "$warnings file(s) approaching limit."; fi
exit 0
