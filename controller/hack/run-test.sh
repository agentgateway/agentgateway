#!/usr/bin/env bash

set -eEuo pipefail

# Script to run any test - both e2e and unit tests
#
# This script intelligently finds and runs test cases:
#   - E2E tests: Delegates to run-e2e-test.sh with full setup handling
#   - Unit tests: Runs go test directly on the appropriate package
#
# The script uses git grep to find tests and automatically determines:
#   - Whether it's an e2e test or a unit test
#   - Which package contains the test
#   - The most specific go test -run pattern
#
# Environment Variables (for e2e tests):
#   PERSIST_INSTALL - If set to true/1/yes/y, will skip 'make setup' if a kind
#                     cluster already exists (passed to run-e2e-test.sh)
#   AUTO_SETUP      - If set to true/1/yes/y, will automatically clean up conflicting
#                     Helm releases if detected (passed to run-e2e-test.sh)
#   CLUSTER_NAME    - Name of the kind cluster (default: kind)
#
# Usage: ./hack/run-test.sh [OPTIONS] [TEST_PATTERN]
#
# Options:
#   --list, -l          List all available tests (both e2e and unit)
#   --unit, -u          Force treating as unit test (skip e2e detection)
#   --e2e, -e           Force treating as e2e test
#   --package PKG       Run tests in specific package
#   --verbose, -v       Enable verbose output
#   --rebuild, -r       Delete the kind cluster, rebuild all docker images, create a
#                       new kind cluster, load images into kind, and then run tests.
#                       (Only applies to e2e tests)
#   --dry-run, -n       Print the test command that would be run without executing it
#
# Examples:
#   # Run an e2e test suite
#   ./hack/run-test.sh SessionPersistence
#
#   # Run a unit test
#   ./hack/run-test.sh TestContainMapElements
#
#   # Run a test by name (auto-detect e2e vs unit)
#   ./hack/run-test.sh TestGatewayParameters
#
#   # Run all tests in a package
#   ./hack/run-test.sh --package ./pkg/utils/helmutils
#
#   # Force unit test mode
#   ./hack/run-test.sh --unit TestSomething
#
#   # Auto-cleanup conflicting Helm releases (e2e only)
#   AUTO_SETUP=true ./hack/run-test.sh SessionPersistence
#
#   # Delete cluster and rebuild everything from scratch (e2e only)
#   ./hack/run-test.sh --rebuild SessionPersistence
#
#   # Print test command without running (works for both e2e and unit tests)
#   ./hack/run-test.sh -n TestSomething
#
#   # List all available tests
#   ./hack/run-test.sh --list

# Get the script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

log_info() {
    echo -e "INFO: $*"
}

log_success() {
    echo -e "SUCCESS: $*"
}

log_warn() {
    echo -e "WARNING: $*" >&2
}

log_error() {
    echo -e "ERROR: $*" >&2
}

log_section() {
    echo -e "=== $* ==="
}

# Parse arguments
FORCE_MODE=""
PACKAGE=""
VERBOSE=false
REBUILD=false
DRY_RUN=false

parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            --list|-l)
                list_all_tests
                exit 0
                ;;
            --unit|-u)
                FORCE_MODE="unit"
                shift
                ;;
            --e2e|-e)
                FORCE_MODE="e2e"
                shift
                ;;
            --package)
                PACKAGE="$2"
                shift 2
                ;;
            --verbose|-v)
                VERBOSE=true
                shift
                ;;
            --rebuild|-r)
                REBUILD=true
                shift
                ;;
            --dry-run|-n)
                DRY_RUN=true
                shift
                ;;
            --help|-h)
                show_help
                exit 0
                ;;
            -*)
                log_error "Unknown option: $1"
                show_help
                exit 1
                ;;
            *)
                # This is the test pattern
                TEST_PATTERN="$1"
                shift
                ;;
        esac
    done
}

show_help() {
    cat << 'EOF'
Usage: ./hack/run-test.sh [OPTIONS] [TEST_PATTERN]

Options:
  --list, -l          List all available tests (both e2e and unit)
  --unit, -u          Force treating as unit test (skip e2e detection)
  --e2e, -e           Force treating as e2e test
  --package PKG       Run tests in specific package
  --verbose, -v       Enable verbose output
  --rebuild, -r       Delete the kind cluster, rebuild all docker images, create a
                      new kind cluster, load images into kind, and then run tests.
                      (Only applies to e2e tests)
  --dry-run, -n       Print the test command that would be run without executing it
  --help, -h          Show this help message

Examples:
  # Run an e2e test suite
  ./hack/run-test.sh SessionPersistence

  # Run a unit test
  ./hack/run-test.sh TestContainMapElements

  # Run all tests in a package
  ./hack/run-test.sh --package ./pkg/utils/helmutils

  # Force unit test mode
  ./hack/run-test.sh --unit TestSomething

  # Auto-cleanup conflicting Helm releases (e2e only)
  AUTO_SETUP=true ./hack/run-test.sh SessionPersistence

  # Delete cluster and rebuild everything from scratch (e2e only)
  ./hack/run-test.sh --rebuild SessionPersistence

  # Print test command without running (works for both e2e and unit tests)
  ./hack/run-test.sh -n TestSomething

  # List all available tests
  ./hack/run-test.sh --list
EOF
}

# List all tests
list_all_tests() {
    log_section "E2E Tests"
    "${SCRIPT_DIR}/run-e2e-test.sh" --list
    echo ""

    log_section "Unit Tests (sample)"
    log_info "Unit test functions:"
    git grep -h '^func Test.*\(t \*testing\.T\)' -- '**/*_test.go' | \
        grep -v 'test/e2e/' | \
        sed -n 's/func \(Test[^(]*\).*/  - \1/p' | sort -u | head -30
    echo "  ... (showing first 30, use git grep to find more)"
    echo ""

    log_info "Ginkgo Describe blocks:"
    git grep -h 'Describe("' -- '**/*_test.go' | \
        grep -v 'test/e2e/tests/' | \
        grep -v 'vendor/' | \
        sed -n 's/.*Describe("\([^"]*\)".*/  - \1/p' | sort -u | head -30
    echo "  ... (showing first 30)"
}

# Check if a pattern is an e2e test by finding where it's defined
is_e2e_test() {
    local pattern="$1"

    # Search for the test pattern across all test files and check if path contains /e2e/
    local test_location

    # Try exact suite name match first
    test_location=$(git grep -l "Register(\"${pattern}\"" 2>/dev/null | head -1)
    [[ -n "$test_location" && "$test_location" == *"/e2e/"* ]] && return 0

    # Try test function match (handles both TestXxx and just Xxx patterns)
    if [[ "$pattern" == Test* ]]; then
        test_location=$(git grep -l "func ${pattern}" 2>/dev/null | grep '_test\.go$' | head -1)
    else
        test_location=$(git grep -l "func Test${pattern}" 2>/dev/null | grep '_test\.go$' | head -1)
    fi
    [[ -n "$test_location" && "$test_location" == *"/e2e/"* ]] && return 0

    # Try partial match for suite names
    test_location=$(git grep -l "Register(\".*${pattern}.*\"" 2>/dev/null | head -1)
    [[ -n "$test_location" && "$test_location" == *"/e2e/"* ]] && return 0

    # Try test method in suite (e.g., func (s *suite) TestMethodName)
    if [[ "$pattern" == Test* ]]; then
        test_location=$(git grep -l "func (.*) ${pattern}" 2>/dev/null | head -1)
    else
        test_location=$(git grep -l "func (.*) Test${pattern}" 2>/dev/null | head -1)
    fi
    [[ -n "$test_location" && "$test_location" == *"/e2e/"* ]] && return 0

    # Try as a key in testCases map (e.g., "TestMethodName": {)
    test_location=$(git grep -l "\"${pattern}\":" 2>/dev/null | grep -E '(suite|types)\.go$' | head -1)
    [[ -n "$test_location" && "$test_location" == *"/e2e/"* ]] && return 0

    return 1
}

# Find unit test and return package path
find_unit_test() {
    local pattern="$1"

    if [[ -n "${PACKAGE:-}" ]]; then
        log_info "Using specified package: ${PACKAGE}" >&2
        echo "$PACKAGE"
        return 0
    fi

    log_info "Searching for unit test: ${pattern}" >&2

    # Search for test function, excluding e2e tests
    local test_file
    # Check if pattern already starts with Test
    local search_pattern
    if [[ "$pattern" == Test* ]]; then
        search_pattern="func ${pattern}"
    else
        search_pattern="func Test${pattern}"
    fi

    test_file=$(git grep -l "${search_pattern}" 2>/dev/null | \
        grep '_test\.go$' | \
        grep -v 'test/e2e/tests/' | \
        grep -v 'vendor/' | \
        head -1)

    if [[ -z "$test_file" ]]; then
        # Try Ginkgo Describe blocks
        test_file=$(git grep -l "Describe(\".*${pattern}.*\"" 2>/dev/null | \
            grep '_test\.go$' | \
            grep -v 'test/e2e/tests/' | \
            grep -v 'vendor/' | \
            head -1)
    fi

    if [[ -z "$test_file" ]]; then
        # Try partial match on function name (with or without Test prefix)
        if [[ "$pattern" == Test* ]]; then
            search_pattern="func .*${pattern}"
        else
            search_pattern="func Test.*${pattern}"
        fi
        test_file=$(git grep -l "${search_pattern}" 2>/dev/null | \
            grep '_test\.go$' | \
            grep -v 'test/e2e/tests/' | \
            grep -v 'vendor/' | \
            head -1)
    fi

    if [[ -z "$test_file" ]]; then
        log_error "No unit test found matching: ${pattern}"
        return 1
    fi

    log_info "Found test in: ${test_file}" >&2

    # Get the package directory
    local pkg_dir
    pkg_dir=$(dirname "$test_file")

    echo "./${pkg_dir}"
}

# Run unit test
run_unit_test() {
    local pattern="$1"
    local pkg_path="$2"

    log_section "Running Unit Test"
    log_info "Pattern: ${pattern}"
    log_info "Package: ${pkg_path}"
    echo ""

    # Set default version for tests
    local test_version="${VERSION:-v1.0.0-ci1}"
    local ldflags="-X github.com/kgateway-dev/kgateway/v2/pkg/version.Version=${test_version}"

    # If dry-run mode, just print the command and exit
    if [[ "$DRY_RUN" == "true" ]]; then
        echo ""
        log_info "Dry-run mode: printing command without executing"
        echo ""
        if [[ -n "$pattern" ]]; then
            echo "go test -v -ldflags=\"${ldflags}\" -run ${pattern} ${pkg_path}"
        else
            echo "go test -v -ldflags=\"${ldflags}\" ${pkg_path}"
        fi
        exit 0
    fi

    if [[ "$VERBOSE" == "true" ]]; then
        set -x
    fi

    if [[ -n "$pattern" ]]; then
        go test -v -ldflags="${ldflags}" -run "${pattern}" "${pkg_path}"
    else
        go test -v -ldflags="${ldflags}" "${pkg_path}"
    fi
    local result=$?

    if [[ "$VERBOSE" == "true" ]]; then
        set +x
    fi

    if [[ $result -eq 0 ]]; then
        echo ""
        log_success "Test passed!"
    else
        echo ""
        log_error "Test failed with exit code: ${result}"
        exit $result
    fi
}

# Run e2e test
run_e2e_test() {
    local pattern="$1"

    log_section "Running E2E Test (delegating to run-e2e-test.sh)"
    echo ""

    local e2e_args=()
    if [[ "$REBUILD" == "true" ]]; then
        e2e_args+=("--rebuild")
    fi
    if [[ "$DRY_RUN" == "true" ]]; then
        e2e_args+=("--dry-run")
    fi
    e2e_args+=("$pattern")

    exec "${SCRIPT_DIR}/run-e2e-test.sh" "${e2e_args[@]}"
}

# Main script
main() {
    local TEST_PATTERN=""

    parse_args "$@"

    # If package is specified but no pattern, run all tests in package
    if [[ -n "$PACKAGE" && -z "$TEST_PATTERN" ]]; then
        log_info "Running all tests in package: ${PACKAGE}"
        run_unit_test "" "$PACKAGE"
        exit 0
    fi

    if [[ -z "$TEST_PATTERN" ]]; then
        log_error "No test pattern specified"
        echo ""
        show_help
        exit 1
    fi

    # Determine test type
    local is_e2e=false

    if [[ "$FORCE_MODE" == "e2e" ]]; then
        is_e2e=true
    elif [[ "$FORCE_MODE" == "unit" ]]; then
        is_e2e=false
    else
        # Auto-detect
        if is_e2e_test "$TEST_PATTERN"; then
            is_e2e=true
        fi
    fi

    if [[ "$is_e2e" == "true" ]]; then
        log_info "Detected E2E test"
        run_e2e_test "$TEST_PATTERN"
    else
        log_info "Detected unit test"

        # Warn if --rebuild was specified for a unit test
        if [[ "$REBUILD" == "true" ]]; then
            log_warn "--rebuild flag only applies to e2e tests, ignoring it for unit tests"
        fi

        local pkg_path
        pkg_path=$(find_unit_test "$TEST_PATTERN")
        if [[ $? -ne 0 ]]; then
            exit 1
        fi
        run_unit_test "$TEST_PATTERN" "$pkg_path"
    fi
}

main "$@"
