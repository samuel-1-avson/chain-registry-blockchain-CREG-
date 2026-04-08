#!/bin/sh
set -e

CORS_ORIGIN="${IPFS_CORS_ORIGIN:-http://localhost:3000,http://localhost:8080,http://creg-testnet.local}"

echo "Setting up IPFS CORS..."
echo "  Allowed origins: $CORS_ORIGIN"
ipfs config --json API.HTTPHeaders.Access-Control-Allow-Origin "[\"$(echo "$CORS_ORIGIN" | sed 's/,/","/g')\"]"
ipfs config --json API.HTTPHeaders.Access-Control-Allow-Methods '["PUT", "POST", "GET"]'
echo "CORS set!"