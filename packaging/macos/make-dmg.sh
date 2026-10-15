#!/usr/bin/env bash
# Build the distributable disk image with hdiutil.
#
# Two layouts:
#   CLI only (today):   cross, link-cli.sh, README.txt
#   GUI (Sprint 6):     set APP_BUNDLE=/path/to/Cross.app -> the .app plus an
#                       /Applications symlink; the CLI ships inside the bundle at
#                       Contents/MacOS/cross (Tauri externalBin) and link-cli.sh links it.
#
# Environment:
#   CLI_BIN       CLI binary to ship (default: $DIST_DIR/cross from build-cli.sh)
#   APP_BUNDLE    GUI .app bundle; switches to the GUI layout (hook for Sprint 6)
#   DMG_NAME      output file name (default: cross-app-<version>-arm64.dmg)
#   DMG_FORMAT    hdiutil -format (default: UDZO, zlib; ULFO/ULMO also work on macOS 14+)
#   VERIFY_MOUNT  1 (default): attach the image read-only, check its contents, detach
#   DIST_DIR      output directory (default: <repo>/dist)
#   DRY_RUN=1     print commands instead of running them
#
# Sign the result with sign.sh and notarize it with notarize.sh afterwards.
source "$(dirname "$0")/common.sh"

require_cmd hdiutil
version="$(workspace_version)"
[[ -n "$version" ]] || die "could not read the workspace version from Cargo.toml"
CLI_BIN="${CLI_BIN:-$DIST_DIR/cross}"
DMG_NAME="${DMG_NAME:-cross-app-$version-arm64.dmg}"
DMG_FORMAT="${DMG_FORMAT:-UDZO}"
VERIFY_MOUNT="${VERIFY_MOUNT:-1}"
volume="cross-app $version"
out="$DIST_DIR/$DMG_NAME"

stage="$(mktemp -d -t cross-dmg)"
mountpoint=""
cleanup() {
    if [[ -n "$mountpoint" ]]; then
        hdiutil detach "$mountpoint" -quiet || hdiutil detach "$mountpoint" -force -quiet || true
        rmdir "$mountpoint" 2>/dev/null || true
    fi
    rm -rf "$stage"
}
trap cleanup EXIT

expected=()
if [[ -n "${APP_BUNDLE:-}" ]]; then
    [[ -d "$APP_BUNDLE/Contents" ]] || die "APP_BUNDLE is not an .app bundle: $APP_BUNDLE"
    app_name="$(basename "${APP_BUNDLE%/}")"
    [[ -x "$APP_BUNDLE/Contents/MacOS/cross" ]] ||
        warn "$app_name has no Contents/MacOS/cross; link-cli.sh will not find the CLI"
    log "GUI layout: $app_name + /Applications link"
    # ditto keeps code signatures, extended attributes and symlinks intact.
    run ditto "$APP_BUNDLE" "$stage/$app_name"
    run ln -s /Applications "$stage/Applications"
    expected=("$app_name/Contents/Info.plist" "Applications")
else
    is_dry_run || [[ -x "$CLI_BIN" ]] || die "CLI binary not found: $CLI_BIN (run build-cli.sh first)"
    log "CLI layout: $(basename "$CLI_BIN")"
    run ditto "$CLI_BIN" "$stage/cross"
    expected=("cross")
fi
run cp "$PACKAGING_DIR/link-cli.sh" "$stage/link-cli.sh"
expected+=("link-cli.sh" "README.txt")
if ! is_dry_run; then
    if [[ -n "${APP_BUNDLE:-}" ]]; then
        sed -e "s/@VERSION@/$version/g" -e "s/@APP_NAME@/$app_name/g" \
            "$PACKAGING_DIR/dmg-readme-gui.txt" >"$stage/README.txt"
    else
        sed "s/@VERSION@/$version/g" "$PACKAGING_DIR/dmg-readme.txt" >"$stage/README.txt"
    fi
fi

run mkdir -p "$DIST_DIR"
log "creating $out ($DMG_FORMAT)"
run hdiutil create -volname "$volume" -srcfolder "$stage" -ov -format "$DMG_FORMAT" "$out"
run hdiutil verify "$out"

if [[ "$VERIFY_MOUNT" == 1 ]] && ! is_dry_run; then
    mountpoint="$(mktemp -d -t cross-dmg-mnt)"
    hdiutil attach "$out" -readonly -nobrowse -noautoopen -mountpoint "$mountpoint" -quiet
    for item in "${expected[@]}"; do
        [[ -e "$mountpoint/$item" || -L "$mountpoint/$item" ]] || die "missing from image: $item"
    done
    if [[ -z "${APP_BUNDLE:-}" ]]; then
        "$mountpoint/cross" --version >/dev/null || die "the CLI inside the image does not run"
    fi
    log "mounted check passed: ${expected[*]}"
    hdiutil detach "$mountpoint" -quiet
    rmdir "$mountpoint" 2>/dev/null || true
    mountpoint=""
fi

# No checksum here: signing and stapling change the file. Hash the final artifact.
log "done: $out"
