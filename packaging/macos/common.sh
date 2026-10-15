# shellcheck shell=bash
# Shared helpers for the packaging scripts. Source it; do not run it.
#
#   DRY_RUN=1   print every command that would change something instead of running it
#   DIST_DIR    output directory (default: <repo>/dist)

set -euo pipefail

PACKAGING_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$PACKAGING_DIR/../.." && pwd)"
DIST_DIR="${DIST_DIR:-$REPO_ROOT/dist}"
DRY_RUN="${DRY_RUN:-0}"
# Rust target of every shipped binary: Apple Silicon only (see SPRINT_PLAN.md, not-a-goal: Intel).
TARGET_TRIPLE="aarch64-apple-darwin"

log() { printf '==> %s\n' "$*" >&2; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

is_dry_run() { [[ "$DRY_RUN" == 1 ]]; }

# Run a command, or only print it (shell-quoted) under DRY_RUN=1.
run() {
    if is_dry_run; then
        printf '[dry-run]' >&2
        printf ' %q' "$@" >&2
        printf '\n' >&2
    else
        "$@"
    fi
}

# Fail unless every named environment variable is non-empty.
require_env() {
    local missing=() name
    for name in "$@"; do
        [[ -n "${!name:-}" ]] || missing+=("$name")
    done
    ((${#missing[@]} == 0)) || die "missing environment variable(s): ${missing[*]}"
}

require_cmd() {
    local name
    for name in "$@"; do
        command -v "$name" >/dev/null 2>&1 || die "required tool not found: $name"
    done
}

# Version of the workspace, read from the root Cargo.toml ([workspace.package] version).
workspace_version() {
    sed -n '/^\[workspace.package\]/,/^\[/{s/^version *= *"\(.*\)"/\1/p;}' "$REPO_ROOT/Cargo.toml" | head -n 1
}
