#!/bin/bash
# Generate self-signed TLS certificates for testnet
# These are for TESTING ONLY — use proper CA-signed certs for production
set -e

CERT_DIR="${1:-./testnet/certs}"
DAYS=365
CN="creg-testnet.local"

mkdir -p "$CERT_DIR"

echo "Generating self-signed TLS certificate for testnet..."
echo "  Output: $CERT_DIR"
echo "  CN: $CN"
echo "  Valid: $DAYS days"

# Generate CA key and cert
openssl genrsa -out "$CERT_DIR/ca.key" 4096
openssl req -new -x509 -days "$DAYS" -key "$CERT_DIR/ca.key" \
  -out "$CERT_DIR/ca.crt" \
  -subj "/C=US/ST=Test/L=Test/O=ChainRegistry/CN=CREG Testnet CA"

# Generate server key and CSR
openssl genrsa -out "$CERT_DIR/server.key" 2048
openssl req -new -key "$CERT_DIR/server.key" \
  -out "$CERT_DIR/server.csr" \
  -subj "/C=US/ST=Test/L=Test/O=ChainRegistry/CN=$CN"

# Create SAN extension file for the single-validator Docker host and the
# validator-host compose variant.
cat > "$CERT_DIR/san.ext" <<EOF
authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage = digitalSignature, nonRepudiation, keyEncipherment, dataEncipherment
subjectAltName = @alt_names

[alt_names]
DNS.1 = $CN
DNS.2 = localhost
DNS.3 = *.creg-testnet.local
DNS.4 = creg-testnet-node-1
DNS.5 = creg-validator
DNS.6 = creg-node
IP.1 = 127.0.0.1
IP.2 = 0.0.0.0
EOF

# Sign the server cert with CA
openssl x509 -req -in "$CERT_DIR/server.csr" \
  -CA "$CERT_DIR/ca.crt" -CAkey "$CERT_DIR/ca.key" -CAcreateserial \
  -out "$CERT_DIR/server.crt" -days "$DAYS" \
  -extfile "$CERT_DIR/san.ext"

# Clean up intermediate files
rm -f "$CERT_DIR/server.csr" "$CERT_DIR/san.ext" "$CERT_DIR/ca.srl"

# Set restrictive permissions
chmod 600 "$CERT_DIR/server.key" "$CERT_DIR/ca.key"
chmod 644 "$CERT_DIR/server.crt" "$CERT_DIR/ca.crt"

echo ""
echo "TLS certificates generated successfully:"
echo "  CA cert:     $CERT_DIR/ca.crt"
echo "  Server cert: $CERT_DIR/server.crt"
echo "  Server key:  $CERT_DIR/server.key"
echo ""
echo "Add to docker-compose environment:"
echo "  CREG_TLS_CERT: /app/certs/server.crt"
echo "  CREG_TLS_KEY:  /app/certs/server.key"
