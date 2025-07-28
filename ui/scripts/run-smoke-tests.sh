#!/bin/bash

# Smoke Test Runner with Backend Check
# This script ensures the backend is running before executing smoke tests

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Check if backend is running
check_backend() {
    print_status "Checking if AgentGateway backend is running..."
    
    if curl -s -f "http://localhost:15021/healthz/ready" > /dev/null 2>&1; then
        print_success "Backend is already running and healthy"
        return 0
    else
        print_warning "Backend is not running on port 15021"
        return 1
    fi
}

# Start backend if needed
start_backend_if_needed() {
    if ! check_backend; then
        print_status "Starting AgentGateway backend..."
        
        # Find the binary (look in parent directory since we're in ui/)
        local binary=""
        if [[ -x "../target/release/agentgateway" ]]; then
            binary="../target/release/agentgateway"
        elif [[ -x "../target/debug/agentgateway" ]]; then
            binary="../target/debug/agentgateway"
        else
            print_error "AgentGateway binary not found. Please build it first with: cargo build --release"
            return 1
        fi
        
        print_status "Using binary: $binary"
        
        # Start in background
        cd ..
        "$binary" --file test-config.yaml > /dev/null 2>&1 &
        local backend_pid=$!
        cd ui
        
        print_status "Backend started (PID: $backend_pid)"
        
        # Wait for it to be ready
        local count=0
        while [[ $count -lt 30 ]]; do
            if curl -s -f "http://localhost:15021/healthz/ready" > /dev/null 2>&1; then
                print_success "Backend is ready!"
                return 0
            fi
            sleep 1
            ((count++))
        done
        
        print_error "Backend failed to start within 30 seconds"
        return 1
    fi
}

# Main execution
main() {
    print_status "AgentGateway Smoke Test Runner"
    
    # Ensure we're in the ui directory
    if [[ ! -f "package.json" ]]; then
        print_error "This script must be run from the ui directory"
        exit 1
    fi
    
    # Check/start backend
    if ! start_backend_if_needed; then
        print_error "Failed to ensure backend is running"
        exit 1
    fi
    
    # Run smoke tests
    print_status "Running smoke tests..."
    npm run e2e:smoke
    
    local exit_code=$?
    
    if [[ $exit_code -eq 0 ]]; then
        print_success "Smoke tests completed successfully!"
    else
        print_error "Smoke tests failed"
    fi
    
    exit $exit_code
}

main "$@"
