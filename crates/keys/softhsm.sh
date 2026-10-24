#!/bin/sh
# The smart-card tests against a real PKCS #11 module: SoftHSM 2 with a token holding an RSA and
# an EC (P-256) certificate and their keys, from a throwaway CA. Run in Docker from the repo root:
#   docker run --rm -v "$PWD:/src:ro" rust:1-bookworm sh /src/crates/keys/softhsm.sh
# Signatures made on the card are checked by OpenSSL as well.
set -eu
apt-get update -qq && apt-get install -y -qq softhsm2 opensc openssl >/dev/null
M=/usr/lib/softhsm/libsofthsm2.so
W=$(mktemp -d) && cd "$W"
mkdir tokens && echo "directories.tokendir = $W/tokens" > softhsm2.conf
export SOFTHSM2_CONF="$W/softhsm2.conf"
softhsm2-util --init-token --free --label mailcraft --pin 123456 --so-pin 12345678 >/dev/null

openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem -subj "/CN=Card Test CA" -days 3650 2>/dev/null
cat > user.ext <<EOF
basicConstraints = CA:FALSE
keyUsage = critical, digitalSignature, keyEncipherment, keyAgreement
extendedKeyUsage = emailProtection
subjectAltName = email:card@posteo.example
EOF
i=1
for kind in rsa ec; do
  if [ $kind = rsa ]; then openssl genrsa -out $kind.key 2048 2>/dev/null; else openssl ecparam -name prime256v1 -genkey -noout -out $kind.key; fi
  openssl req -new -key $kind.key -subj "/CN=card $kind/emailAddress=card@posteo.example" -out $kind.csr
  openssl x509 -req -in $kind.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 825 -extfile user.ext -out $kind.pem 2>/dev/null
  openssl pkcs8 -topk8 -nocrypt -in $kind.key -outform DER -out $kind.p8
  openssl x509 -in $kind.pem -outform DER -out $kind.der
  if [ $kind = rsa ]; then usage="--usage-sign --usage-decrypt"; else usage="--usage-sign --usage-derive"; fi
  pkcs11-tool --module $M --login --pin 123456 --write-object $kind.p8 --type privkey --id 0$i --label $kind $usage >/dev/null
  pkcs11-tool --module $M --login --pin 123456 --write-object $kind.der --type cert --id 0$i --label $kind >/dev/null
  i=$((i + 1))
done
pkcs11-tool --module $M --login --pin 123456 --list-objects | grep -E "Certificate|Private Key"

mkdir out
mkdir "$W/src" && tar -C /src --exclude=./target --exclude=./.git -cf - . | tar -C "$W/src" -xf - && cd "$W/src"
MAILCRAFT_PKCS11_MODULE=$M MAILCRAFT_PKCS11_CA="$W/ca.pem" MAILCRAFT_PKCS11_OUT="$W/out" CARGO_TARGET_DIR="$W/target" \
  cargo test -q -p mailcraft-keys -- --include-ignored

printf 'Content-Type: text/plain\r\n\r\nSigned on the card.\r\n' > "$W/content"
for kind in rsa ec; do
  openssl cms -verify -binary -inform DER -in "$W/out/$kind.sig.der" -content "$W/content" -CAfile "$W/ca.pem" -purpose any -out /dev/null
  echo "OpenSSL: the $kind signature made on the card holds"
done
