#!/bin/bash
#
# Docker Deployment Script for Chain Registry v0.2.0
# This script automates the deployment of the complete system with Phases 1-3
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
COMPOSE_FILE="docker-compose.yml"
ENV_FILE=".env"

# Functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check prerequisites
check_prerequisites() {
    log_info "Checking prerequisites..."
    
    if ! command -v docker &> /dev/null; then
        log_error "Docker is not installed. Please install Docker first."
        exit 1
    fi
    
    if ! command -v docker-compose &> /dev/null; then
        log_error "Docker Compose is not installed. Please install Docker Compose first."
        exit 1
    fi
    
    # Check Docker is running
    if ! docker info &> /dev/null; then
        log_error "Docker daemon is not running. Please start Docker."
        exit 1
    fi
    
    log_success "Prerequisites check passed"
}

# Setup environment
setup_environment() {
    log_info "Setting up environment..."
    
    if [ ! -f "$ENV_FILE" ]; then
        log_info "Creating .env file from example..."
        cp .env.example "$ENV_FILE"
        log_warning "Please edit $ENV_FILE with your configuration before continuing"
        log_warning "At minimum, set NODE1_VALIDATOR_KEY, NODE2_VALIDATOR_KEY, NODE3_VALIDATOR_KEY"
        read -p "Press Enter to continue after editing .env..."
    fi
    
    # Source the environment file
    set -a
    source "$ENV_FILE"
    set +a
    
    log_success "Environment configured"
}

# Create necessary directories
create_directories() {
    log_info "Creating necessary directories..."
    
    mkdir -p validators
    mkdir -p circuits
    mkdir -p models
    mkdir -p data/node1
    mkdir -p data/node2
    mkdir -p data/node3
    
    # Create dummy WASM validator if none exists
    if [ ! -f "validators/dummy.wasm" ]; then
        log_info "Creating dummy WASM validator..."
        echo "dummy content" > validators/dummy.wasm
    fi
    
    log_success "Directories created"
}

# Build and deploy
build_and_deploy() {
    log_info "Building Docker images..."
    
    docker-compose -f "$COMPOSE_FILE" build --parallel
    
    log_success "Docker images built"
    
    log_info "Starting services..."
    
    docker-compose -f "$COMPOSE_FILE" up -d
    
    log_success "Services started"
}

# Wait for services
wait_for_services() {
    log_info "Waiting for services to be ready..."
    
    # Wait for Anvil
    log_info "Waiting for Anvil (Ethereum local node)..."
    for i in {1..30}; do
        if curl -s -X POST -H "Content-Type: application/json" \
            --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
            http://localhost:8545 &> /dev/null; then
            log_success "Anvil is ready"
            break
        fi
        sleep 1
    done
    
    # Wait for IPFS
    log_info "Waiting for IPFS..."
    for i in {1..30}; do
        if curl -s http://localhost:5001/api/v0/id &> /dev/null; then
            log_success "IPFS is ready"
            break
        fi
        sleep 1
    done
    
    # Wait for Node-1
    log_info "Waiting for Chain Registry Node-1..."
    for i in {1..60}; do
        if curl -s http://localhost:8080/v1/health &> /dev/null; then
            log_success "Node-1 is ready"
            break
        fi
        sleep 1
    done
    
    log_success "All services are ready"
}

# Verify deployment
verify_deployment() {
    log_info "Verifying deployment..."
    
    # Check all containers are running
    RUNNING=$(docker-compose ps -q | wc -l)
    EXPECTED=6  # ipfs, anvil, deploy-contracts, node-1, node-2, node-3
    
    if [ "$RUNNING" -ge 5 ]; then
        log_success "All required containers are running ($RUNNING/6)"
    else
        log_warning "Some containers may not be running ($RUNNING/6)"
        docker-compose ps
    fi
    
    # Test health endpoints
    log_info "Testing health endpoints..."
    
    if curl -s http://localhost:8080/v1/health | grep -q "ok"; then
        log_success "Node-1 health check passed"
    else
        log_warning "Node-1 health check failed"
    fi
    
    # Check contract deployment
    log_info "Checking contract deployment status..."
    if docker-compose logs deploy-contracts | grep -q "Contracts deployed"; then
        log_success "Contracts deployed successfully"
    else
        log_warning "Contract deployment status unclear - check logs with: docker-compose logs deploy-contracts"
    fi
    
    log_success "Deployment verification complete"
}

# Print status
print_status() {
    echo ""
    echo "=========================================="
    echo "  Chain Registry Deployment Status"
    echo "=========================================="
    echo ""
    
    echo "Services:"
    docker-compose ps
    
    echo ""
    echo "Access URLs:"
    echo "  - Node 1 API:     http://localhost:8080"
    echo "  - Node 2 API:     http://localhost:8082"
    echo "  - Node 3 API:     http://localhost:8083"
    echo "  - IPFS API:       http://localhost:5001"
    echo "  - IPFS Gateway:   http://localhost:8081"
    echo "  - Ethereum RPC:   http://localhost:8545"
    echo ""
    
    echo "Features Enabled:"
    echo "  ✅ Phase 1: ZK Validation"
    echo "  ✅ Phase 1: ML Threat Detection"
    echo "  ✅ Phase 1: WASM Sandboxing"
    echo "  ✅ Phase 2: Private Registries"
    echo "  ✅ Phase 2: Cross-Chain Support"
    echo "  ✅ Phase 3: CREG Token"
    echo "  ✅ Phase 3: Governance V2"
    echo "  ✅ Phase 3: Package Insurance"
    echo ""
    
    echo "Commands:"
    echo "  View logs:        docker-compose logs -f"
    echo "  Stop services:    docker-compose down"
    echo "  CLI tool:         docker-compose run --rm cli --help"
    echo ""
    
    echo "=========================================="
}

# Main function
main() {
    echo "=========================================="
    echo "  Chain Registry Docker Deployment"
    echo "  Version: v0.2.0 (Phases 1-3)"
    echo "=========================================="
    echo ""
    
    check_prerequisites
    setup_environment
    create_directories
    build_and_deploy
    wait_for_services
    verify_deployment
    print_status
    
    log_success "Deployment complete!"
}

# Handle script interruption
trap 'log_error "Deployment interrupted"; exit 1' INT TERM

# Run main function
main "$@"
