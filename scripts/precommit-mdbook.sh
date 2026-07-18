#!/usr/bin/env bash
set -euo pipefail

if ! command -v mdbook &>/dev/null; then
    echo "ERROR: mdbook not found on PATH." >&2
    echo "Install with: cargo install mdbook" >&2
    exit 1
fi

exec mdbook build docs/book
