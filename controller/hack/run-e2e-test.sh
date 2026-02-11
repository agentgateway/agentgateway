#!/usr/bin/env bash

set -eEuo pipefail

# Script to run a single e2e test case
#
# This script intelligently finds and runs e2e test cases using git grep.
# It can run:
#   - Top-level test functions (e.g., TestKgateway)
#   - Entire test suites (e.g., SessionPersistence)
#   - Individual test methods within suites (e.g., TestCookieSessionPersistence)
#
# The script will automatically generate the most specific go test -run pattern.
#
# Environment Variables:
#   PERSIST_INSTALL       - If set to true/1/yes/y, will skip 'make setup' if a kind
#                           cluster already exists. This speeds up test runs when you're
#                           iterating locally.
#   FAIL_FAST_AND_PERSIST - If set to true (default for this script), skip cleanup on
#                           test failure to allow resource inspection. Set to false to
#                           always cleanup. Use --cleanup-on-failure flag to override.
#   SKIP_ALL_TEARDOWN     - If set to true/1/yes/y, skip all cleanup/teardown operations
#                           regardless of test success or failure. Use --skip-teardown flag.
#   AUTO_SETUP            - If set to true/1/yes/y, will automatically clean up conflicting
#                           Helm releases if detected. Otherwise, will error out.
#   CLUSTER_NAME          - Name of the kind cluster (default: kind)
#   TEST_PKG              - Go test package to run (default: ./test/e2e/tests)
#
# Usage: ./hack/run-e2e-test.sh [OPTIONS] [TEST_PATTERN]
#
# Options:
#   --rebuild, -r             Delete the kind cluster, rebuild all docker images, create a
#                             new kind cluster, load images into kind, and then run tests.
#                             This ensures a completely fresh environment.
#   --persist, -p             Skip 'make setup' if kind cluster exists (faster iteration).
#                             Equivalent to setting PERSIST_INSTALL=true.
#   --cleanup-on-failure, -c  Always cleanup resources even if test fails (disables default
#                             FAIL_FAST_AND_PERSIST behavior). Useful for CI or running all tests.
#   --skip-teardown, -s       Skip all cleanup/teardown operations regardless of test result.
#                             Equivalent to setting SKIP_ALL_TEARDOWN=true.
#   --list, -l                List all available test suites and top-level tests
#   --dry-run, -n             Print the test command that would be run without executing it
#   --help, -h                Show this help message
#
# Examples:
#   # Run an entire test suite (default: skip cleanup on failure for debugging)
#   ./hack/run-e2e-test.sh SessionPersistence
#
#   # Run a specific test method within a suite
#   ./hack/run-e2e-test.sh TestCookieSessionPersistence
#
#   # Run a top-level test function
#   ./hack/run-e2e-test.sh TestKgateway
#
#   # Always cleanup/tear-down, even on failure
#   ./hack/run-e2e-test.sh --cleanup-on-failure SessionPersistence
#
#   # Run a suite within a specific parent test (using slash notation)
#   ./hack/run-e2e-test.sh TestKgateway/SessionPersistence
#
#   # Run a specific test method using slash notation
#   ./hack/run-e2e-test.sh TestKgateway/^SessionPersistence$/TestHeaderSessionPersistence
#
#   # Skip setup if cluster exists (faster iteration) - using flag
#   ./hack/run-e2e-test.sh --persist SessionPersistence
#
#   # Skip setup if cluster exists (faster iteration) - using env var
#   PERSIST_INSTALL=true ./hack/run-e2e-test.sh SessionPersistence
#
#   # Auto-cleanup conflicting Helm releases
#   AUTO_SETUP=true ./hack/run-e2e-test.sh SessionPersistence
#
#   # Delete cluster and rebuild everything from scratch
#   ./hack/run-e2e-test.sh --rebuild SessionPersistence
#
#   # Use a different cluster name
#   CLUSTER_NAME=my-cluster ./hack/run-e2e-test.sh SessionPersistence
#
#   # Print the test command without running it
#   ./hack/run-e2e-test.sh -n TestCookieSessionPersistence

# Get the script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

# Check if KIND is available
KIND="${KIND:-kind}"
CLUSTER_NAME="${CLUSTER_NAME:-kind}"

log_info() {
    echo -e "INFO: $*" >&2
}

log_success() {
    echo -e "SUCCESS: $*" >&2
}

log_warn() {
    echo -e "WARNING: $*" >&2
}

log_error() {
    echo -e "ERROR: $*" >&2
}

# Check if an environment variable is truthy (1, true, yes, y)
is_truthy() {
    local val="${!1:-}"
    [[ "${val,,}" =~ ^(1|true|yes|y)$ ]]
}

# Check if kind cluster exists
kind_cluster_exists() {
    ${KIND} get clusters 2>/dev/null | grep -q "^${CLUSTER_NAME}$"
}

# Check for and handle conflicting Helm releases
# Returns: 0 if no cleanup needed, 1 if cleanup was performed
check_and_cleanup_helm_conflicts() {
    # Only check if kubectl is available and cluster exists
    if ! command -v kubectl &> /dev/null; then
        return 0
    fi

    if ! kubectl cluster-info &> /dev/null; then
        return 0
    fi

    log_info "Checking for existing kgateway installations..."

    # When AUTO_SETUP is enabled, clean up everything for a fresh start
    if is_truthy AUTO_SETUP; then
        log_info "AUTO_SETUP is enabled, cleaning up all kgateway installations..."

        # Uninstall Helm releases from common namespaces
        for ns in kgateway-system kgateway-test default; do
            for release in kgateway-crds kgateway; do
                if helm list -a -n "$ns" 2>/dev/null | grep -q "^${release}"; then
                    log_info "Uninstalling Helm release '${release}' from namespace '${ns}'..."
                    helm uninstall "${release}" -n "${ns}" 2>/dev/null || true
                fi
            done
        done

        # Delete ALL kgateway CRDs
        log_info "Deleting all kgateway CRDs..."
        local crds_list
        crds_list=$(kubectl get crds 2>/dev/null | grep 'kgateway.dev' | awk '{print $1}' || true)
        if [[ -n "$crds_list" ]]; then
            while IFS= read -r crd_name; do
                if [[ -n "$crd_name" ]]; then
                    log_info "Deleting CRD: ${crd_name}"
                    kubectl delete crd "${crd_name}" --ignore-not-found=true 2>/dev/null || true
                fi
            done <<< "$crds_list"
        fi

        # Wait a moment for deletions to complete
        sleep "${SLEEP_FOR_DELETE:-2}"

        log_info "Cleanup complete, will run 'make setup' to reinstall"
        echo ""
        return 1  # Signal that cleanup happened and setup is needed
    fi

    # If AUTO_SETUP is not enabled, just check and return
    log_info "No cleanup performed (set AUTO_SETUP=true to enable automatic cleanup)"
    return 0
}

# Build the go test run pattern
build_test_pattern() {
    local pattern="$1"

    # If pattern contains regex metacharacters, treat it as a regex
    if [[ "$pattern" =~ [\.\*\+\?\[\]\(\)\|] ]]; then
        log_info "Detected regex pattern: ${pattern}"
        # If it looks like a test method pattern (starts with Test), search across all suites
        if [[ "$pattern" == Test* ]]; then
            log_info "Searching across all suites for matching test methods"
            echo ".*/.*/${pattern}"
        else
            # Otherwise pass through as-is
            echo "${pattern}"
        fi
        return
    fi

    # Check if the pattern contains slashes (nested test path like TestKgateway/SessionPersistence/TestHeaderSessionPersistence)
    if [[ "$pattern" == */* ]]; then
        log_info "Detected nested test path: ${pattern}"

        # Split by '/', wrap each element with '^' and '$', then join with '/'
        IFS='/' read -ra parts <<< "$pattern"
        local result=""
        for ((i=0; i<${#parts[@]}; i++)); do
            local part="${parts[i]}"
            # Strip existing anchors
            part="${part#^}"
            part="${part%\$}"
            # Add to result
            if [[ $i -eq 0 ]]; then
                result="^${part}\$"
            else
                result="${result}/^${part}\$"
            fi
        done

        log_info "Running nested test: ${pattern}"
        echo "$result"
        return
    fi

    # Check if it's a top-level test function
    local test_func
    if [[ "$pattern" == Test* ]]; then
        # Pattern already starts with Test, search for exact match
        test_func=$(git grep -h --no-line-number "^func ${pattern}(" -- 'test/e2e/tests/*_test.go' 2>/dev/null | head -1 || true)
    else
        # Pattern doesn't start with Test, add it
        test_func=$(git grep -h --no-line-number "^func Test${pattern}(" -- 'test/e2e/tests/*_test.go' 2>/dev/null | head -1 || true)
    fi

    if [[ -n "$test_func" ]]; then
        # Extract the test function name
        local func_name
        func_name=$(echo "$test_func" | sed -E 's/func (Test[^(]*).*/\1/')
        log_info "Running top-level test: ${func_name}"
        echo "^${func_name}$"
        return
    fi

    # Check if it's a test method within a suite (e.g., TestCookieSessionPersistence)
    if [[ "$pattern" == Test* ]]; then
        # Search for method with exact or partial match (find ALL matches, not just first)
        local method_matches
        method_matches=$(git grep "func (.*) ${pattern}[^(]*\(\)" -- 'test/e2e/features' 2>/dev/null || true)

        if [[ -n "$method_matches" ]]; then
            # Extract the actual full method name from first match
            local method_name
            method_name=$(echo "$method_matches" | head -1 | sed -E 's/^[^:]*:[0-9]*:func .* (Test[^(]*)\(\).*/\1/')

            # Find ALL files containing this method
            local method_files
            method_files=$(git grep -l "func (.*) ${method_name}\(\)" -- 'test/e2e/features' 2>/dev/null || true)

            if [[ -n "$method_files" ]]; then
                # Build patterns for all matching suites
                local patterns=()
                local suite_info=()

                while IFS= read -r method_file; do
                    # Get the package import path relative to the features directory
                    local rel_path
                    rel_path=$(echo "$method_file" | sed 's|test/e2e/features/||' | xargs dirname)

                    # Find the suite registration that imports from this path
                    local suite_line
                    suite_line=$(git grep "Register(\"[^\"]*\", .*\\.NewTestingSuite)" -- 'test/e2e/tests/*.go' 2>/dev/null | \
                        grep -v "^\s*//" | \
                        while IFS=: read -r file line_content; do
                            # Extract the package identifier from the Register call
                            pkg_id=$(echo "$line_content" | sed -E 's/.*Register\("[^"]*", ([^.]*)\..*/\1/')
                            # Check if this package identifier's import path matches our method's path
                            # Look for either:
                            # 1. Aliased import: pkg_id "path/to/features/rel_path"
                            # 2. Non-aliased import: "path/to/features/rel_path" (where pkg_id matches the last component)
                            # IMPORTANT: Match exactly /features/rel_path" to avoid matching /features/foo/rel_path"
                            if git grep -q "[[:space:]]${pkg_id}[[:space:]]\".*\/features\/${rel_path}\"" "$file" 2>/dev/null; then
                                echo "${file}:${line_content}"
                                break
                            elif git grep -q "\".*\/features\/${rel_path}\"" "$file" 2>/dev/null; then
                                # For non-aliased imports, verify pkg_id matches the last path component
                                local import_line
                                import_line=$(git grep "\".*\/features\/${rel_path}\"" "$file" 2>/dev/null | head -1)
                                # Extract the last component of the import path
                                local last_component
                                last_component=$(echo "$import_line" | sed -E 's|.*/([^/"]+)".*|\1|')
                                if [[ "$last_component" == "$pkg_id" ]]; then
                                    echo "${file}:${line_content}"
                                    break
                                fi
                            fi
                        done || true)

                    if [[ -n "$suite_line" ]]; then
                        local suite_name
                        suite_name=$(echo "$suite_line" | sed -E 's/.*Register\("([^"]*)".*/\1/')

                        local file
                        file=$(echo "$suite_line" | cut -d: -f1)

                        local parent_test
                        parent_test=$(git grep "func Test.*\(t \*testing\.T\)" "$file" | cut -d: -f3- | head -1 | sed -E 's/func (Test[^(]*).*/\1/')

                        if [[ -z "$parent_test" ]]; then
                            local dir=$(dirname "$file")
                            local base=$(basename "$file" .go)
                            if [[ "$base" == *_tests ]]; then
                                local alt_file="${dir}/${base%s}.go"
                                if [[ -f "$alt_file" ]]; then
                                    parent_test=$(git grep "func Test.*\(t \*testing\.T\)" "$alt_file" | cut -d: -f3- | head -1 | sed -E 's/func (Test[^(]*).*/\1/')
                                fi
                            fi
                        fi

                        if [[ -n "$parent_test" ]]; then
                            patterns+=("^${parent_test}$/^${suite_name}$/^${method_name}$")
                            suite_info+=("${parent_test} -> ${suite_name} -> ${method_name}")
                        fi
                    fi
                done <<< "$method_files"

                # If we found patterns, combine them and return
                if [[ ${#patterns[@]} -gt 0 ]]; then
                    if [[ ${#patterns[@]} -eq 1 ]]; then
                        log_info "Running suite test: ${suite_info[0]}"
                        echo "${patterns[0]}"
                    else
                        log_info "Running test in ${#patterns[@]} suites:"
                        for info in "${suite_info[@]}"; do
                            log_info "  - ${info}"
                        done
                        # For multiple patterns, we need to combine them intelligently
                        # If they share the same suite/method but different parent tests,
                        # we can use alternation at the parent level
                        # Otherwise, we need to run them separately or use complex regex

                        # Extract parent tests, suite names, and method names
                        local parent_tests=()
                        local suite_names=()
                        local method_names=()
                        for pattern in "${patterns[@]}"; do
                            # Pattern format: ^ParentTest$/^Suite$/^Method$
                            local parts
                            IFS='/' read -ra parts <<< "$pattern"
                            parent_tests+=("${parts[0]#^}")
                            parent_tests[-1]="${parent_tests[-1]%\$}"
                            suite_names+=("${parts[1]#^}")
                            suite_names[-1]="${suite_names[-1]%\$}"
                            method_names+=("${parts[2]#^}")
                            method_names[-1]="${method_names[-1]%\$}"
                        done

                        # Check if suite and method are the same across all patterns
                        local first_suite="${suite_names[0]}"
                        local first_method="${method_names[0]}"
                        local same_suite_method=true
                        for ((i=1; i<${#suite_names[@]}; i++)); do
                            if [[ "${suite_names[i]}" != "$first_suite" ]] || [[ "${method_names[i]}" != "$first_method" ]]; then
                                same_suite_method=false
                                break
                            fi
                        done

                        if [[ "$same_suite_method" == true ]]; then
                            # Same suite/method, different parents - use alternation at parent level
                            local parent_alternation
                            parent_alternation=$(IFS='|'; echo "${parent_tests[*]}")
                            echo "^(${parent_alternation})$/^${first_suite}$/^${first_method}$"
                        else
                            # Different suites or methods - combine with | at top level
                            local combined_pattern
                            combined_pattern=$(IFS='|'; echo "${patterns[*]}")
                            echo "(${combined_pattern})"
                        fi
                    fi
                    return
                fi
            fi
        fi
    fi

    # Check if it's a suite
    local suite_line
    suite_line=$(git grep "Register(\"${pattern}\"" -- 'test/e2e/tests/*.go' 2>/dev/null | head -1 || true)

    if [[ -z "$suite_line" ]]; then
        # Try partial match
        suite_line=$(git grep "Register(\".*${pattern}.*\"" -- 'test/e2e/tests/*.go' 2>/dev/null | head -1 || true)
    fi

    if [[ -n "$suite_line" ]]; then
        # Extract the suite name from Register("SuiteName", ...)
        local suite_name
        suite_name=$(echo "$suite_line" | sed -E 's/.*Register\("([^"]*)".*/\1/')

        # Find the parent test function
        local file
        file=$(echo "$suite_line" | cut -d: -f1)

        # Find which test function calls this suite runner
        local parent_test
        parent_test=$(git grep "func Test.*\(t \*testing\.T\)" "$file" | cut -d: -f3- | head -1 | sed -E 's/func (Test[^(]*).*/\1/')

        # If not found in the same file, try related files (e.g., kgateway_test.go for kgateway_tests.go)
        if [[ -z "$parent_test" ]]; then
            local dir=$(dirname "$file")
            local base=$(basename "$file" .go)
            # Try removing 's' from 'tests' suffix (kgateway_tests.go -> kgateway_test.go)
            if [[ "$base" == *_tests ]]; then
                local alt_file="${dir}/${base%s}.go"
                if [[ -f "$alt_file" ]]; then
                    parent_test=$(git grep "func Test.*\(t \*testing\.T\)" "$alt_file" | cut -d: -f3- | head -1 | sed -E 's/func (Test[^(]*).*/\1/')
                fi
            fi
        fi

        if [[ -z "$parent_test" ]]; then
            log_error "Could not find parent test function for suite: ${suite_name}"
            exit 1
        fi

        # Check if the pattern might be a specific test method (starts with Test but not the suite name)
        if [[ "$pattern" != "$suite_name" && "$pattern" == Test* ]]; then
            # The pattern looks like a test method name
            local method_match
            method_match=$(git grep -h "func (.*) ${pattern}\(\)" -- 'test/e2e/features' 2>/dev/null | head -1 || true)

            if [[ -n "$method_match" ]]; then
                # Running a specific test method within a suite
                local method_name
                method_name=$(echo "$method_match" | sed -E 's/func .* ([^(]*)\(\).*/\1/')
                log_info "Running suite test: ${parent_test} -> ${suite_name} -> ${method_name}"
                echo "^${parent_test}$/^${suite_name}$/^${method_name}$"
                return
            fi
        fi

        # Running entire suite
        log_info "Running entire suite: ${parent_test} -> ${suite_name}"
        echo "^${parent_test}$/^${suite_name}$"
        return
    fi

    # Fallback: just use the pattern as-is
    log_error "Test pattern not found: ${pattern}"
    exit 1
}

# List available tests
list_tests() {
    log_info "Available test suites:"
    git grep -h 'Register("' -- 'test/e2e/tests/*.go' | \
        sed -E 's/.*Register\("([^"]*)".*/  - \1/' | sort
    echo ""
    log_info "Available top-level tests:"
    git grep -h '^func Test.*\(t \*testing\.T\)' -- 'test/e2e/tests/*_test.go' | \
        sed -E 's/func (Test[^(]*).*/  - \1/' | sort
}

# Show help message
show_help() {
    cat << EOF
Usage: $0 [OPTIONS] [TEST_PATTERN]

Script to run a single e2e test case

This script intelligently finds and runs e2e test cases using git grep.
It can run:
  - Top-level test functions (e.g., TestKgateway)
  - Entire test suites (e.g., SessionPersistence)
  - Individual test methods within suites (e.g., TestCookieSessionPersistence)

Options:
  --help, -h                Show this help message
  --list, -l                List all available test suites and top-level tests
  --rebuild, -r             Delete the kind cluster, rebuild images, and create a fresh cluster
  --persist, -p             Skip 'make setup' if kind cluster exists (faster iteration)
  --cleanup-on-failure, -c  Always cleanup resources even if test fails (default: skip cleanup on failure)
  --skip-teardown, -s       Skip all cleanup/teardown operations regardless of test result
  --dry-run, -n             Print the test command that would be run without executing it

Environment Variables:
  PERSIST_INSTALL           If set to true/1/yes/y, skip 'make setup' if kind cluster exists
  FAIL_FAST_AND_PERSIST     If set to true (default), skip cleanup on test failure
  SKIP_ALL_TEARDOWN         If set to true/1/yes/y, skip all cleanup/teardown operations
  AUTO_SETUP                If set to true/1/yes/y, automatically cleanup conflicting Helm releases
  CLUSTER_NAME              Name of the kind cluster (default: kind)
  TEST_PKG                  Go test package to run (default: ./test/e2e/tests)

Examples:
  # Run an entire test suite (default: skip cleanup on failure for debugging)
  $0 SessionPersistence

  # Run a specific test method within a suite
  $0 TestCookieSessionPersistence

  # Run a top-level test function
  $0 TestKgateway

  # Always cleanup/tear-down, even on failure
  $0 --cleanup-on-failure SessionPersistence

  # Skip all teardown/cleanup regardless of test result
  $0 --skip-teardown SessionPersistence

  # Run a suite within a specific parent test (using slash notation)
  $0 TestKgateway/SessionPersistence

  # Run a specific test method using slash notation
  $0 TestKgateway/^SessionPersistence$/TestHeaderSessionPersistence

  # Skip setup if cluster exists (faster iteration) - using flag
  $0 --persist SessionPersistence

  # Skip setup if cluster exists (faster iteration) - using env var
  PERSIST_INSTALL=true $0 SessionPersistence

  # Auto-cleanup conflicting Helm releases
  AUTO_SETUP=true $0 SessionPersistence

  # Delete cluster and rebuild everything from scratch
  $0 --rebuild SessionPersistence

  # Use a different cluster name
  CLUSTER_NAME=my-cluster $0 SessionPersistence

  # Print the test command without running it
  $0 -n TestCookieSessionPersistence
EOF
}

# Main script
main() {
    local rebuild_cluster=false
    local dry_run=false
    local persist_install=false
    local cleanup_on_failure=false
    local skip_teardown=false
    local test_pattern=""

    # Parse arguments
    while [[ "$#" -gt 0 ]]; do
        case "$1" in
            --help|-h)
                show_help
                exit 0
                ;;
            --list|-l)
                list_tests
                exit 0
                ;;
            --rebuild|-r)
                rebuild_cluster=true
                shift
                ;;
            --dry-run|-n)
                dry_run=true
                shift
                ;;
            --persist|-p)
                persist_install=true
                shift
                ;;
            --cleanup-on-failure|-c)
                cleanup_on_failure=true
                shift
                ;;
            --skip-teardown|-s)
                skip_teardown=true
                shift
                ;;
            *)
                test_pattern="$1"
                shift
                ;;
        esac
    done

    # Set PERSIST_INSTALL environment variable if --persist flag was used
    if [[ "$persist_install" == "true" ]]; then
        export PERSIST_INSTALL=true
    fi

    # Set SKIP_ALL_TEARDOWN environment variable if --skip-teardown flag was used
    if [[ "$skip_teardown" == "true" ]]; then
        export SKIP_ALL_TEARDOWN=true
        log_info "All teardown/cleanup is DISABLED (will skip cleanup regardless of test result)"
    elif is_truthy SKIP_ALL_TEARDOWN; then
        log_info "All teardown/cleanup is DISABLED (SKIP_ALL_TEARDOWN env var is set)"
    fi

    # Set FAIL_FAST_AND_PERSIST to true by default (for local development/debugging)
    # unless --cleanup-on-failure flag was used or it's already set
    if [[ "$cleanup_on_failure" == "true" ]]; then
        export FAIL_FAST_AND_PERSIST=false
        log_info "Cleanup on failure is ENABLED (will cleanup even if tests fail)"
    elif [[ -z "${FAIL_FAST_AND_PERSIST:-}" ]]; then
        export FAIL_FAST_AND_PERSIST=true
        log_info "Cleanup on failure is DISABLED by default (will skip cleanup if tests fail)"
        log_info "Use --cleanup-on-failure to always cleanup, even on test failure"
    fi

    if [[ -z "$test_pattern" ]]; then
        log_error "Usage: $0 [OPTIONS] TEST_PATTERN"
        echo ""
        echo "Examples:"
        echo "  $0 SessionPersistence"
        echo "  $0 TestCookieSessionPersistence"
        echo "  $0 TestKgateway"
        echo "  $0 TestKgateway/SessionPersistence"
        echo "  $0 TestKgateway/SessionPersistence/TestHeaderSessionPersistence"
        echo "  $0 --persist SessionPersistence"
        echo "  $0 --cleanup-on-failure SessionPersistence"
        echo "  $0 --rebuild SessionPersistence"
        echo "  $0 -n TestCookieSessionPersistence"
        echo ""
        echo "Options:"
        echo "  --help, -h                Show full help message"
        echo "  --list, -l                List all available test suites and top-level tests"
        echo "  --rebuild, -r             Delete the kind cluster, rebuild images, and create a fresh cluster"
        echo "  --persist, -p             Skip 'make setup' if kind cluster exists (faster iteration)"
        echo "  --cleanup-on-failure, -c  Always cleanup resources even if test fails (default: skip cleanup on failure)"
        echo "  --skip-teardown, -s       Skip all cleanup/teardown operations regardless of test result"
        echo "  --dry-run, -n             Print the test command that would be run without executing it"
        exit 1
    fi

    # Build the test run pattern
    local run_pattern
    run_pattern=$(build_test_pattern "$test_pattern")

    # Determine test package
    local test_pkg="${TEST_PKG:-./test/e2e/tests}"

    # Skip setup if in dry-run mode
    if [[ "$dry_run" == "true" ]]; then
        log_info "Dry-run mode: skipping setup and cluster operations"
        echo ""
    # Handle cluster rebuild
    elif [[ "$rebuild_cluster" == "true" ]]; then
        log_info "Rebuild flag set: deleting and recreating kind cluster"
        if kind_cluster_exists; then
            log_info "Deleting existing kind cluster '${CLUSTER_NAME}'..."
            ${KIND} delete cluster --name "${CLUSTER_NAME}"
            echo ""
        else
            log_info "Kind cluster '${CLUSTER_NAME}' does not exist"
        fi

        log_info "Running 'make setup' to rebuild images and create fresh cluster..."
        make setup
        echo ""
    else
        # Check for conflicting Helm releases before proceeding
        # Returns 1 if cleanup was performed, 0 otherwise
        check_and_cleanup_helm_conflicts
        local cleanup_performed=$?

        # Check if we should run setup
        local should_setup=true

        if is_truthy PERSIST_INSTALL; then
            log_info "PERSIST_INSTALL is set"
            if kind_cluster_exists; then
                if [[ $cleanup_performed -eq 1 ]]; then
                    log_info "Kind cluster '${CLUSTER_NAME}' exists, but cleanup was performed, so running setup"
                    should_setup=true
                else
                    log_info "Kind cluster '${CLUSTER_NAME}' already exists, skipping setup"
                    should_setup=false
                fi
            else
                log_info "Kind cluster '${CLUSTER_NAME}' does not exist, will run setup"
            fi
        fi

        # Run setup if needed
        if [[ "$should_setup" == "true" ]]; then
            log_info "Running 'make setup'..."
            make setup
            echo ""
        else
            log_info "Packaging charts (required for tests)..."
            make package-kgateway-charts package-agentgateway-charts
            echo ""
        fi
    fi

    # Run the test
    log_info "Running test with pattern: ${run_pattern}"
    log_info "Test package: ${test_pkg}"
    echo ""

    # Export PERSIST_INSTALL so the test code can use it
    if is_truthy PERSIST_INSTALL; then
        export PERSIST_INSTALL
    fi

    # Escape $ for make/shell
    local escaped_pattern="${run_pattern//\$/\$\$}"

    # If dry-run mode, just print the command and exit
    if [[ "$dry_run" == "true" ]]; then
        echo ""
        log_info "Dry-run mode: printing command without executing"
        echo ""
        echo "make go-test \\"
        echo "    GO_TEST_USER_ARGS=\"-failfast -run '$escaped_pattern'\" \\"
        echo "    TEST_PKG=\"${test_pkg}\" TEST_TAG=e2e"
        echo ""
        log_info "Environment variables:"
        if is_truthy PERSIST_INSTALL; then
            echo "  PERSIST_INSTALL=true"
        fi
        if is_truthy SKIP_ALL_TEARDOWN; then
            echo "  SKIP_ALL_TEARDOWN=true (skip all teardown/cleanup)"
        fi
        if is_truthy FAIL_FAST_AND_PERSIST; then
            echo "  FAIL_FAST_AND_PERSIST=true (skip cleanup on failure)"
        else
            echo "  FAIL_FAST_AND_PERSIST=false (always cleanup)"
        fi
        echo ""
        log_success "Dry-run completed!"
        exit 0
    fi

    # Use a temp file to capture output for checking
    local test_output_file
    test_output_file=$(mktemp)
    trap "rm -f '$test_output_file'" EXIT

    # Temporarily disable exit-on-error so we can check the test results
    set +e
    set -x
    make go-test \
        "GO_TEST_USER_ARGS=-failfast -run '$escaped_pattern'" \
        TEST_PKG="${test_pkg}" TEST_TAG=e2e 2>&1 | tee "$test_output_file"
    test_exit_code=${PIPESTATUS[0]}
    set +x
    set -e

    # Check if no tests were run
    # Look for various indicators that no tests matched:
    # - "[no tests to run]" from gotestsum
    # - "DONE 0 tests" from test output
    # - "EMPTY" package with cached result
    if grep -q '\[no tests to run\]' "$test_output_file" || \
       grep -q 'DONE 0 tests' "$test_output_file" || \
       (grep -q 'EMPTY' "$test_output_file" && grep -q 'DONE 0 tests' "$test_output_file"); then
        echo ""
        log_error "No tests were run! The pattern '${run_pattern}' did not match any tests."
        log_error ""
        log_error "This usually means:"
        log_error "  1. The test pattern is incorrect"
        log_error "  2. The test name was misspelled"
        log_error "  3. The test does not exist in package ${test_pkg}"
        log_error ""
        log_error "Run '$0 --list' to see available tests"
        exit 1
    fi

    # Check the test exit code
    if [[ $test_exit_code -ne 0 ]]; then
        echo ""
        log_error "Test execution failed with exit code $test_exit_code"
        exit $test_exit_code
    fi

    echo ""
    log_success "Test execution completed!"
}

main "$@"
