#!/bin/bash
# Chain Registry TUI Explorer Launcher
# Usage: ./scripts/tui-explorer.sh [dev|testnet|light]

set -e

MODE="${1:-dev}"
COMPOSE_FILE="docker-compose.yml"
NODE_SERVICE="node"
NODE_CONTAINER="creg-node"
COMPOSE_ARGS=()

# Determine which compose file and node container to use
case "$MODE" in
  dev|single)
    COMPOSE_FILE="docker-compose.yml"
    NODE_SERVICE="node"
    NODE_CONTAINER="creg-node"
    ;;
  testnet)
    COMPOSE_FILE="docker-compose.testnet.yml"
    NODE_SERVICE="node-1"
    NODE_CONTAINER="creg-testnet-node-1"
    COMPOSE_ARGS+=(--env-file .env.testnet)
    ;;
  light)
    COMPOSE_FILE="docker-compose.light.yml"
    NODE_SERVICE="node-light"
    NODE_CONTAINER="creg-node-light"
    ;;
  *)
    echo "Usage: $0 [dev|testnet|light]"
    echo ""
    echo "Modes:"
    echo "  dev      - Single validator development setup (default)"
    echo "  testnet  - Single-validator testnet bootstrap host"
    echo "  light    - Resource-constrained light node"
    exit 1
    ;;
esac

compose() {
  docker compose "${COMPOSE_ARGS[@]}" -f "$COMPOSE_FILE" "$@"
}

echo "🚀 Launching Chain Registry TUI Explorer..."
echo "   Mode: $MODE"
echo "   Compose file: $COMPOSE_FILE"
echo ""

# Check if docker compose is available
if ! command -v docker &> /dev/null; then
    echo "❌ Docker is not installed"
    exit 1
fi

# Ensure the node is running
echo "🔍 Checking if node is running..."
if ! docker ps --filter "name=^/${NODE_CONTAINER}$" --format '{{.Names}}' | grep -q .; then
    echo "⚠️  Node is not running. Starting it first..."
  if [ "$MODE" = "testnet" ]; then
    compose up -d ipfs anvil postgres
    compose up -d --no-deps node-1 faucet web-explorer
  else
    compose up -d "$NODE_SERVICE"
  fi
    
    # Wait for node to be healthy
    echo "⏳ Waiting for node to be ready..."
    sleep 5
    
    for i in {1..30}; do
        if curl -s http://localhost:8080/v1/health > /dev/null 2>&1; then
            echo "✅ Node is ready!"
            break
        fi
        echo "   Still waiting... ($i/30)"
        sleep 2
    done
fi

# Launch TUI explorer
echo ""
echo "🖥️  Starting TUI Explorer..."
echo "   Press '?' for help, 'q' to quit"
echo ""

if ! docker exec -it "$NODE_CONTAINER" /app/creg console --node-url http://127.0.0.1:8080; then
  echo "⚠️  Direct console attach failed, falling back to one-shot TUI container..."
  compose run --rm --no-deps tui-explorer
fi

echo ""
echo "👋 TUI Explorer closed"
