#!/usr/bin/env bash
# Submit a signed artifact to Apple's notary service, wait, then staple the ticket.
#
#   notarize.sh FILE     FILE: a signed .dmg (preferred), .zip or .pkg
#
# Credentials, first match wins (the same variable names Tauri's bundler reads):
#   NOTARY_PROFILE                                   keychain profile from `xcrun notarytool store-credentials`
#   APPLE_API_KEY_PATH + APPLE_API_KEY + APPLE_API_ISSUER   App Store Connect API key (.p8 path, key id, issuer UUID)
#   APPLE_ID + APPLE_PASSWORD + APPLE_TEAM_ID        Apple ID with an app-specific password
#
# Other environment:
#   NOTARY_TIMEOUT   passed to `notarytool submit --timeout` (default: 1h)
#   STAPLE=0         skip stapling (zip archives cannot be stapled at all)
#   DRY_RUN=1        print the commands instead of running them
#
# A bare Mach-O cannot be submitted or stapled: ship the CLI inside the notarized DMG.
# Flags checked against `xcrun notarytool submit --help` and `xcrun stapler` usage on
# this machine (Xcode CLT, macOS 26.5). UNVERIFIED end to end: no Apple Developer account.
source "$(dirname "$0")/common.sh"

(($# == 1)) || die "usage: notarize.sh FILE"
file="$1"
[[ -f "$file" ]] || die "not found: $file"
require_cmd xcrun plutil

auth=()
if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    auth=(--keychain-profile "$NOTARY_PROFILE")
elif [[ -n "${APPLE_API_KEY_PATH:-}" ]]; then
    require_env APPLE_API_KEY APPLE_API_ISSUER
    auth=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
elif [[ -n "${APPLE_ID:-}" ]]; then
    require_env APPLE_PASSWORD APPLE_TEAM_ID
    auth=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
elif is_dry_run; then
    auth=(--keychain-profile "DRY-RUN-PROFILE")
else
    die "no notarization credentials: set NOTARY_PROFILE, APPLE_API_KEY_PATH(+APPLE_API_KEY, APPLE_API_ISSUER) or APPLE_ID(+APPLE_PASSWORD, APPLE_TEAM_ID)"
fi

log "submitting $file to the notary service (this can take minutes)"
if is_dry_run; then
    # Print a redacted command: never echo a password.
    redacted=()
    for arg in "${auth[@]}"; do
        if [[ -n "${APPLE_PASSWORD:-}" && "$arg" == "$APPLE_PASSWORD" ]]; then
            redacted+=("<redacted>")
        else
            redacted+=("$arg")
        fi
    done
    run xcrun notarytool submit "$file" "${redacted[@]}" --wait \
        --timeout "${NOTARY_TIMEOUT:-1h}" --output-format json
    status="dry run"
else
    result="$(mktemp -t cross-notary)"
    trap 'rm -f "$result"' EXIT
    xcrun notarytool submit "$file" "${auth[@]}" --wait \
        --timeout "${NOTARY_TIMEOUT:-1h}" --output-format json >"$result" || true
    # UNVERIFIED: the JSON keys "id" and "status" are what notarytool is documented to
    # print, but this was never observed here. plutil reads JSON as well as plists.
    status="$(plutil -extract status raw -o - "$result" 2>/dev/null || echo unknown)"
    submission="$(plutil -extract id raw -o - "$result" 2>/dev/null || echo)"
    log "notary status: $status${submission:+ (submission $submission)}"
    if [[ "$status" != "Accepted" ]]; then
        cat "$result" >&2
        if [[ -n "$submission" ]]; then
            xcrun notarytool log "$submission" "${auth[@]}" >&2 || true
        fi
        die "notarization did not succeed"
    fi
fi

case "$file" in
*.zip)
    log "zip archives cannot be stapled; Gatekeeper checks the ticket online"
    ;;
*)
    if [[ "${STAPLE:-1}" == 1 ]]; then
        run xcrun stapler staple "$file"
        run xcrun stapler validate "$file"
    fi
    ;;
esac

if [[ "$file" == *.dmg ]]; then
    # Gatekeeper's verdict on the image as a downloaded document. `--context
    # context:primary-signature` is not in the local spctl(8) page (it comes from Apple
    # DTS guidance for disk images); spctl on macOS 26.5 accepts it and rejects an
    # unsigned image with "source=no usable signature". Informational only.
    run spctl --assess --type open --context context:primary-signature -v "$file" || \
        warn "spctl did not accept $file"
fi
log "notarized: $file ($status)"
