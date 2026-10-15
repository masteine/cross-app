#!/usr/bin/env bash
# Build the release `cross` binary for Apple Silicon and stage it in $DIST_DIR.
#
#   build-cli.sh                  build, stage and archive
#   build-cli.sh --archive-only   only re-create the tarball from $DIST_DIR/cross
#                                 (after sign.sh changed the binary)
#
# Outputs:
#   $DIST_DIR/cross                                     the binary (unsigned: the linker's ad-hoc signature)
#   $DIST_DIR/cross-<version>-aarch64-apple-darwin.tar.gz
#
# Environment:
#   DIST_DIR            output directory (default: <repo>/dist)
#   TAURI_SIDECAR_DIR   if set, also copy the binary there as `cross-aarch64-apple-darwin`,
#                       the name Tauri's `bundle.externalBin = ["binaries/cross"]` expects
#                       (Sprint 6 GUI: crates/cross-gui/src-tauri/binaries)
#   DRY_RUN=1           print commands instead of running them
source "$(dirname "$0")/common.sh"

archive_only=0
case "${1:-}" in
"") ;;
--archive-only) archive_only=1 ;;
*) die "usage: build-cli.sh [--archive-only]" ;;
esac

require_cmd tar
version="$(workspace_version)"
[[ -n "$version" ]] || die "could not read the workspace version from Cargo.toml"
tarball="cross-$version-$TARGET_TRIPLE.tar.gz"

if ((archive_only)); then
    [[ -x "$DIST_DIR/cross" ]] || is_dry_run || die "$DIST_DIR/cross not found"
    run tar -C "$DIST_DIR" -czf "$DIST_DIR/$tarball" cross
    log "done: $DIST_DIR/$tarball"
    exit 0
fi

require_cmd cargo

log "building cross $version for $TARGET_TRIPLE (release profile)"
run cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --release --locked \
    -p cross-cli --target "$TARGET_TRIPLE"

built="$REPO_ROOT/target/$TARGET_TRIPLE/release/cross"
run mkdir -p "$DIST_DIR"
run cp "$built" "$DIST_DIR/cross"

if ! is_dry_run; then
    # The binary must report the version we are about to publish.
    reported="$("$DIST_DIR/cross" --version)"
    [[ "$reported" == "cross $version" ]] || die "binary reports '$reported', expected 'cross $version'"
    file "$DIST_DIR/cross" | grep -q 'arm64' || die "$DIST_DIR/cross is not an arm64 Mach-O"
fi

run tar -C "$DIST_DIR" -czf "$DIST_DIR/$tarball" cross

if [[ -n "${TAURI_SIDECAR_DIR:-}" ]]; then
    run mkdir -p "$TAURI_SIDECAR_DIR"
    run cp "$DIST_DIR/cross" "$TAURI_SIDECAR_DIR/cross-$TARGET_TRIPLE"
fi

log "done: $DIST_DIR/cross, $DIST_DIR/$tarball"
