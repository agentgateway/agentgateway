#!/bin/bash

# AgentGateway Docker Benchmark Runner
# This script runs AgentGateway benchmarks in Docker with full multi-process support

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
DOCKER_DIR="$PROJECT_ROOT/docker"

echo -e "${BLUE}🚀 AgentGateway Docker Benchmark Runner${NC}"
echo -e "${BLUE}=======================================${NC}"

# Check if Docker is installed
if ! command -v docker &> /dev/null; then
    echo -e "${RED}❌ Docker is not installed or not in PATH${NC}"
    echo "Please install Docker and try again."
    exit 1
fi

# Check if Docker Compose is available
if ! command -v docker-compose &> /dev/null && ! docker compose version &> /dev/null; then
    echo -e "${RED}❌ Docker Compose is not available${NC}"
    echo "Please install Docker Compose and try again."
    exit 1
fi

# Function to run docker compose (handle both docker-compose and docker compose)
run_docker_compose() {
    if command -v docker-compose &> /dev/null; then
        docker-compose "$@"
    else
        docker compose "$@"
    fi
}

# Parse command line arguments
BENCHMARK_TYPE="all"
CLEAN_BUILD=false
INTERACTIVE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --type)
            BENCHMARK_TYPE="$2"
            shift 2
            ;;
        --clean)
            CLEAN_BUILD=true
            shift
            ;;
        --interactive)
            INTERACTIVE=true
            shift
            ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --type TYPE        Benchmark type: all, real-proxy, baseline, integration"
            echo "  --clean           Clean build (remove existing containers and images)"
            echo "  --interactive     Run in interactive mode"
            echo "  --help            Show this help message"
            echo ""
            echo "Examples:"
            echo "  $0                           # Run all benchmarks"
            echo "  $0 --type real-proxy         # Run only real proxy benchmarks"
            echo "  $0 --clean                   # Clean build and run all benchmarks"
            echo "  $0 --interactive             # Run in interactive mode"
            exit 0
            ;;
        *)
            echo -e "${RED}❌ Unknown option: $1${NC}"
            echo "Use --help for usage information."
            exit 1
            ;;
    esac
done

# Change to docker directory
cd "$DOCKER_DIR"

echo -e "${YELLOW}📋 Configuration:${NC}"
echo "  Benchmark Type: $BENCHMARK_TYPE"
echo "  Clean Build: $CLEAN_BUILD"
echo "  Interactive: $INTERACTIVE"
echo "  Project Root: $PROJECT_ROOT"
echo "  Docker Dir: $DOCKER_DIR"
echo ""

# Clean build if requested
if [ "$CLEAN_BUILD" = true ]; then
    echo -e "${YELLOW}🧹 Cleaning existing containers and images...${NC}"
    run_docker_compose -f docker-compose.benchmark.yml down --rmi all --volumes --remove-orphans || true
    docker system prune -f || true
    echo -e "${GREEN}✅ Cleanup completed${NC}"
    echo ""
fi

# Clean up any existing networks that might have configuration conflicts
echo -e "${YELLOW}🧹 Cleaning up Docker networks...${NC}"
run_docker_compose -f docker-compose.benchmark.yml down --remove-orphans 2>/dev/null || true
docker network prune -f 2>/dev/null || true

# Build the benchmark image
echo -e "${YELLOW}🔨 Building benchmark Docker image...${NC}"
run_docker_compose -f docker-compose.benchmark.yml build
echo -e "${GREEN}✅ Build completed${NC}"
echo ""

# Run benchmarks based on type
case $BENCHMARK_TYPE in
    "all")
        echo -e "${YELLOW}🏃 Running all benchmarks...${NC}"
        if [ "$INTERACTIVE" = true ]; then
            run_docker_compose -f docker-compose.benchmark.yml run --rm agentgateway-benchmark bash
        else
            run_docker_compose -f docker-compose.benchmark.yml run --rm agentgateway-benchmark
        fi
        ;;
    "real-proxy")
        echo -e "${YELLOW}🏃 Running real proxy benchmarks...${NC}"
        if [ "$INTERACTIVE" = true ]; then
            run_docker_compose -f docker-compose.benchmark.yml run --rm real-proxy-benchmark bash
        else
            run_docker_compose -f docker-compose.benchmark.yml run --rm real-proxy-benchmark
        fi
        ;;
    "baseline")
        echo -e "${YELLOW}🏃 Running baseline comparison benchmarks...${NC}"
        run_docker_compose -f docker-compose.benchmark.yml run --rm agentgateway-benchmark \
            cargo bench --package agentgateway --features internal_benches --bench baseline_comparison
        ;;
    "integration")
        echo -e "${YELLOW}🏃 Running integration test benchmarks...${NC}"
        run_docker_compose -f docker-compose.benchmark.yml run --rm agentgateway-benchmark \
            cargo bench --package agentgateway --features internal_benches --bench integration_test
        ;;
    *)
        echo -e "${RED}❌ Unknown benchmark type: $BENCHMARK_TYPE${NC}"
        echo "Valid types: all, real-proxy, baseline, integration"
        exit 1
        ;;
esac

# Check if benchmark reports were generated
REPORTS_DIR="$PROJECT_ROOT/target/benchmark_reports"
if [ -d "$REPORTS_DIR" ] && [ "$(ls -A "$REPORTS_DIR" 2>/dev/null)" ]; then
    echo ""
    echo -e "${GREEN}📊 Benchmark reports generated:${NC}"
    ls -la "$REPORTS_DIR"
    echo ""
    echo -e "${BLUE}📁 Reports location: $REPORTS_DIR${NC}"
    
    # Show HTML report if available
    HTML_REPORT="$REPORTS_DIR/benchmark_report.html"
    if [ -f "$HTML_REPORT" ]; then
        echo -e "${BLUE}🌐 HTML Report: file://$HTML_REPORT${NC}"
    fi
else
    echo -e "${YELLOW}⚠️  No benchmark reports found in $REPORTS_DIR${NC}"
fi

# Show baseline comparison report if available
BASELINE_REPORT="$PROJECT_ROOT/target/baseline_comparison_report.md"
if [ -f "$BASELINE_REPORT" ]; then
    echo -e "${BLUE}📈 Baseline Comparison: $BASELINE_REPORT${NC}"
fi

echo ""
echo -e "${GREEN}🎉 Benchmark execution completed!${NC}"

# Cleanup containers
echo -e "${YELLOW}🧹 Cleaning up containers...${NC}"
run_docker_compose -f docker-compose.benchmark.yml down
echo -e "${GREEN}✅ Cleanup completed${NC}"
