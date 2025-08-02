#!/bin/bash

# E2E Setup Validation Script
# This script tests our improvements in a clean environment

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

print_status() {
    echo -e "${BLUE}[TEST]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
}

print_error() {
    echo -e "${RED}[FAIL]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

# Test results tracking
TESTS_PASSED=0
TESTS_FAILED=0
TEST_RESULTS=()

run_test() {
    local test_name="$1"
    local test_command="$2"
    
    print_status "Running test: $test_name"
    
    if eval "$test_command" > /tmp/test_output 2>&1; then
        print_success "$test_name"
        TESTS_PASSED=$((TESTS_PASSED + 1))
        TEST_RESULTS+=("PASS: $test_name")
    else
        print_error "$test_name"
        echo "Error output:"
        cat /tmp/test_output
        TESTS_FAILED=$((TESTS_FAILED + 1))
        TEST_RESULTS+=("FAIL: $test_name")
    fi
    echo
}

print_status "=== E2E Setup Infrastructure Validation ==="
print_status "Testing AgentGateway E2E setup improvements in clean environment"
echo

# Test 1: Verify setup script exists and is executable
run_test "Setup script exists and is executable" \
    "test -x scripts/setup-first-time.sh"

# Test 2: Verify enhanced test runner exists and is executable  
run_test "Enhanced test runner exists and is executable" \
    "test -x scripts/run-e2e-tests.sh"

# Test 3: Test setup script help/usage
run_test "Setup script shows help" \
    "scripts/setup-first-time.sh --help | grep -q 'One-command setup'"

# Test 4: Test enhanced test runner help/usage
run_test "Enhanced test runner shows help" \
    "scripts/run-e2e-tests.sh --help | grep -q 'Enhanced'"

# Test 5: Test setup script dry-run mode
run_test "Setup script dry-run mode works" \
    "scripts/setup-first-time.sh --dry-run | grep -q 'DRY RUN MODE'"

# Test 6: Test resource detection integration
run_test "Resource detection script exists" \
    "test -f scripts/detect-system-resources.js"

# Test 7: Verify setup script can detect missing dependencies
run_test "Setup script detects system state" \
    "scripts/setup-first-time.sh --dry-run --verbose | grep -q 'Checking System Prerequisites'"

# Test 8: Test enhanced test runner auto-detection
run_test "Enhanced test runner has auto-detection" \
    "scripts/run-e2e-tests.sh --help | grep -q 'auto-detect'"

# Test 9: Verify memory bank documentation exists
run_test "Memory bank documentation exists" \
    "test -f memory-bank/activeContext.md && test -f memory-bank/progress.md"

# Test 10: Test setup script prerequisite checking
run_test "Setup script prerequisite checking works" \
    "timeout 30 scripts/setup-first-time.sh --dry-run --skip-deps | grep -q 'Prerequisites'"

# Test 11: Verify test configuration exists
run_test "Test configuration file exists" \
    "test -f test-config.yaml"

# Test 12: Test enhanced error guidance
run_test "Enhanced test runner provides error guidance" \
    "scripts/run-e2e-tests.sh --help | grep -q 'Enhanced error messages'"

# Test 13: Test actual setup script execution (dry-run)
run_test "Setup script can run full dry-run without errors" \
    "timeout 60 scripts/setup-first-time.sh --dry-run --skip-deps --skip-build"

# Test 14: Test resource detection script execution
run_test "Resource detection script can execute" \
    "timeout 30 node scripts/detect-system-resources.js --help | grep -q 'Usage'"

# Test 15: Verify enhanced test runner can show configuration
run_test "Enhanced test runner shows configuration info" \
    "scripts/run-e2e-tests.sh --help | grep -q 'FIRST-TIME SETUP'"

# Summary
echo
print_status "=== Test Results Summary ==="
echo "Tests passed: $TESTS_PASSED"
echo "Tests failed: $TESTS_FAILED"
echo "Total tests: $((TESTS_PASSED + TESTS_FAILED))"
echo

if [ $TESTS_FAILED -eq 0 ]; then
    print_success "All tests passed! E2E setup improvements are working correctly."
    
    # Show some additional validation info
    echo
    print_status "=== Additional Validation Info ==="
    echo "Setup script size: $(wc -l < scripts/setup-first-time.sh) lines"
    echo "Enhanced test runner size: $(wc -l < scripts/run-e2e-tests.sh) lines"
    echo "Memory bank files: $(ls -la memory-bank/ | wc -l) files"
    echo
    
    exit 0
else
    print_error "Some tests failed. E2E setup improvements need attention."
    echo
    print_status "Detailed results:"
    for result in "${TEST_RESULTS[@]}"; do
        echo "  $result"
    done
    exit 1
fi
