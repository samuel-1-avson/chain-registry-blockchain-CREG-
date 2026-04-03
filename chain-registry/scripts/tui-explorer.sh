#!/bin/bash
# Chain Registry TUI Explorer Launcher
# Usage: ./scripts/tui-explorer.sh [dev|testnet|light]

set -e

MODE="${1:-dev}"
COMPOSE_FILE="docker-compose.yml"

# Determine which compose file to use
case "$MODE" in
  dev|single)
    COMPOSE_FILE="docker-compose.yml"
    ;;
  testnet)
    COMPOSE_FILE="docker-compose.testnet.yml"
    ;;
  light)
    COMPOSE_FILE="docker-compose.light.yml"
    ;;
  *)
    echo "Usage: $0 [dev|testnet|light]"
    echo ""
    echo "Modes:"
    echo "  dev      - Single validator development setup (default)"
    echo "  testnet  - 10-validator testnet"
    echo "  light    - Resource-constrained light node"
    exit 1
    ;;
esac

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
if ! docker compose -f "$COMPOSE_FILE" ps | grep -q "creg-node"; then
    echo "⚠️  Node is not running. Starting it first..."
    docker compose -f "$COMPOSE_FILE" up -d
    
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

# Run with TTY allocation
docker compose -f "$COMPOSE_FILE" run --rm tui-explorer

echo ""
echo "👋 TUI Explorer closed"
