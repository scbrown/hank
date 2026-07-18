#!/usr/bin/env bash
set -euo pipefail
WARN_LIMIT=400
ERROR_LIMIT=500
errors=0; warnings=0
for file in $(git diff --cached --name-only --diff-filter=ACM | grep '\.rs$'); do
    if [[ "$file" =~ tests\.rs$ ]] || [[ "$file" =~ _test\.rs$ ]]; then continue; fi
    lines=$(wc -l < "$file")
    if [ "$lines" -gt "$ERROR_LIMIT" ]; then echo "ERROR: $file has $lines lines (limit: $ERROR_LIMIT)"; errors=$((errors + 1))
    elif [ "$lines" -gt "$WARN_LIMIT" ]; then echo "WARNING: $file has $lines lines (warn: $WARN_LIMIT)"; warnings=$((warnings + 1)); fi
done
if [ "$errors" -gt 0 ]; then echo "$errors file(s) exceed $ERROR_LIMIT lines."; exit 1; fi
if [ "$warnings" -gt 0 ]; then echo "$warnings file(s) approaching limit."; fi
exit 0
