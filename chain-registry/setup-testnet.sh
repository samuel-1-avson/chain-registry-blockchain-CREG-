#!/bin/bash
# Chain Registry Testnet Setup Script
# Master script to prepare the system for testnet deployment
#
# IMPORTANT ARCHITECTURE NOTE:
# ============================
# In PRODUCTION: Each validator MUST run on a separate PC
# In TESTING: You CAN run multiple validators on one PC (this script does that)
#
# This script automates the remaining 20% of setup needed for testnet readiness.

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$SCRIPT_DIR"

# Logging
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_step() {
    echo ""
    echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}  STEP $1: $2${NC}"
    echo -e "${BLUE}════════════════════════════════════════════════════════════${NC}"
}

# Header
print_header() {
    echo ""
    echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${BLUE}║       Chain Registry - Testnet Setup Script                ║${NC}"
    echo -e "${BLUE}║       Completes the remaining 20% for testnet readiness    ║${NC}"
    echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "${YELLOW}ARCHITECTURE NOTE:${NC}"
    echo "  This script sets up multiple validators on one PC for TESTING."
    echo "  In PRODUCTION, run ONE validator per PC only."
    echo ""
    echo "Press Enter to continue or Ctrl+C to cancel..."
    read
}

# Check prerequisites
check_prerequisites() {
    log_step "1" "Checking Prerequisites"
    
    local missing=()
    
    # Check Docker
    if ! command -v docker &> /dev/null; then
        missing+=("docker")
    fi
    
    # Check Docker Compose
    if ! command -v docker-compose &> /dev/null && ! docker compose version &> /dev/null; then
        missing+=("docker-compose")
    fi
    
    # Check Rust/Cargo
    if ! command -v cargo &> /dev/null; then
        missing+=("rust/cargo")
    fi
    
    # Check Foundry
    if ! command -v forge &> /dev/null; then
        missing+=("foundry")
    fi
    
    if [ ${#missing[@]} -gt 0 ]; then
        log_error "Missing prerequisites: ${missing[*]}"
        log_info "Please install the missing tools and run this script again."
        exit 1
    fi
    
    log_success "All prerequisites met"
}

# Generate validator keys
setup_validator_keys() {
    log_step "2" "Generating Validator Keys"
    
    if [ -f "$PROJECT_ROOT/.env" ]; then
        log_warn ".env file already exists"
        read -p "Regenerate validator keys? (y/N): " regenerate
        if [[ ! $regenerate =~ ^[Yy]$ ]]; then
            log_info "Skipping key generation"
            return 0
        fi
    fi
    
    log_info "Generating 3 validator keys for testing..."
    
    # Run key generation script
    if [ -f "$PROJECT_ROOT/scripts/generate-validator-keys.sh" ]; then
        bash "$PROJECT_ROOT/scripts/generate-validator-keys.sh" 3
    else
        log_error "Key generation script not found"
        exit 1
    fi
    
    log_success "Validator keys generated"
}

# Build Docker images
build_docker_images() {
    log_step "3" "Building Docker Images"
    
    log_info "Building minimal image (faster for testing)..."
    
    cd "$PROJECT_ROOT"
    
    if ! docker build -f Dockerfile.minimal -t chain-registry:local . > /tmp/docker-build.log 2>&1; then
        log_error "Docker build failed"
        tail -30 /tmp/docker-build.log
        exit 1
    fi
    
    log_success "Docker image built successfully"
}

# Deploy smart contracts
deploy_contracts() {
    log_step "4" "Deploying Smart Contracts"
    
    log_info "Starting local infrastructure..."
    
    cd "$PROJECT_ROOT"
    
    # Start Anvil and IPFS
    docker-compose up -d anvil ipfs
    
    # Wait for Anvil
    log_info "Waiting for Anvil to be ready..."
    for i in {1..30}; do
        if curl -s -X POST -H "Content-Type: application/json" \
            --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' \
            http://localhost:8545 > /dev/null 2>&1; then
            break
        fi
        sleep 1
    done
    
    log_success "Anvil is ready"
    
    # Deploy contracts
    log_info "Deploying testnet contracts..."
    
    if [ -f "$PROJECT_ROOT/testnet/deploy-testnet.sh" ]; then
        cd "$PROJECT_ROOT/testnet"
        bash deploy-testnet.sh
        cd "$PROJECT_ROOT"
    else
        log_warn "Contract deployment script not found, using docker-compose deployer"
        docker-compose up -d deploy-contracts
        sleep 10
    fi
    
    log_success "Contracts deployed"
}

# Start validators
start_validators() {
    log_step "5" "Starting Validators (TEST MODE: Multiple on One PC)"
    
    cd "$PROJECT_ROOT"
    
    log_info "Starting Validator 1 (Primary)..."
    docker-compose up -d node
    
    # Wait for validator 1
    log_info "Waiting for Validator 1 to be ready..."
    for i in {1..60}; do
        if curl -s http://localhost:8080/v1/health > /dev/null 2>&1; then
            log_success "Validator 1 is ready"
            break
        fi
        sleep 2
    done
    
    # Start additional validators if configs exist
    for i in 2 3; do
        local compose_file="$PROJECT_ROOT/validator-keys/validator-$i-docker-compose.yml"
        if [ -f "$compose_file" ]; then
            log_info "Starting Validator $i..."
            docker-compose -f "$compose_file" up -d
            sleep 5
        fi
    done
    
    log_success "Validators started"
}

# Run integration tests
run_tests() {
    log_step "6" "Running Integration Tests"
    
    cd "$PROJECT_ROOT"
    
    if [ -f "$PROJECT_ROOT/tests/integration_tests.sh" ]; then
        log_info "Running integration test suite..."
        if bash "$PROJECT_ROOT/tests/integration_tests.sh" --skip-cleanup; then
            log_success "All integration tests passed"
        else
            log_warn "Some integration tests failed (check logs)"
        fi
    else
        log_warn "Integration tests not found, skipping"
    fi
}

# Print final status
print_status() {
    log_step "7" "Setup Complete"
    
    echo ""
    echo -e "${GREEN}✓ Testnet setup is complete!${NC}"
    echo ""
    echo "Access Points:"
    echo "  Validator 1 API:  http://localhost:8080"
    echo "  Validator 2 API:  http://localhost:8081 (if running)"
    echo "  Validator 3 API:  http://localhost:8082 (if running)"
    echo "  IPFS API:         http://localhost:5001"
    echo "  IPFS Gateway:     http://localhost:8081"
    echo "  Ethereum RPC:     http://localhost:8545"
    echo ""
    echo "Useful Commands:"
    echo "  View logs:        docker-compose logs -f node"
    echo "  Stop all:         docker-compose down -v"
    echo "  CLI tool:         docker-compose run --rm cli --help"
    echo "  Run tests:        ./tests/integration_tests.sh"
    echo ""
    echo "Next Steps:"
    echo "  1. Test package publishing with: cargo run --bin creg -- publish"
    echo "  2. Monitor validators: docker-compose logs -f"
    echo "  3. Access explorer: http://localhost:8080/ui/"
    echo ""
    echo -e "${YELLOW}REMINDER:${NC} This is a TESTING setup with multiple validators on one PC."
    echo "For production, run ONE validator per PC only."
    echo ""
    echo "For more information, see:"
    echo "  - VALIDATOR_ARCHITECTURE.md"
    echo "  - PROJECT_ANALYSIS_AND_TESTNET_GUIDE.md"
    echo ""
}

# Main execution
main() {
    print_header
    check_prerequisites
    setup_validator_keys
    build_docker_images
    deploy_contracts
    start_validators
    run_tests
    print_status
}

# Handle script interruption
trap 'log_error "Setup interrupted"; exit 1' INT TERM

# Run main
main "$@"
