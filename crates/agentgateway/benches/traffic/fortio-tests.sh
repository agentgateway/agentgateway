#!/bin/bash

# AgentGateway Fortio Traffic Testing Suite
# Multi-process architecture for real-world proxy performance testing

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Script directory and paths
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"
TRAFFIC_DIR="$SCRIPT_DIR"
RESULTS_DIR="$TRAFFIC_DIR/results"

echo -e "${BLUE}🚀 AgentGateway Fortio Traffic Testing Suite${NC}"
echo -e "${BLUE}=============================================${NC}"

# Configuration
PROXY_PORT=8080
BACKEND_PORT=3001
MCP_BACKEND_PORT=3002
A2A_BACKEND_PORT=3003
TEST_DURATION=60s
CONCURRENCY_LEVELS=(16 64 256 512)

# Parse command line arguments
PROTOCOLS="all"
BENCHMARK_TYPE="comprehensive"
VERBOSE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --protocols)
            PROTOCOLS="$2"
            shift 2
            ;;
        --type)
            BENCHMARK_TYPE="$2"
            shift 2
            ;;
        --duration)
            TEST_DURATION="$2"
            shift 2
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --protocols PROTO    Protocols to test: all, http, mcp, a2a"
            echo "  --type TYPE          Benchmark type: comprehensive, quick, latency, throughput"
            echo "  --duration DURATION  Test duration (default: 60s)"
            echo "  --verbose           Enable verbose output"
            echo "  --help              Show this help message"
            echo ""
            echo "Examples:"
            echo "  $0                                    # Run all protocols, comprehensive tests"
            echo "  $0 --protocols http --type quick      # Quick HTTP tests only"
            echo "  $0 --protocols mcp --duration 30s     # MCP tests for 30 seconds"
            exit 0
            ;;
        *)
            echo -e "${RED}❌ Unknown option: $1${NC}"
            echo "Use --help for usage information."
            exit 1
            ;;
    esac
done

# Check if Fortio is installed
if ! command -v fortio &> /dev/null; then
    echo -e "${RED}❌ Fortio is not installed${NC}"
    echo "Please install Fortio:"
    echo "  # Linux/macOS:"
    echo "  curl -L https://github.com/fortio/fortio/releases/download/v1.60.3/fortio_linux_amd64.tar.gz | tar xz"
    echo "  sudo mv fortio /usr/local/bin/"
    echo ""
    echo "  # Or using Go:"
    echo "  go install fortio.org/fortio@latest"
    exit 1
fi

# Create results directory
mkdir -p "$RESULTS_DIR"

echo -e "${YELLOW}📋 Configuration:${NC}"
echo "  Protocols: $PROTOCOLS"
echo "  Benchmark Type: $BENCHMARK_TYPE"
echo "  Test Duration: $TEST_DURATION"
echo "  Proxy Port: $PROXY_PORT"
echo "  Backend Ports: $BACKEND_PORT (HTTP), $MCP_BACKEND_PORT (MCP), $A2A_BACKEND_PORT (A2A)"
echo "  Results Directory: $RESULTS_DIR"
echo ""

# Build AgentGateway and test servers
echo -e "${YELLOW}🔨 Building AgentGateway and test servers...${NC}"
cd "$PROJECT_ROOT"
cargo build --release --bin agentgateway
cargo build --release --bin test-server
echo -e "${GREEN}✅ Build completed${NC}"
echo ""

# PID tracking for cleanup
BACKEND_PIDS=()
PROXY_PID=""

# Cleanup function
cleanup() {
    echo -e "${YELLOW}🧹 Cleaning up processes...${NC}"
    
    if [ -n "$PROXY_PID" ]; then
        kill $PROXY_PID 2>/dev/null || true
        echo "  Stopped AgentGateway proxy (PID: $PROXY_PID)"
    fi
    
    for pid in "${BACKEND_PIDS[@]}"; do
        kill $pid 2>/dev/null || true
        echo "  Stopped backend server (PID: $pid)"
    done
    
    # Wait a moment for graceful shutdown
    sleep 2
    
    # Force kill if still running
    if [ -n "$PROXY_PID" ]; then
        kill -9 $PROXY_PID 2>/dev/null || true
    fi
    
    for pid in "${BACKEND_PIDS[@]}"; do
        kill -9 $pid 2>/dev/null || true
    done
    
    echo -e "${GREEN}✅ Cleanup completed${NC}"
}

# Set up signal handlers
trap cleanup EXIT INT TERM

# Function to wait for service to be ready
wait_for_service() {
    local port=$1
    local service_name=$2
    local max_attempts=30
    local attempt=0
    
    echo -n "  Waiting for $service_name on port $port"
    
    while [ $attempt -lt $max_attempts ]; do
        if curl -s "http://localhost:$port/health" >/dev/null 2>&1 || \
           nc -z localhost $port >/dev/null 2>&1; then
            echo -e " ${GREEN}✅${NC}"
            return 0
        fi
        
        echo -n "."
        sleep 1
        ((attempt++))
    done
    
    echo -e " ${RED}❌${NC}"
    echo -e "${RED}❌ $service_name failed to start on port $port${NC}"
    return 1
}

# Function to run HTTP proxy tests
run_http_tests() {
    echo -e "${YELLOW}=== HTTP Proxy Performance Tests ===${NC}"
    
    # Start HTTP backend server
    echo -e "${YELLOW}🚀 Starting HTTP backend server...${NC}"
    ./target/release/test-server --port $BACKEND_PORT &
    local backend_pid=$!
    BACKEND_PIDS+=($backend_pid)
    
    wait_for_service $BACKEND_PORT "HTTP backend"
    
    # Start AgentGateway proxy
    echo -e "${YELLOW}🚀 Starting AgentGateway proxy...${NC}"
    ./target/release/agentgateway --config "$TRAFFIC_DIR/configs/http-proxy.yaml" &
    PROXY_PID=$!
    
    wait_for_service $PROXY_PORT "AgentGateway proxy"
    
    # Run tests based on benchmark type
    case $BENCHMARK_TYPE in
        "quick")
            CONCURRENCY_LEVELS=(16 64)
            ;;
        "latency")
            CONCURRENCY_LEVELS=(16 32 64)
            ;;
        "throughput")
            CONCURRENCY_LEVELS=(64 256 512)
            ;;
        "comprehensive")
            CONCURRENCY_LEVELS=(16 64 256 512)
            ;;
    esac
    
    for concurrency in "${CONCURRENCY_LEVELS[@]}"; do
        echo -e "${BLUE}📊 Testing HTTP proxy with concurrency: $concurrency${NC}"
        
        # Latency test
        echo "  Running latency test..."
        fortio load -c $concurrency -t $TEST_DURATION -a \
            -json "$RESULTS_DIR/http-latency-c${concurrency}.json" \
            http://localhost:$PROXY_PORT/test
        
        # Throughput test (if not quick mode)
        if [ "$BENCHMARK_TYPE" != "quick" ]; then
            echo "  Running throughput test..."
            fortio load -c $concurrency -qps 0 -t $TEST_DURATION -a \
                -json "$RESULTS_DIR/http-throughput-c${concurrency}.json" \
                http://localhost:$PROXY_PORT/test
        fi
        
        # Payload size tests (comprehensive mode only)
        if [ "$BENCHMARK_TYPE" = "comprehensive" ]; then
            for payload_size in 1024 10240 102400; do # 1KB, 10KB, 100KB
                echo "  Running payload test (${payload_size}B)..."
                fortio load -c $concurrency -t 30s -a \
                    -json "$RESULTS_DIR/http-payload-${payload_size}B-c${concurrency}.json" \
                    -payload-size $payload_size \
                    http://localhost:$PROXY_PORT/test
            done
        fi
    done
    
    # Stop services
    kill $PROXY_PID 2>/dev/null || true
    kill $backend_pid 2>/dev/null || true
    PROXY_PID=""
    BACKEND_PIDS=("${BACKEND_PIDS[@]/$backend_pid}")
    
    sleep 2
    echo -e "${GREEN}✅ HTTP tests completed${NC}"
    echo ""
}

# Function to run MCP protocol tests
run_mcp_tests() {
    echo -e "${YELLOW}=== MCP Protocol Performance Tests ===${NC}"
    
    # Start MCP backend server
    echo -e "${YELLOW}🚀 Starting MCP backend server...${NC}"
    ./target/release/test-server --port $MCP_BACKEND_PORT --protocol mcp &
    local backend_pid=$!
    BACKEND_PIDS+=($backend_pid)
    
    wait_for_service $MCP_BACKEND_PORT "MCP backend"
    
    # Start AgentGateway proxy with MCP configuration
    echo -e "${YELLOW}🚀 Starting AgentGateway proxy (MCP mode)...${NC}"
    ./target/release/agentgateway --config "$TRAFFIC_DIR/configs/mcp-proxy.yaml" &
    PROXY_PID=$!
    
    wait_for_service $PROXY_PORT "AgentGateway proxy (MCP)"
    
    # Test different MCP message types
    local mcp_messages=("initialize" "list_resources" "call_tool" "get_prompt")
    
    for msg_type in "${mcp_messages[@]}"; do
        echo -e "${BLUE}📊 Testing MCP message type: $msg_type${NC}"
        
        # Use custom payload for MCP messages
        fortio load -c 64 -t 30s -a \
            -json "$RESULTS_DIR/mcp-${msg_type}.json" \
            -payload "@$TRAFFIC_DIR/payloads/mcp-${msg_type}.json" \
            -H "Content-Type: application/json" \
            http://localhost:$PROXY_PORT/mcp
    done
    
    # Concurrent session test
    if [ "$BENCHMARK_TYPE" = "comprehensive" ]; then
        echo -e "${BLUE}📊 Testing MCP concurrent sessions${NC}"
        fortio load -c 128 -t $TEST_DURATION -a \
            -json "$RESULTS_DIR/mcp-concurrent-sessions.json" \
            -payload "@$TRAFFIC_DIR/payloads/mcp-initialize.json" \
            -H "Content-Type: application/json" \
            http://localhost:$PROXY_PORT/mcp
    fi
    
    # Stop services
    kill $PROXY_PID 2>/dev/null || true
    kill $backend_pid 2>/dev/null || true
    PROXY_PID=""
    BACKEND_PIDS=("${BACKEND_PIDS[@]/$backend_pid}")
    
    sleep 2
    echo -e "${GREEN}✅ MCP tests completed${NC}"
    echo ""
}

# Function to run A2A protocol tests
run_a2a_tests() {
    echo -e "${YELLOW}=== A2A Protocol Performance Tests ===${NC}"
    
    # Start A2A backend server
    echo -e "${YELLOW}🚀 Starting A2A backend server...${NC}"
    ./target/release/test-server --port $A2A_BACKEND_PORT --protocol a2a &
    local backend_pid=$!
    BACKEND_PIDS+=($backend_pid)
    
    wait_for_service $A2A_BACKEND_PORT "A2A backend"
    
    # Start AgentGateway proxy with A2A configuration
    echo -e "${YELLOW}🚀 Starting AgentGateway proxy (A2A mode)...${NC}"
    ./target/release/agentgateway --config "$TRAFFIC_DIR/configs/a2a-proxy.yaml" &
    PROXY_PID=$!
    
    wait_for_service $PROXY_PORT "AgentGateway proxy (A2A)"
    
    # Test different A2A operations
    local a2a_operations=("discovery" "capability_exchange" "message_routing")
    
    for operation in "${a2a_operations[@]}"; do
        echo -e "${BLUE}📊 Testing A2A operation: $operation${NC}"
        
        fortio load -c 64 -t 30s -a \
            -json "$RESULTS_DIR/a2a-${operation}.json" \
            -payload "@$TRAFFIC_DIR/payloads/a2a-${operation}.json" \
            -H "Content-Type: application/json" \
            http://localhost:$PROXY_PORT/a2a
    done
    
    # Multi-hop communication test
    if [ "$BENCHMARK_TYPE" = "comprehensive" ]; then
        echo -e "${BLUE}📊 Testing A2A multi-hop communication${NC}"
        fortio load -c 32 -t $TEST_DURATION -a \
            -json "$RESULTS_DIR/a2a-multi-hop.json" \
            -payload "@$TRAFFIC_DIR/payloads/a2a-message_routing.json" \
            -H "Content-Type: application/json" \
            http://localhost:$PROXY_PORT/a2a
    fi
    
    # Stop services
    kill $PROXY_PID 2>/dev/null || true
    kill $backend_pid 2>/dev/null || true
    PROXY_PID=""
    BACKEND_PIDS=("${BACKEND_PIDS[@]/$backend_pid}")
    
    sleep 2
    echo -e "${GREEN}✅ A2A tests completed${NC}"
    echo ""
}

# Main execution
echo -e "${YELLOW}🏃 Starting Fortio traffic tests...${NC}"

case $PROTOCOLS in
    "http")
        run_http_tests
        ;;
    "mcp")
        run_mcp_tests
        ;;
    "a2a")
        run_a2a_tests
        ;;
    "all")
        run_http_tests
        run_mcp_tests
        run_a2a_tests
        ;;
    *)
        echo -e "${RED}❌ Unknown protocol: $PROTOCOLS${NC}"
        echo "Valid protocols: all, http, mcp, a2a"
        exit 1
        ;;
esac

# Generate comparison report
echo -e "${YELLOW}📊 Generating comparison report...${NC}"
if [ -f "$TRAFFIC_DIR/reports/generate-comparison.py" ]; then
    cd "$TRAFFIC_DIR/reports"
    python3 generate-comparison.py "$RESULTS_DIR"
    echo -e "${GREEN}✅ Comparison report generated${NC}"
else
    echo -e "${YELLOW}⚠️  Comparison report generator not found${NC}"
fi

# Summary
echo ""
echo -e "${GREEN}🎉 Fortio traffic testing completed!${NC}"
echo -e "${BLUE}📁 Results saved to: $RESULTS_DIR${NC}"

if [ "$(ls -A "$RESULTS_DIR" 2>/dev/null)" ]; then
    echo -e "${BLUE}📊 Generated reports:${NC}"
    ls -la "$RESULTS_DIR"/*.json 2>/dev/null | sed 's/^/  /'
    
    # Show summary statistics
    echo ""
    echo -e "${BLUE}📈 Quick Summary:${NC}"
    for json_file in "$RESULTS_DIR"/*.json; do
        if [ -f "$json_file" ]; then
            local filename=$(basename "$json_file")
            local p95=$(jq -r '.DurationHistogram.Percentiles[] | select(.Percentile == 95) | .Value' "$json_file" 2>/dev/null || echo "N/A")
            local qps=$(jq -r '.ActualQPS' "$json_file" 2>/dev/null || echo "N/A")
            echo "  $filename: p95=${p95}s, QPS=${qps}"
        fi
    done
else
    echo -e "${YELLOW}⚠️  No results found in $RESULTS_DIR${NC}"
fi

echo ""
echo -e "${BLUE}🔗 Next steps:${NC}"
echo "  1. Review results in $RESULTS_DIR"
echo "  2. Run comparison analysis: python3 $TRAFFIC_DIR/reports/generate-comparison.py"
echo "  3. Compare with published baselines"
echo "  4. Integrate into CI/CD pipeline"
