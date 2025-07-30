#!/usr/bin/env bash

# Test script for get-agentproxy install script
# This tests the version checking functionality that was fixed in GitHub issue #61

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test counters
TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0

# Helper functions
log_info() {
    echo -e "${YELLOW}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[PASS]${NC} $1"
    ((TESTS_PASSED++))
}

log_error() {
    echo -e "${RED}[FAIL]${NC} $1"
    ((TESTS_FAILED++))
}

run_test() {
    ((TESTS_RUN++))
    echo -e "\n${YELLOW}Running test: $1${NC}"
}

# Test 1: Verify agentgateway binary supports --version flag
test_version_flag_works() {
    run_test "agentgateway --version flag works"
    
    # Build the binary if it doesn't exist
    if [ ! -f "./target/debug/agentgateway" ]; then
        log_info "Building agentgateway binary for testing..."
        unset CARGO_BUILD_JOBS
        cargo build --bin agentgateway >/dev/null 2>&1
    fi
    
    # Test that --version works
    if ./target/debug/agentgateway --version >/dev/null 2>&1; then
        log_success "agentgateway --version works correctly"
    else
        log_error "agentgateway --version failed"
        return 1
    fi
}

# Test 2: Verify agentgateway binary rejects 'version' subcommand
test_version_subcommand_fails() {
    run_test "agentgateway version subcommand fails as expected"
    
    # Test that 'version' subcommand fails with expected error
    local output
    output=$(./target/debug/agentgateway version 2>&1 || true)
    
    if echo "$output" | grep -q "unexpected argument 'version' found"; then
        log_success "agentgateway version subcommand correctly fails with expected error"
    else
        log_error "agentgateway version subcommand did not fail as expected. Output: $output"
        return 1
    fi
}

# Test 3: Test the fixed version check function from the install script
test_install_script_version_check() {
    run_test "Install script version check function works"
    
    # Source the install script functions
    source ./common/scripts/get-agentproxy
    
    # Set up test environment
    export AGENTGATEWAY_INSTALL_DIR="./target/debug"
    export BINARY_NAME="agentgateway"
    export TAG="v0.4.13-test"  # Set a test tag that won't match
    
    # Test the checkAgentGatewayInstalledVersion function
    # This should return 1 (needs update) since our test tag won't match the actual version
    if ! checkAgentGatewayInstalledVersion >/dev/null 2>&1; then
        log_success "checkAgentGatewayInstalledVersion function works correctly"
    else
        log_error "checkAgentGatewayInstalledVersion function failed"
        return 1
    fi
}

# Test 4: Simulate the old broken version check to ensure it would fail
test_old_version_check_would_fail() {
    run_test "Old version check method fails as expected"
    
    # Simulate the old broken version check
    export AGENTGATEWAY_INSTALL_DIR="./target/debug"
    export BINARY_NAME="agentgateway"
    
    # This should fail with the "unexpected argument" error
    local output
    output=$("${AGENTGATEWAY_INSTALL_DIR}/${BINARY_NAME}" version --template="{{ .Version }}" 2>&1 || true)
    
    if echo "$output" | grep -q "unexpected argument 'version' found"; then
        log_success "Old version check method correctly fails (confirming the bug existed)"
    else
        log_error "Old version check method did not fail as expected. Output: $output"
        return 1
    fi
}

# Test 5: Test that the new version check extracts version information
test_new_version_check_extracts_info() {
    run_test "New version check extracts version information and parses GitTag"
    
    export AGENTGATEWAY_INSTALL_DIR="./target/debug"
    export BINARY_NAME="agentgateway"
    
    # Test the new version check method
    local version_output
    version_output=$("${AGENTGATEWAY_INSTALL_DIR}/${BINARY_NAME}" --version 2>&1)
    
    if [ -n "$version_output" ] && echo "$version_output" | grep -q "version.BuildInfo"; then
        log_success "New version check successfully extracts version information"
        log_info "Version output: $version_output"
        
        # Test GitTag extraction
        local git_tag
        git_tag=$(echo "$version_output" | sed -n 's/.*GitTag:"\([^"]*\)".*/\1/p')
        
        if [ -n "$git_tag" ]; then
            log_success "Successfully extracted GitTag: $git_tag"
        else
            log_error "Failed to extract GitTag from version output"
            return 1
        fi
    else
        log_error "New version check failed to extract version information. Output: $version_output"
        return 1
    fi
}

# Test 6: Test parsing with the exact output format provided by maintainer
test_maintainer_provided_format() {
    run_test "Version parsing works with maintainer's example output"
    
    # This is the exact output format provided by howardjohn in the PR review
    local maintainer_example='agentgateway-app version.BuildInfo{RustVersion:"1.88.0", BuildProfile:"release", BuildStatus:"Modified", GitTag:"v0.6.0", Version:"2c7ba0d4ed47fcafa97fa411fdbf1a8ca40cf6a9-dirty", GitRevision:"2c7ba0d4ed47fcafa97fa411fdbf1a8ca40cf6a9-dirty"}'
    
    # Test our GitTag extraction with this exact format
    local extracted_version
    extracted_version=$(echo "$maintainer_example" | sed -n 's/.*GitTag:"\([^"]*\)".*/\1/p')
    
    if [ "$extracted_version" = "v0.6.0" ]; then
        log_success "Successfully extracted version 'v0.6.0' from maintainer's example"
        log_info "Extracted: $extracted_version"
    else
        log_error "Failed to extract correct version from maintainer's example. Got: '$extracted_version', expected: 'v0.6.0'"
        return 1
    fi
    
    # Test that our install script function would work with this format
    # Create a mock binary that outputs the maintainer's example
    local mock_binary_dir="./test_mock_binary"
    mkdir -p "$mock_binary_dir"
    
    cat > "$mock_binary_dir/agentgateway" << 'EOF'
#!/bin/bash
if [ "$1" = "--version" ]; then
    echo 'agentgateway-app version.BuildInfo{RustVersion:"1.88.0", BuildProfile:"release", BuildStatus:"Modified", GitTag:"v0.6.0", Version:"2c7ba0d4ed47fcafa97fa411fdbf1a8ca40cf6a9-dirty", GitRevision:"2c7ba0d4ed47fcafa97fa411fdbf1a8ca40cf6a9-dirty"}'
else
    exit 1
fi
EOF
    chmod +x "$mock_binary_dir/agentgateway"
    
    # Source the install script functions
    source ./common/scripts/get-agentproxy
    
    # Set up test environment with mock binary
    export AGENTGATEWAY_INSTALL_DIR="$mock_binary_dir"
    export BINARY_NAME="agentgateway"
    export TAG="v0.6.0"  # This should match the GitTag in the mock output
    
    # Test the checkAgentGatewayInstalledVersion function
    local output
    output=$(checkAgentGatewayInstalledVersion 2>&1)
    local result=$?
    
    # Clean up mock binary
    rm -rf "$mock_binary_dir"
    
    if [ $result -eq 0 ] && echo "$output" | grep -q "agentgateway v0.6.0 is already"; then
        log_success "Install script correctly handles maintainer's example format"
        log_info "Output: $output"
    else
        log_error "Install script failed with maintainer's example format. Output: $output, Exit code: $result"
        return 1
    fi
}

# Main test execution
main() {
    log_info "Starting tests for get-agentproxy install script"
    log_info "Testing the fix for GitHub issue #61: Install script shows version error"
    
    # Run all tests
    test_version_flag_works || true
    test_version_subcommand_fails || true
    test_install_script_version_check || true
    test_old_version_check_would_fail || true
    test_new_version_check_extracts_info || true
    test_maintainer_provided_format || true
    
    # Print summary
    echo -e "\n${YELLOW}=== Test Summary ===${NC}"
    echo "Tests run: $TESTS_RUN"
    echo -e "Tests passed: ${GREEN}$TESTS_PASSED${NC}"
    echo -e "Tests failed: ${RED}$TESTS_FAILED${NC}"
    
    if [ $TESTS_FAILED -eq 0 ]; then
        echo -e "\n${GREEN}All tests passed! The install script version check fix is working correctly.${NC}"
        exit 0
    else
        echo -e "\n${RED}Some tests failed. Please review the output above.${NC}"
        exit 1
    fi
}

# Run the tests
main "$@"
