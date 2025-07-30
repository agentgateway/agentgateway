#!/bin/bash

# AgentGateway E2E Test Runner
# This script automatically sets up the environment and runs E2E tests

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BACKEND_PORT=8080
UI_PORT=3000
BACKEND_URL="http://localhost:${BACKEND_PORT}"
UI_URL="http://localhost:${UI_PORT}/ui"
TIMEOUT=60
PARALLEL_WORKERS=4

# Default values
MODE="parallel"
HEADLESS=true
CLEANUP=true
VERBOSE=false
AUTO_DETECT=true
MEMORY_LIMIT=""
SETUP_CONFIG_FILE=".e2e-setup-config"

# Function to print colored output
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

# Function to show usage
show_usage() {
    cat << EOF
Usage: $0 [OPTIONS]

AgentGateway E2E Test Runner - Automatically sets up environment and runs tests

This enhanced version includes:
  - Automatic resource detection and optimization
  - Intelligent worker count selection
  - Enhanced error messages with solutions
  - Setup validation and health checks

OPTIONS:
    -m, --mode MODE         Test execution mode: parallel, sequential, interactive (default: parallel)
    -w, --workers NUM       Number of parallel workers (default: auto-detect)
    -t, --timeout SEC       Timeout for service startup (default: 60)
    --memory-limit PERCENT  Memory usage limit percentage (default: auto-detect)
    --no-auto-detect       Disable automatic resource detection
    --no-cleanup           Don't cleanup processes after tests
    --headed               Run tests in headed mode (visible browser)
    --verbose              Enable verbose logging
    -h, --help             Show this help message

EXAMPLES:
    $0                                    # Run tests with auto-detected optimal settings
    $0 --mode sequential                  # Run tests sequentially
    $0 --mode interactive                 # Open Cypress test runner
    $0 --workers 8 --verbose            # Run with 8 workers and verbose logging
    $0 --headed --no-cleanup             # Run with visible browser, don't cleanup
    $0 --no-auto-detect --workers 2     # Disable auto-detection, use 2 workers

FIRST-TIME SETUP:
    If this is your first time running E2E tests, consider using:
    ./scripts/setup-first-time.sh       # One-command setup for new developers

ENVIRONMENT VARIABLES:
    AGENTGATEWAY_BINARY    Path to agentgateway binary (default: auto-detect)
    SKIP_BUILD            Skip building agentgateway (default: false)
    SKIP_BACKEND          Skip starting backend (assume already running)
    SKIP_UI               Skip starting UI (assume already running)
    CI                    CI mode - enables additional logging (default: false)

EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -m|--mode)
            MODE="$2"
            shift 2
            ;;
        -w|--workers)
            PARALLEL_WORKERS="$2"
            shift 2
            ;;
        -t|--timeout)
            TIMEOUT="$2"
            shift 2
            ;;
        --memory-limit)
            MEMORY_LIMIT="$2"
            shift 2
            ;;
        --no-auto-detect)
            AUTO_DETECT=false
            shift
            ;;
        --no-cleanup)
            CLEANUP=false
            shift
            ;;
        --headed)
            HEADLESS=false
            shift
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        -h|--help)
            show_usage
            exit 0
            ;;
        *)
            print_error "Unknown option: $1"
            show_usage
            exit 1
            ;;
    esac
done

# Validate mode
if [[ ! "$MODE" =~ ^(parallel|sequential|interactive)$ ]]; then
    print_error "Invalid mode: $MODE. Must be one of: parallel, sequential, interactive"
    exit 1
fi

# Global variables for process tracking
BACKEND_PID=""
UI_PID=""

# Cleanup function
cleanup() {
    if [[ "$CLEANUP" == "true" ]]; then
        print_status "Cleaning up processes..."
        
        if [[ -n "$BACKEND_PID" ]]; then
            print_status "Stopping AgentGateway backend (PID: $BACKEND_PID)"
            kill $BACKEND_PID 2>/dev/null || true
            wait $BACKEND_PID 2>/dev/null || true
        fi
        
        if [[ -n "$UI_PID" ]]; then
            print_status "Stopping UI development server (PID: $UI_PID)"
            kill $UI_PID 2>/dev/null || true
            wait $UI_PID 2>/dev/null || true
        fi
        
        # Kill any remaining processes on our ports
        lsof -ti:$BACKEND_PORT | xargs kill -9 2>/dev/null || true
        lsof -ti:$UI_PORT | xargs kill -9 2>/dev/null || true
        
        print_success "Cleanup completed"
    else
        print_warning "Skipping cleanup - processes left running"
        if [[ -n "$BACKEND_PID" ]]; then
            print_warning "AgentGateway backend PID: $BACKEND_PID"
        fi
        if [[ -n "$UI_PID" ]]; then
            print_warning "UI development server PID: $UI_PID"
        fi
    fi
}

# Set up signal handlers
trap cleanup EXIT
trap 'print_error "Script interrupted"; exit 1' INT TERM

# Function to wait for service to be ready
wait_for_service() {
    local url=$1
    local service_name=$2
    local timeout=$3
    
    print_status "Waiting for $service_name to be ready at $url..."
    
    local count=0
    while [[ $count -lt $timeout ]]; do
        if curl -s -f "$url" > /dev/null 2>&1; then
            print_success "$service_name is ready!"
            return 0
        fi
        
        if [[ $((count % 10)) -eq 0 ]] && [[ $count -gt 0 ]]; then
            print_status "Still waiting for $service_name... (${count}s elapsed)"
        fi
        
        sleep 1
        ((count++))
    done
    
    print_error "$service_name failed to start within ${timeout}s"
    return 1
}

# Function to find agentgateway binary
find_agentgateway_binary() {
    if [[ -n "$AGENTGATEWAY_BINARY" ]]; then
        if [[ -x "$AGENTGATEWAY_BINARY" ]]; then
            echo "$AGENTGATEWAY_BINARY"
            return 0
        else
            print_error "AGENTGATEWAY_BINARY is set but not executable: $AGENTGATEWAY_BINARY"
            return 1
        fi
    fi
    
    # Try common locations
    local candidates=(
        "./target/release/agentgateway"
        "./target/debug/agentgateway"
        "./target/release/agentgateway-app"
        "./target/debug/agentgateway-app"
        "$(which agentgateway 2>/dev/null)"
        "$(which agentgateway-app 2>/dev/null)"
    )
    
    for candidate in "${candidates[@]}"; do
        if [[ -x "$candidate" ]]; then
            echo "$candidate"
            return 0
        fi
    done
    
    return 1
}

# Function to build agentgateway if needed
build_agentgateway() {
    if [[ "$SKIP_BUILD" == "true" ]]; then
        print_status "Skipping build (SKIP_BUILD=true)"
        return 0
    fi
    
    print_status "Building AgentGateway..."
    
    # Always show build output for debugging
    if [[ "$VERBOSE" == "true" ]] || [[ "$CI" == "true" ]]; then
        cargo build --release --bin agentgateway
        local build_result=$?
    else
        # Capture output for error reporting
        local build_output
        build_output=$(cargo build --release --bin agentgateway 2>&1)
        local build_result=$?
        
        if [[ $build_result -ne 0 ]]; then
            print_error "AgentGateway build failed with output:"
            echo "$build_output"
        fi
    fi
    
    if [[ $build_result -eq 0 ]]; then
        print_success "AgentGateway build completed"
    else
        print_error "AgentGateway build failed"
        return 1
    fi
}

# Function to start AgentGateway backend
start_backend() {
    print_status "Starting AgentGateway backend..."
    
    # Check if port is already in use
    if lsof -Pi :$BACKEND_PORT -sTCP:LISTEN -t >/dev/null; then
        print_warning "Port $BACKEND_PORT is already in use"
        if curl -s -f "$BACKEND_URL/health" > /dev/null 2>&1; then
            print_success "AgentGateway is already running and healthy"
            return 0
        else
            print_error "Port $BACKEND_PORT is occupied by another service"
            return 1
        fi
    fi
    
    # Find or build the binary
    local binary
    if ! binary=$(find_agentgateway_binary); then
        print_status "AgentGateway binary not found, building..."
        build_agentgateway
        if ! binary=$(find_agentgateway_binary); then
            print_error "Failed to find AgentGateway binary after build"
            return 1
        fi
    fi
    
    print_status "Using AgentGateway binary: $binary"
    
    # Start the backend with test configuration
    if [[ "$VERBOSE" == "true" ]]; then
        "$binary" --file test-config.yaml &
    else
        "$binary" --file test-config.yaml > /dev/null 2>&1 &
    fi
    
    BACKEND_PID=$!
    print_status "AgentGateway backend started (PID: $BACKEND_PID)"
    
    # Wait for it to be ready (use readiness endpoint)
    if ! wait_for_service "http://localhost:15021/healthz/ready" "AgentGateway backend" $TIMEOUT; then
        return 1
    fi
    
    return 0
}

# Function to start UI development server
start_ui() {
    print_status "Starting UI development server..."
    
    # Check if port is already in use
    if lsof -Pi :$UI_PORT -sTCP:LISTEN -t >/dev/null; then
        print_warning "Port $UI_PORT is already in use"
        if curl -s -f "$UI_URL" > /dev/null 2>&1; then
            print_success "UI development server is already running"
            return 0
        else
            print_error "Port $UI_PORT is occupied by another service"
            return 1
        fi
    fi
    
    # Navigate to UI directory
    if [[ ! -d "ui" ]]; then
        print_error "UI directory not found. Please run this script from the project root."
        return 1
    fi
    
    cd ui
    
    # Install dependencies if needed
    if [[ ! -d "node_modules" ]]; then
        print_status "Installing UI dependencies..."
        if [[ "$VERBOSE" == "true" ]]; then
            npm install
        else
            npm install > /dev/null 2>&1
        fi
    fi
    
    # Start the development server
    print_status "Starting Next.js development server..."
    if [[ "$VERBOSE" == "true" ]]; then
        npm run dev &
    else
        npm run dev > /dev/null 2>&1 &
    fi
    
    UI_PID=$!
    print_status "UI development server started (PID: $UI_PID)"
    
    # Wait for it to be ready
    if ! wait_for_service "$UI_URL" "UI development server" $TIMEOUT; then
        cd ..
        return 1
    fi
    
    cd ..
    return 0
}

# Function to detect optimal settings
detect_optimal_settings() {
    if [[ "$AUTO_DETECT" == "false" ]]; then
        print_status "Auto-detection disabled, using provided settings"
        return 0
    fi
    
    print_status "Detecting optimal test configuration..."
    
    # Check if setup config exists from first-time setup
    if [[ -f "$SETUP_CONFIG_FILE" ]]; then
        print_status "Loading configuration from previous setup..."
        if [[ "$VERBOSE" == "true" ]]; then
            print_status "Configuration file: $SETUP_CONFIG_FILE"
        fi
        # Could parse and apply settings here
        return 0
    fi
    
    # Run resource detection if available
    if [[ -f "scripts/detect-system-resources.js" ]]; then
        print_status "Running system resource detection..."
        
        local resource_output
        resource_output=$(node scripts/detect-system-resources.js --quiet 2>/dev/null || echo "")
        
        if [[ -n "$resource_output" ]] && [[ "$resource_output" != *"error"* ]]; then
            # Extract recommended settings (simplified parsing)
            local recommended_workers=$(echo "$resource_output" | grep -o "Max Workers: [0-9]*" | grep -o "[0-9]*" || echo "")
            local recommended_memory=$(echo "$resource_output" | grep -o "Memory Limit: [0-9]*%" | grep -o "[0-9]*" || echo "")
            
            if [[ -n "$recommended_workers" ]] && [[ "$recommended_workers" -gt 0 ]]; then
                if [[ "$PARALLEL_WORKERS" == "4" ]]; then  # Only override if using default
                    PARALLEL_WORKERS="$recommended_workers"
                    print_success "Auto-detected optimal workers: $PARALLEL_WORKERS"
                fi
            fi
            
            if [[ -n "$recommended_memory" ]] && [[ -z "$MEMORY_LIMIT" ]]; then
                MEMORY_LIMIT="$recommended_memory"
                print_success "Auto-detected memory limit: ${MEMORY_LIMIT}%"
            fi
            
            if [[ "$VERBOSE" == "true" ]]; then
                echo "$resource_output"
            fi
        else
            print_warning "Resource detection failed, using conservative defaults"
            # Apply conservative defaults for unknown environments
            if [[ "$PARALLEL_WORKERS" == "4" ]]; then  # Only override if using default
                PARALLEL_WORKERS="2"
                print_status "Using conservative worker count: $PARALLEL_WORKERS"
            fi
        fi
    else
        print_warning "Resource detection script not found, using defaults"
    fi
    
    return 0
}

# Function to validate setup with optional health check
validate_setup() {
    print_status "Validating test environment setup..."
    
    # Run health check if available and requested
    if [[ -f "scripts/health-check-validator.js" ]] && [[ "$AUTO_DETECT" == "true" ]]; then
        print_status "Running quick health check..."
        
        # Run health check without runtime checks (services not started yet)
        if node scripts/health-check-validator.js > /dev/null 2>&1; then
            print_success "Health check passed - system ready for testing"
            return 0
        else
            print_warning "Health check detected issues, but continuing with basic validation"
            # Fall through to basic validation
        fi
    fi
    
    # Basic validation
    local validation_warnings=()
    
    # Check if this looks like a first-time setup
    if [[ ! -f "$SETUP_CONFIG_FILE" ]] && [[ ! -d "ui/node_modules" ]]; then
        validation_warnings+=("This appears to be a first-time setup")
        validation_warnings+=("Consider running: ./scripts/setup-first-time.sh")
    fi
    
    # Check system resources
    local total_memory_kb=$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}' || echo "0")
    local total_memory_gb=$((total_memory_kb / 1024 / 1024))
    
    if [[ "$total_memory_gb" -lt 4 ]]; then
        validation_warnings+=("Low system memory: ${total_memory_gb}GB (recommend 8GB+)")
        if [[ "$PARALLEL_WORKERS" -gt 2 ]]; then
            print_warning "Reducing workers due to low memory"
            PARALLEL_WORKERS="1"
        fi
    fi
    
    # Check available disk space
    local available_space_kb=$(df . | tail -1 | awk '{print $4}')
    local available_space_gb=$((available_space_kb / 1024 / 1024))
    if [[ "$available_space_gb" -lt 1 ]]; then
        validation_warnings+=("Low disk space: ${available_space_gb}GB available")
    fi
    
    # Report validation results
    if [[ ${#validation_warnings[@]} -gt 0 ]]; then
        print_warning "Setup validation warnings:"
        for warning in "${validation_warnings[@]}"; do
            print_warning "  - $warning"
        done
        echo
    else
        print_success "Setup validation passed"
    fi
    
    return 0
}

# Function to provide enhanced error messages
provide_error_guidance() {
    local error_type="$1"
    local exit_code="$2"
    
    echo
    print_error "Test execution failed. Here's how to troubleshoot:"
    echo
    
    case "$error_type" in
        "backend_start")
            cat << EOF
${YELLOW}Backend Startup Issues:${NC}
  1. Check if AgentGateway binary exists:
     ls -la target/release/agentgateway
  
  2. Try building manually:
     cargo build --release --bin agentgateway
  
  3. Check test configuration:
     cat test-config.yaml
  
  4. Verify port availability:
     lsof -i :$BACKEND_PORT
  
  5. Run with verbose output:
     $0 --verbose

EOF
            ;;
        "ui_start")
            cat << EOF
${YELLOW}UI Startup Issues:${NC}
  1. Check Node.js and npm versions:
     node --version && npm --version
  
  2. Install UI dependencies:
     cd ui && npm install
  
  3. Check port availability:
     lsof -i :$UI_PORT
  
  4. Try starting UI manually:
     cd ui && npm run dev
  
  5. Run with verbose output:
     $0 --verbose

EOF
            ;;
        "test_execution")
            cat << EOF
${YELLOW}Test Execution Issues:${NC}
  1. Try with fewer workers:
     $0 --workers 1
  
  2. Run in sequential mode:
     $0 --mode sequential
  
  3. Check system resources:
     node scripts/detect-system-resources.js
  
  4. Run minimal test for debugging:
     node scripts/test-e2e-minimal.js --verbose
  
  5. Check for resource constraints:
     $0 --memory-limit 50 --workers 1

EOF
            ;;
        *)
            cat << EOF
${YELLOW}General Troubleshooting:${NC}
  1. Run first-time setup:
     ./scripts/setup-first-time.sh
  
  2. Check system requirements:
     - Rust toolchain installed
     - Node.js 20+ installed
     - At least 4GB RAM available
     - At least 2GB disk space
  
  3. Run with verbose output:
     $0 --verbose
  
  4. Check documentation:
     - E2E Testing Guide: ui/cypress/README.md
     - Troubleshooting: E2E_TESTING_FIXES.md

EOF
            ;;
    esac
    
    print_status "For more help, check: https://github.com/agentgateway/agentgateway/issues"
}

# Function to run tests
run_tests() {
    print_status "Running E2E tests in $MODE mode..."
    
    cd ui
    
    local exit_code=0
    
    case $MODE in
        "parallel")
            print_status "Running tests with $PARALLEL_WORKERS workers..."
            if [[ -n "$MEMORY_LIMIT" ]]; then
                print_status "Memory limit: ${MEMORY_LIMIT}%"
            fi
            
            if [[ "$VERBOSE" == "true" ]]; then
                npm run test:e2e:parallel -- --workers $PARALLEL_WORKERS --debug
            else
                npm run test:e2e:parallel -- --workers $PARALLEL_WORKERS
            fi
            exit_code=$?
            ;;
        "sequential")
            print_status "Running tests sequentially..."
            if [[ "$HEADLESS" == "true" ]]; then
                npm run e2e
            else
                npm run cypress:run:headed
            fi
            exit_code=$?
            ;;
        "interactive")
            print_status "Opening Cypress test runner..."
            npm run e2e:open
            exit_code=$?
            ;;
    esac
    
    cd ..
    
    if [[ $exit_code -eq 0 ]]; then
        print_success "E2E tests completed successfully!"
    else
        print_error "E2E tests failed with exit code: $exit_code"
        provide_error_guidance "test_execution" "$exit_code"
    fi
    
    return $exit_code
}

# Main execution
main() {
    print_status "AgentGateway E2E Test Runner (Enhanced)"
    print_status "Mode: $MODE"
    print_status "Auto-detection: $AUTO_DETECT"
    print_status "Timeout: ${TIMEOUT}s"
    print_status "Cleanup: $CLEANUP"
    echo
    
    # Check prerequisites
    if ! command -v cargo &> /dev/null; then
        print_error "Cargo is required but not installed"
        print_status "Try running: ./scripts/setup-first-time.sh"
        exit 1
    fi
    
    if ! command -v node &> /dev/null; then
        print_error "Node.js is required but not installed"
        print_status "Try running: ./scripts/setup-first-time.sh"
        exit 1
    fi
    
    if ! command -v npm &> /dev/null; then
        print_error "npm is required but not installed"
        print_status "Try running: ./scripts/setup-first-time.sh"
        exit 1
    fi
    
    # Detect optimal settings and validate setup
    if ! detect_optimal_settings; then
        print_warning "Resource detection had issues, but continuing with defaults"
    fi
    
    if ! validate_setup; then
        print_warning "Setup validation had issues, but continuing"
    fi
    
    # Show final configuration
    print_status "Final configuration:"
    if [[ "$MODE" == "parallel" ]]; then
        print_status "  Workers: $PARALLEL_WORKERS"
        if [[ -n "$MEMORY_LIMIT" ]]; then
            print_status "  Memory limit: ${MEMORY_LIMIT}%"
        fi
    fi
    echo
    
    # Start services
    if ! start_backend; then
        print_error "Failed to start AgentGateway backend"
        provide_error_guidance "backend_start" "1"
        exit 1
    fi
    
    if ! start_ui; then
        print_error "Failed to start UI development server"
        provide_error_guidance "ui_start" "1"
        exit 1
    fi
    
    # Run tests
    if ! run_tests; then
        print_error "E2E tests failed"
        exit 1
    fi
    
    print_success "All E2E tests completed successfully!"
    
    # Show summary
    echo
    print_status "Test execution summary:"
    print_status "  Mode: $MODE"
    if [[ "$MODE" == "parallel" ]]; then
        print_status "  Workers used: $PARALLEL_WORKERS"
    fi
    print_status "  Backend: AgentGateway (PID: $BACKEND_PID)"
    print_status "  UI: Next.js dev server (PID: $UI_PID)"
    
    if [[ "$CLEANUP" == "false" ]]; then
        echo
        print_warning "Services are still running (--no-cleanup was used)"
        print_status "To stop manually:"
        print_status "  Backend: kill $BACKEND_PID"
        print_status "  UI: kill $UI_PID"
    fi
}

# Run main function
main "$@"
