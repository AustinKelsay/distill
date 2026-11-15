#!/usr/bin/env bash
# One time, on macOS: create a local code-signing identity for Distill in the login keychain.
#
# Privacy grants such as Full Disk Access are tied to a binary's code signature. `cargo install`
# produces an ad-hoc signature that changes on every build, so each reinstall resets the grant.
# Signed with a stable local certificate (see scripts/install.sh), the grant survives reinstalls.
#
# The private key is created in a temporary folder, imported into the keychain, and deleted;
# it never touches the repo. macOS asks once for your password to trust the certificate for
# code signing.
set -euo pipefail

NAME="Distill Local Code Signing"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

if security find-identity -v -p codesigning | grep -q "\"$NAME\""; then
  echo "\"$NAME\" already exists and is trusted for code signing."
  exit 0
fi

# LibreSSL ships with macOS and writes a PKCS#12 file `security import` can read.
OPENSSL=/usr/bin/openssl
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

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

"$OPENSSL" req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -config "$tmp/cert.cnf" -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null
pass=$("$OPENSSL" rand -hex 16)
"$OPENSSL" pkcs12 -export -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
  -name "$NAME" -passout "pass:$pass" -out "$tmp/identity.p12"

security import "$tmp/identity.p12" -k "$KEYCHAIN" -P "$pass" -T /usr/bin/codesign
echo "Trusting the certificate for code signing (macOS asks for your password)..."
security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$tmp/cert.pem"

security find-identity -v -p codesigning | grep "\"$NAME\""
echo "Done. Now run scripts/install.sh."
