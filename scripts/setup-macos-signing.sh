#!/bin/sh
# Explicit, one-time setup. Never called automatically by a build.
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
    echo "Local signing requires macOS." >&2
    exit 1
fi
identity='Taypeer Local Development'
keychain=$(/usr/bin/security default-keychain -d user | sed 's/^[[:space:]]*"//; s/"[[:space:]]*$//')
if [ ! -f "$keychain" ]; then
    echo "A user default keychain is required." >&2
    exit 1
fi
if /usr/bin/security find-certificate -c "$identity" "$keychain" >/dev/null 2>&1; then
    echo "Certificate already exists; retaining it. Check with security find-identity -v -p codesigning."
    exit 0
fi
umask 077
signing_tmp=$(mktemp -d /private/tmp/taypeer-signing.XXXXXX)
cleanup() {
    rm -f "$signing_tmp/key.pem" "$signing_tmp/cert.pem"
    rmdir "$signing_tmp"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
/usr/bin/openssl req -new -x509 -newkey rsa:3072 -nodes -sha256 -days 3650 \
    -config scripts/macos-signing.cnf \
    -keyout "$signing_tmp/key.pem" -out "$signing_tmp/cert.pem"
# No -A: other applications are not granted unrestricted use of this private key.
/usr/bin/security import "$signing_tmp/key.pem" -k "$keychain" -t priv \
    -x -T /usr/bin/codesign
/usr/bin/security add-trusted-cert -r trustRoot -p codeSign -k "$keychain" \
    "$signing_tmp/cert.pem"
/usr/bin/security find-identity -v -p codesigning "$keychain"
echo "Local signing identity installed. Its private key remains only in the user keychain."
