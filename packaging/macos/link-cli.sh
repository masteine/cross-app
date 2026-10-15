#!/usr/bin/env bash
# Put the `cross` command on your PATH.
#
#   link-cli.sh                 symlink ~/.local/bin/cross -> the CLI inside the app (no admin rights)
#   link-cli.sh --copy          copy the CLI instead (use this from the CLI-only disk image)
#   link-cli.sh --system        use /usr/local/bin instead: NEEDS ADMIN RIGHTS (asks for
#                               your password through sudo when the directory is not writable)
#   link-cli.sh --uninstall     remove what this script installed (combine with --system)
#   link-cli.sh --source PATH   the CLI to link or copy (default: see below)
#   link-cli.sh --bin-dir DIR   any other target directory
#
# Default source, first that exists: $CROSS_BIN, ./cross next to this script,
# /Applications/*.app/Contents/MacOS/cross, ~/Applications/*.app/Contents/MacOS/cross.
# A symlink into the .app keeps working after the app updates itself; a copy does not.
# This script is self-contained (it ships inside the disk image).
set -euo pipefail

bin_dir="$HOME/.local/bin"
mode=link
action=install
source_bin="${CROSS_BIN:-}"
system=0

usage() {
    sed -n '2,15s/^# \{0,1\}//p' "$0"
    exit "${1:-0}"
}
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

while (($# > 0)); do
    case "$1" in
    --copy) mode=copy ;;
    --system)
        system=1
        bin_dir=/usr/local/bin
        ;;
    --uninstall) action=uninstall ;;
    --source)
        [[ $# -ge 2 ]] || die "--source needs a path"
        source_bin="$2"
        shift
        ;;
    --bin-dir)
        [[ $# -ge 2 ]] || die "--bin-dir needs a directory"
        bin_dir="$2"
        shift
        ;;
    -h | --help) usage 0 ;;
    *)
        printf 'unknown option: %s\n\n' "$1" >&2
        usage 2
        ;;
    esac
    shift
done

target="$bin_dir/cross"

# Whether we can create or write $bin_dir: check it, or its nearest existing ancestor.
can_write_bin_dir() {
    local dir="$bin_dir"
    while [[ ! -e "$dir" && "$dir" != "/" ]]; do
        dir="$(dirname "$dir")"
    done
    [[ -w "$dir" ]]
}

# sudo only when the directory is not writable by us (e.g. /usr/local/bin on a fresh Mac).
as_needed() {
    if can_write_bin_dir; then
        "$@"
    else
        printf 'note: %s is not writable by %s; running with sudo (admin password required)\n' \
            "$bin_dir" "$(id -un)" >&2
        sudo "$@"
    fi
}

is_cross_binary() {
    [[ -f "$1" ]] && "$1" --version 2>/dev/null | grep -q '^cross '
}

if [[ "$action" == uninstall ]]; then
    if [[ -L "$target" ]]; then
        as_needed rm "$target"
        echo "removed symlink $target"
    elif [[ -f "$target" ]] && is_cross_binary "$target"; then
        as_needed rm "$target"
        echo "removed $target"
    elif [[ -e "$target" ]]; then
        die "$target is not a cross binary or symlink; leaving it alone"
    else
        echo "nothing to remove at $target"
    fi
    exit 0
fi

if [[ -z "$source_bin" ]]; then
    here="$(cd "$(dirname "$0")" && pwd)"
    candidates=("$here/cross")
    for app in /Applications/*.app "$HOME"/Applications/*.app; do
        candidates+=("$app/Contents/MacOS/cross")
    done
    for candidate in "${candidates[@]}"; do
        if is_cross_binary "$candidate"; then
            source_bin="$candidate"
            break
        fi
    done
    [[ -n "$source_bin" ]] || die "could not find the cross CLI; pass --source PATH"
fi
is_cross_binary "$source_bin" || die "$source_bin is not a working cross binary"
source_bin="$(cd "$(dirname "$source_bin")" && pwd)/$(basename "$source_bin")"

if [[ "$mode" == link && "$source_bin" == /Volumes/* ]]; then
    die "$source_bin is on a mounted disk image, which goes away on eject. Copy the app to /Applications first, or use --copy."
fi

if [[ -e "$target" || -L "$target" ]]; then
    if [[ -L "$target" ]] || is_cross_binary "$target"; then
        echo "replacing existing $target"
    else
        die "$target exists and is not a cross binary; remove it yourself first"
    fi
fi

as_needed mkdir -p "$bin_dir"
if [[ "$mode" == copy ]]; then
    as_needed rm -f "$target"
    as_needed cp "$source_bin" "$target"
    as_needed chmod 755 "$target"
    echo "copied $source_bin -> $target"
else
    as_needed ln -sfn "$source_bin" "$target"
    echo "linked $target -> $source_bin"
fi

case ":$PATH:" in
*":$bin_dir:"*) ;;
*)
    echo
    echo "$bin_dir is not on your PATH. Add it for zsh (the macOS default shell):"
    echo "  echo 'export PATH=\"$bin_dir:\$PATH\"' >> ~/.zprofile && source ~/.zprofile"
    ;;
esac

resolved="$(command -v cross 2>/dev/null || true)"
if [[ -n "$resolved" && "$resolved" != "$target" ]]; then
    echo
    echo "note: \`cross\` currently resolves to $resolved (earlier on PATH than $target)."
    if [[ "$system" == 0 && "$resolved" == "$HOME/.cargo/bin/cross" ]]; then
        echo "      That is a \`cargo install\` build; remove it with: cargo uninstall cross-cli"
    fi
fi
