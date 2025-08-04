#!/bin/bash

# AgentGateway Docker-based Fortio Benchmarking Runner
# This script orchestrates the complete benchmarking process using Docker

set -euo pipefail

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TRAFFIC_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT_ROOT="$(cd "$TRAFFIC_DIR/../../../.." && pwd)"

# Default values
PROTOCOLS="all"
TEST_TYPE="quick"
DURATION="30s"
VERBOSE=false
CLEANUP=true
BUILD_IMAGES=true

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Logging functions
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

# Help function
show_help() {
    cat << EOF
AgentGateway Docker-based Fortio Benchmarking Runner
====================================================

Usage: $0 [OPTIONS]

Options:
  --protocols PROTO    Protocols to test: all, http, mcp, a2a (default: all)
  --type TYPE          Benchmark type: comprehensive, quick, latency, throughput (default: quick)
  --duration DURATION  Test duration (default: 30s)
  --no-build          Skip building Docker images
  --no-cleanup        Skip cleanup after tests
  --verbose           Enable verbose output
  --help              Show this help message

Examples:
  $0                                           # Run all protocols, quick tests
  $0 --protocols http --type comprehensive     # Comprehensive HTTP tests
  $0 --protocols mcp --duration 60s           # MCP tests for 60 seconds
  $0 --no-build --verbose                     # Skip build, verbose output

Docker Services:
  - agentgateway: The proxy being tested
  - test-server: Backend server for testing
  - fortio-benchmark: Fortio load testing tool
  - report-generator: Generates comparison reports

Results:
  - Raw Fortio JSON results: traffic/results/
  - HTML report: traffic/results/benchmark_comparison_report.html
  - Markdown summary: traffic/results/benchmark_summary.md

EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --protocols)
            PROTOCOLS="$2"
            shift 2
            ;;
        --type)
            TEST_TYPE="$2"
            shift 2
            ;;
        --duration)
            DURATION="$2"
            shift 2
            ;;
        --no-build)
            BUILD_IMAGES=false
            shift
            ;;
        --no-cleanup)
            CLEANUP=false
            shift
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --help)
            show_help
            exit 0
            ;;
        *)
            log_error "Unknown option: $1"
            show_help
            exit 1
            ;;
    esac
done

# Validate protocols
case $PROTOCOLS in
    all|http|mcp|a2a) ;;
    *) log_error "Invalid protocol: $PROTOCOLS"; exit 1 ;;
esac

# Validate test type
case $TEST_TYPE in
    comprehensive|quick|latency|throughput) ;;
    *) log_error "Invalid test type: $TEST_TYPE"; exit 1 ;;
esac

# Check Docker availability
check_docker() {
    if ! command -v docker &> /dev/null; then
        log_error "Docker is not installed or not in PATH"
        exit 1
    fi

    if ! command -v docker-compose &> /dev/null && ! docker compose version &> /dev/null; then
        log_error "Docker Compose is not installed or not in PATH"
        exit 1
    fi

    if ! docker info &> /dev/null; then
        log_error "Docker daemon is not running"
        exit 1
    fi
}

# Build Docker images
build_images() {
    if [ "$BUILD_IMAGES" = false ]; then
        log_info "Skipping Docker image build"
        return
    fi

    log_info "Building Docker images..."
    
    cd "$SCRIPT_DIR"
    
    # Build Fortio image
    log_info "Building Fortio benchmark image..."
    if [ "$VERBOSE" = true ]; then
        docker build -f Dockerfile.fortio -t agentgateway-fortio ..
    else
        docker build -f Dockerfile.fortio -t agentgateway-fortio .. > /dev/null 2>&1
    fi
    
    # Build test server image
    log_info "Building test server image..."
    if [ "$VERBOSE" = true ]; then
        docker build -f Dockerfile.test-server -t agentgateway-test-server "$PROJECT_ROOT"
    else
        docker build -f Dockerfile.test-server -t agentgateway-test-server "$PROJECT_ROOT" > /dev/null 2>&1
    fi
    
    log_success "Docker images built successfully"
}

# Setup results directory
setup_results() {
    local results_dir="$TRAFFIC_DIR/results"
    mkdir -p "$results_dir"
    
    # Clean previous results if they exist
    if [ -d "$results_dir" ] && [ "$(ls -A "$results_dir")" ]; then
        log_warning "Cleaning previous results..."
        rm -rf "$results_dir"/*
    fi
    
    log_info "Results will be saved to: $results_dir"
}

# Start infrastructure services
start_infrastructure() {
    log_info "Starting infrastructure services..."
    
    cd "$SCRIPT_DIR"
    
    # Start AgentGateway and test server
    if [ "$VERBOSE" = true ]; then
        docker-compose -f docker-compose.benchmark.yml up -d agentgateway test-server
    else
        docker-compose -f docker-compose.benchmark.yml up -d agentgateway test-server > /dev/null 2>&1
    fi
    
    # Wait for services to be healthy
    log_info "Waiting for services to be ready..."
    local max_wait=60
    local wait_time=0
    
    while [ $wait_time -lt $max_wait ]; do
        if docker-compose -f docker-compose.benchmark.yml ps | grep -q "healthy"; then
            local healthy_count=$(docker-compose -f docker-compose.benchmark.yml ps | grep -c "healthy" || true)
            if [ "$healthy_count" -ge 2 ]; then
                log_success "All services are healthy"
                return 0
            fi
        fi
        
        sleep 2
        wait_time=$((wait_time + 2))
        
        if [ $((wait_time % 10)) -eq 0 ]; then
            log_info "Still waiting for services... (${wait_time}s/${max_wait}s)"
        fi
    done
    
    log_error "Services failed to become healthy within ${max_wait} seconds"
    docker-compose -f docker-compose.benchmark.yml logs
    return 1
}

# Run benchmarks
run_benchmarks() {
    log_info "Running Fortio benchmarks..."
    log_info "Protocols: $PROTOCOLS, Type: $TEST_TYPE, Duration: $DURATION"
    
    cd "$SCRIPT_DIR"
    
    # Prepare benchmark command
    local benchmark_cmd="./fortio-tests.sh --protocols $PROTOCOLS --type $TEST_TYPE --duration $DURATION"
    if [ "$VERBOSE" = true ]; then
        benchmark_cmd="$benchmark_cmd --verbose"
    fi
    
    # Run benchmarks in Docker
    if [ "$VERBOSE" = true ]; then
        docker-compose -f docker-compose.benchmark.yml run --rm \
            -e AGENTGATEWAY_URL=http://agentgateway:8080 \
            -e BACKEND_URL=http://test-server:3001 \
            fortio-benchmark $benchmark_cmd
    else
        docker-compose -f docker-compose.benchmark.yml run --rm \
            -e AGENTGATEWAY_URL=http://agentgateway:8080 \
            -e BACKEND_URL=http://test-server:3001 \
            fortio-benchmark $benchmark_cmd > /dev/null 2>&1
    fi
    
    local exit_code=$?
    
    if [ $exit_code -eq 0 ]; then
        log_success "Benchmarks completed successfully"
    else
        log_error "Benchmarks failed with exit code: $exit_code"
        return $exit_code
    fi
}

# Generate reports
generate_reports() {
    log_info "Generating benchmark reports..."
    
    cd "$SCRIPT_DIR"
    
    # Check if results exist
    if [ ! -d "$TRAFFIC_DIR/results" ] || [ -z "$(ls -A "$TRAFFIC_DIR/results" 2>/dev/null)" ]; then
        log_warning "No benchmark results found, skipping report generation"
        return 0
    fi
    
    # Generate reports
    if [ "$VERBOSE" = true ]; then
        docker-compose -f docker-compose.benchmark.yml run --rm report-generator
    else
        docker-compose -f docker-compose.benchmark.yml run --rm report-generator > /dev/null 2>&1
    fi
    
    local exit_code=$?
    
    if [ $exit_code -eq 0 ]; then
        log_success "Reports generated successfully"
        
        # Show report locations
        local results_dir="$TRAFFIC_DIR/results"
        if [ -f "$results_dir/benchmark_comparison_report.html" ]; then
            log_info "HTML Report: $results_dir/benchmark_comparison_report.html"
        fi
        if [ -f "$results_dir/benchmark_summary.md" ]; then
            log_info "Markdown Summary: $results_dir/benchmark_summary.md"
        fi
    else
        log_error "Report generation failed with exit code: $exit_code"
        return $exit_code
    fi
}

# Cleanup function
cleanup() {
    if [ "$CLEANUP" = false ]; then
        log_info "Skipping cleanup (--no-cleanup specified)"
        return
    fi
    
    log_info "Cleaning up Docker services..."
    
    cd "$SCRIPT_DIR"
    
    if [ "$VERBOSE" = true ]; then
        docker-compose -f docker-compose.benchmark.yml down
    else
        docker-compose -f docker-compose.benchmark.yml down > /dev/null 2>&1
    fi
    
    log_success "Cleanup completed"
}

# Trap cleanup on exit
trap cleanup EXIT

# Main execution
main() {
    log_info "Starting AgentGateway Docker-based Fortio Benchmarking"
    log_info "Configuration: protocols=$PROTOCOLS, type=$TEST_TYPE, duration=$DURATION"
    
    # Pre-flight checks
    check_docker
    
    # Setup
    setup_results
    build_images
    
    # Execute benchmarking
    start_infrastructure
    run_benchmarks
    generate_reports
    
    log_success "Benchmarking completed successfully!"
    log_info "Check the results directory for detailed reports: $TRAFFIC_DIR/results/"
}

# Run main function
main "$@"
