#!/usr/bin/env bash
#
# set-apple-secrets.sh - convert Apple signing material into the form the
# Release workflow expects and push it to the repo's GitHub Actions secrets.
#
# Sets these six secrets (see docs/release-secrets.md):
#   APPLE_CERTIFICATE          base64 of the Developer ID Application .p12
#   APPLE_CERTIFICATE_PASSWORD the .p12 password
#   APPLE_SIGNING_IDENTITY     the cert's Common Name (extracted here)
#   APPLE_API_KEY_P8           base64 of the App Store Connect .p8
#   APPLE_API_KEY              the App Store Connect Key ID
#   APPLE_API_ISSUER           the App Store Connect Issuer ID
#
# Usage:
#   scripts/set-apple-secrets.sh \
#       --p12 GateAICert.p12 \
#       --p8 AuthKey_XXXX.p8 \
#       --issuer <ISSUER_UUID> \
#       [--key-id <KEYID>]   (default: parsed from the .p8 filename) \
#       [--repo owner/name]  (default: the repo in the current directory) \
#       [--yes]              (skip the confirmation prompt)
#
# The .p12 password is read interactively (or from APPLE_CERTIFICATE_PASSWORD
# in the environment) so it never lands in your shell history.

set -euo pipefail

usage() { sed -n '3,24p' "$0" | sed 's/^# \{0,1\}//'; }
die() { echo "error: $*" >&2; exit 1; }

P12="" P8="" ISSUER="" KEY_ID="" REPO="" ASSUME_YES=0

while [ $# -gt 0 ]; do
  case "$1" in
    --p12) P12="${2:-}"; shift 2;;
    --p8) P8="${2:-}"; shift 2;;
    --issuer) ISSUER="${2:-}"; shift 2;;
    --key-id) KEY_ID="${2:-}"; shift 2;;
    --repo) REPO="${2:-}"; shift 2;;
    --yes|-y) ASSUME_YES=1; shift;;
    -h|--help) usage; exit 0;;
    *) die "unknown argument: $1 (try --help)";;
  esac
done

command -v gh >/dev/null || die "gh CLI not found - install it, then run 'gh auth login'"
command -v openssl >/dev/null || die "openssl not found"
gh auth status >/dev/null 2>&1 || die "not authenticated - run 'gh auth login'"

[ -n "$P12" ] || die "--p12 is required"
[ -n "$P8" ] || die "--p8 is required"
[ -n "$ISSUER" ] || die "--issuer is required (App Store Connect Issuer ID)"
[ -f "$P12" ] || die "p12 not found: $P12"
[ -f "$P8" ] || die "p8 not found: $P8"

# Repo: default to whatever the current directory points at.
if [ -z "$REPO" ]; then
  REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null)" \
    || die "couldn't detect the repo - pass --repo owner/name"
fi

# Key ID: derive from the .p8 filename (AuthKey_<KEYID>.p8) unless given.
if [ -z "$KEY_ID" ]; then
  base="$(basename "$P8")"
  KEY_ID="$(printf '%s' "$base" | sed -E 's/^[Aa]uth[Kk]ey_?//; s/\.[Pp]8$//')"
  [ -n "$KEY_ID" ] || die "couldn't derive key id from '$base' - pass --key-id"
fi

# Password: from the environment, else prompt (never echoed, never in argv).
PW="${APPLE_CERTIFICATE_PASSWORD:-}"
if [ -z "$PW" ]; then
  read -r -s -p "Password for $P12: " PW; echo
fi
[ -n "$PW" ] || die "empty .p12 password"

# Signing identity = the leaf cert's Common Name. OpenSSL 3 needs -legacy to
# read Apple's .p12 (legacy RC2/3DES); LibreSSL neither needs nor supports it.
# Try plain first, then -legacy.
extract_identity() {
  openssl pkcs12 -in "$P12" -nokeys -clcerts -passin "pass:$PW" "$@" 2>/dev/null \
    | openssl x509 -noout -subject -nameopt multiline,utf8 2>/dev/null \
    | sed -n 's/ *commonName *= *//p' | head -n1
}
IDENTITY="$(extract_identity || true)"
[ -n "$IDENTITY" ] || IDENTITY="$(extract_identity -legacy || true)"
[ -n "$IDENTITY" ] || die "couldn't read the signing identity from $P12 (wrong password?)"

case "$IDENTITY" in
  "Developer ID Application:"*) ;;
  *) echo "warning: identity is '$IDENTITY' - a DMG for distribution outside" \
          "the App Store needs a 'Developer ID Application' cert" >&2;;
esac

echo
echo "Repo:     $REPO"
echo "Identity: $IDENTITY"
echo "Key ID:   $KEY_ID"
echo "Issuer:   $ISSUER"
echo "Will set: APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD, APPLE_SIGNING_IDENTITY,"
echo "          APPLE_API_KEY_P8, APPLE_API_KEY, APPLE_API_ISSUER"
echo
if [ "$ASSUME_YES" -ne 1 ]; then
  read -r -p "Push these to $REPO? [y/N] " ans
  case "$ans" in y|Y|yes|YES) ;; *) die "aborted";; esac
fi

# Pipe values to gh so they stay out of argv; printf '%s' avoids the trailing
# newline a here-string would add (which would corrupt the identity match).
set_secret() { printf '%s' "$2" | gh secret set "$1" --repo "$REPO"; }

set_secret APPLE_CERTIFICATE          "$(openssl base64 -A -in "$P12")"
set_secret APPLE_CERTIFICATE_PASSWORD "$PW"
set_secret APPLE_SIGNING_IDENTITY     "$IDENTITY"
set_secret APPLE_API_KEY_P8           "$(openssl base64 -A -in "$P8")"
set_secret APPLE_API_KEY              "$KEY_ID"
set_secret APPLE_API_ISSUER           "$ISSUER"

PW=""
echo "Done - 6 secrets set on $REPO."
