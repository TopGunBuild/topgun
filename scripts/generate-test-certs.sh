#!/bin/bash
# scripts/generate-test-certs.sh
set -e

mkdir -p packages/server/test/fixtures
cd packages/server/test/fixtures

# Helper to avoid interactive prompts
SUBJECT="/C=US/ST=State/L=City/O=TopGun/OU=Test/CN=localhost"

echo "Generating CA..."
openssl genrsa -out ca.key 4096
openssl req -new -x509 -days 365 -key ca.key \
  -out ca.crt -subj "/CN=TopGun Test CA"

echo "Generating Server Cert..."
openssl genrsa -out server.key 2048
openssl req -new -key server.key \
  -out server.csr -subj "/CN=localhost"
openssl x509 -req -days 365 -in server.csr \
  -CA ca.crt -CAkey ca.key \
  -CAcreateserial -out server.crt \
  -extfile <(echo "subjectAltName=DNS:localhost,IP:127.0.0.1")

echo "Generating Cluster Node Certs..."
for i in 1 2 3; do
  echo "Node $i..."
  openssl genrsa -out node${i}.key 2048
  openssl req -new -key node${i}.key \
    -out node${i}.csr -subj "/CN=node-${i}"
  openssl x509 -req -days 365 -in node${i}.csr \
    -CA ca.crt -CAkey ca.key \
    -CAcreateserial -out node${i}.crt \
    -extfile <(echo "subjectAltName=DNS:localhost,IP:127.0.0.1")
done

# Cleanup CSRs
rm *.csr
rm *.srl

echo "Done! Certificates generated in packages/server/test/fixtures/"
