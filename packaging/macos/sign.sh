#!/usr/bin/env bash
# Sign cross-app artifacts with a Developer ID and the hardened runtime.
#
#   sign.sh PATH...      PATH: a Mach-O binary (e.g. dist/cross), an .app bundle or a .dmg
#
# Environment:
#   SIGN_IDENTITY      required: "Developer ID Application: Name (TEAMID)", its SHA-1 hash,
#                      or "-" for an ad-hoc signature (local testing only: Gatekeeper
#                      rejects ad-hoc signed downloads and notarization refuses them)
#   SIGN_KEYCHAIN      optional keychain file to take the identity from (CI)
#   ENTITLEMENTS       entitlements plist (default: packaging/macos/entitlements.plist)
#   CLI_IDENTIFIER     optional code-signing identifier for a bare binary (codesign -i);
#                      without it codesign derives one from the file name ("cross")
#   DRY_RUN=1          print the codesign commands instead of running them
#
# Order matters for bundles: nested code first, the bundle last (inside out); `--deep`
# is deprecated for signing (man codesign), so nested executables are signed explicitly.
#
# UNVERIFIED: never run with a real Developer ID on this project yet (no certificate).
source "$(dirname "$0")/common.sh"

(($# > 0)) || die "usage: sign.sh PATH..."
ENTITLEMENTS="${ENTITLEMENTS:-$PACKAGING_DIR/entitlements.plist}"
require_cmd codesign plutil
[[ -f "$ENTITLEMENTS" ]] || die "entitlements not found: $ENTITLEMENTS"
plutil -lint "$ENTITLEMENTS" >/dev/null || die "invalid entitlements plist: $ENTITLEMENTS"

if is_dry_run; then
    SIGN_IDENTITY="${SIGN_IDENTITY:-Developer ID Application: DRY RUN (TEAMID)}"
else
    require_env SIGN_IDENTITY
fi

common_args=(--force --sign "$SIGN_IDENTITY")
if [[ "$SIGN_IDENTITY" != "-" ]]; then
    # Notarization requires a secure timestamp; Apple's timestamp server does not
    # serve ad-hoc signatures, so skip it for "-".
    common_args+=(--timestamp)
fi
if [[ -n "${SIGN_KEYCHAIN:-}" ]]; then
    common_args+=(--keychain "$SIGN_KEYCHAIN")
fi

# Executable code: hardened runtime + entitlements.
sign_code() {
    local path="$1"
    shift
    run codesign "${common_args[@]}" --options runtime --entitlements "$ENTITLEMENTS" "$@" "$path"
}

verify() {
    run codesign --verify --strict --verbose=2 "$1"
}

is_macho() { file -b "$1" | grep -q 'Mach-O'; }

sign_bundle() {
    local app="$1" nested
    # Helpers first (e.g. Contents/MacOS/cross next to the GUI executable), then frameworks.
    # The main executable is sealed by signing the bundle itself.
    local main
    main="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$app/Contents/Info.plist" 2>/dev/null || true)"
    while IFS= read -r -d '' nested; do
        if [[ "$(basename "$nested")" != "$main" ]] && is_macho "$nested"; then
            sign_code "$nested"
        fi
    done < <(find "$app/Contents/MacOS" -type f -print0 2>/dev/null)
    while IFS= read -r -d '' nested; do
        sign_code "$nested"
    done < <(find "$app/Contents/Frameworks" -maxdepth 1 \( -name '*.framework' -o -name '*.dylib' \) -print0 2>/dev/null)
    sign_code "$app"
    verify "$app"
}

for path in "$@"; do
    [[ -e "$path" ]] || die "not found: $path"
    case "$path" in
    *.app | *.app/)
        log "signing bundle $path"
        sign_bundle "${path%/}"
        ;;
    *.dmg)
        # A disk image is not executable code: no hardened runtime, no entitlements.
        log "signing disk image $path"
        run codesign "${common_args[@]}" "$path"
        verify "$path"
        ;;
    *)
        is_macho "$path" || die "not a Mach-O binary, .app or .dmg: $path"
        log "signing binary $path"
        if [[ -n "${CLI_IDENTIFIER:-}" ]]; then
            sign_code "$path" --identifier "$CLI_IDENTIFIER"
        else
            sign_code "$path"
        fi
        verify "$path"
        ;;
    esac
done
