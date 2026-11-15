#!/usr/bin/env bash
# One time, on macOS: create a local code-signing identity for Distill in the login keychain.
#
# Privacy grants such as Full Disk Access are tied to a binary's code signature. `cargo install`
# produces an ad-hoc signature that changes on every build, so each reinstall resets the grant.
# Signed with a stable local certificate (see scripts/install.sh), the grant survives reinstalls,
# as long as this same certificate is kept: a new one means granting access once more.
#
# The private key is only ever written to disk encrypted, under a passphrase that exists only in
# this process, and is deleted once it is in the keychain. macOS asks once for your password to
# trust the certificate for code signing.
set -euo pipefail

NAME="Distill Local Code Signing"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
# LibreSSL ships with macOS and writes a PKCS#12 file `security import` can read.
OPENSSL=/usr/bin/openssl

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

trust() {
  echo "Trusting \"$NAME\" for code signing (macOS asks for your password)..."
  security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$1"
}

if security find-identity -v -p codesigning | grep -q "\"$NAME\""; then
  echo "\"$NAME\" already exists and is trusted for code signing."
  exit 0
fi

# An earlier run may have imported the certificate and then been stopped before it was trusted:
# trust that one rather than making a second identity with the same name.
existing=$(security find-certificate -a -c "$NAME" -Z "$KEYCHAIN" 2>/dev/null | grep -c '^SHA-1' || true)
if [[ "$existing" -gt 1 ]]; then
  echo "There are $existing certificates named \"$NAME\". Delete all but one in Keychain Access, then run this again." >&2
  exit 1
fi
if [[ "$existing" -eq 1 ]]; then
  security find-certificate -c "$NAME" -p "$KEYCHAIN" > "$tmp/cert.pem"
  trust "$tmp/cert.pem"
else
  cat > "$tmp/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
CNF
  pass=$("$OPENSSL" rand -hex 16)
  "$OPENSSL" req -x509 -newkey rsa:2048 -days 3650 -config "$tmp/cert.cnf" \
    -passout "pass:$pass" -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null
  "$OPENSSL" pkcs12 -export -inkey "$tmp/key.pem" -passin "pass:$pass" -in "$tmp/cert.pem" \
    -name "$NAME" -passout "pass:$pass" -out "$tmp/identity.p12"
  rm -f "$tmp/key.pem"
  security import "$tmp/identity.p12" -k "$KEYCHAIN" -P "$pass" -T /usr/bin/codesign
  trust "$tmp/cert.pem"
fi

security find-identity -v -p codesigning | grep "\"$NAME\""
echo "Done. Now run scripts/install.sh."
