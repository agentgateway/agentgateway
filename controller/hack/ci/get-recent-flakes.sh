#!/usr/bin/env bash

# Get merge queue e2e failures from the past `DAYS` days, sorted by frequency.

show_help() {
    cat << EOF
Usage: $0 [OPTIONS]

Get merge queue e2e failures from the past N days, sorted by frequency.

Options:
  -h, --help                        Show this help message
  -d, --days DAYS                   Number of days to look back (default: 1)
  --show-urls                       Display run URLs in output (default: true)
  --no-show-urls                    Hide run URLs in output
  --disable-helper-command          Disable printing gh command for each run
  --no-disable-helper-command       Enable printing gh command for each run (default)
  --disable-reproduce-command       Disable printing local reproduce command for each test
  --no-disable-reproduce-command    Enable printing local reproduce command for each test (default)

Environment variables (overridden by command-line options):
  DAYS                              Number of days to look back (default: 1)
  SHOW_URLS                         Whether to display run URLs (default: true)
  DISABLE_HELPER_COMMAND            Set to disable printing gh command for each run (default: false)
  DISABLE_REPRODUCE_COMMAND         Set to disable printing local reproduce command (default: false)

Examples:
  $0 --days 7
  $0 --no-show-urls
  DAYS=3 $0
  $0 --disable-reproduce-command
EOF
    exit 0
}

# Default values from environment variables
days=${DAYS:-1}
show_urls=${SHOW_URLS:-true}
disable_helper_command=${DISABLE_HELPER_COMMAND:-false}
disable_reproduce_command=${DISABLE_REPRODUCE_COMMAND:-false}

# Parse command-line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -h|--help)
            show_help
            ;;
        -d|--days)
            days="$2"
            shift 2
            ;;
        --show-urls)
            show_urls="true"
            shift
            ;;
        --no-show-urls)
            show_urls="false"
            shift
            ;;
        --disable-helper-command)
            disable_helper_command="true"
            shift
            ;;
        --no-disable-helper-command)
            disable_helper_command="false"
            shift
            ;;
        --disable-reproduce-command)
            disable_reproduce_command="true"
            shift
            ;;
        --no-disable-reproduce-command)
            disable_reproduce_command="false"
            shift
            ;;
        *)
            echo "Unknown option: $1"
            echo "Use --help for usage information"
            exit 1
            ;;
    esac
done

if date --version >/dev/null 2>&1; then
  # GNU date
  start_date=$(date -d "$days days ago" +"%Y-%m-%dT%H:%M:%S")
else
  # BSD date
  start_date=$(date -v-${days}d +"%Y-%m-%dT%H:%M:%S")
fi

repo_info=$(gh repo view --json owner,name -q '.owner.login + "/" + .name')
failing_run_ids=$(gh run list -w=".github/workflows/e2e.yaml" --event=merge_group --created ">$start_date" --limit 100 -s failure --json databaseId | jq '.[].databaseId' -r)

# map of test_name to list of run_urls
# Format: test_map["test_name"]="run_url1,run_url2,run_url3"
declare -A test_map

for run_id in $failing_run_ids
do
    run_url="https://github.com/$repo_info/actions/runs/$run_id"
    failure_lines=$(gh run view $run_id --log-failed | grep FAIL:)
    # extract the test names from the log lines
    failing_tests=$(echo $failure_lines | grep -o 'FAIL: [^ ]*' | sed 's/FAIL: //')

    # associate each failing test with this run_url
    while IFS= read -r test_name; do
        if [[ -n "$test_name" ]]; then
            # Check if this test already exists in our map
            if [[ -n "${test_map[$test_name]}" ]]; then
                # Test exists, append run_url to existing list
                test_map[$test_name]="${test_map[$test_name]},$run_url"
            else
                # New test, add to map
                test_map[$test_name]="$run_url"
            fi
        fi
    done <<< "$failing_tests"
    echo -n "." # progress indicator
done

echo -e "\nFailing e2e tests in merge queue over last $days day(s):"
echo "====================================="

# Process each test entry and count occurrences
sorted_tests=""
for test_name in "${!test_map[@]}"; do
    run_urls="${test_map[$test_name]}"
    # Count occurrences by counting commas in run_urls + 1
    count=$(echo "$run_urls" | tr ',' '\n' | grep -c .)
    sorted_tests="$sorted_tests$count|$test_name|$run_urls"$'\n'
done

# Sort by count (descending)
sorted_tests=$(echo "$sorted_tests" | sort -t'|' -k1,1nr)

# Filter out tests that are prefixes of other tests and display results
while IFS= read -r line; do
    if [[ -n "$line" ]]; then
        # Extract the count, test name, and run_urls
        count=$(echo "$line" | cut -d'|' -f1)
        test_name=$(echo "$line" | cut -d'|' -f2)
        run_urls=$(echo "$line" | cut -d'|' -f3)

        # Check if this test name is a prefix of any other test name in the list
        is_prefix=false
        while IFS= read -r other_line; do
            if [[ -n "$other_line" && "$line" != "$other_line" ]]; then
                other_test_name=$(echo "$other_line" | cut -d'|' -f2)
                if [[ "$other_test_name" == "$test_name"* ]]; then
                    is_prefix=true
                    break
                fi
            fi
        done <<< "$sorted_tests"

        # Only include if it's not a prefix of another test
        if [[ "$is_prefix" == false ]]; then
            if [[ "$count" == "1" ]]; then
                echo "$test_name (Failed 1 time)"
            else
                echo "$test_name (Failed $count times)"
            fi
            # Print local reproduce command
            if [[ "$disable_reproduce_command" != "true" ]]; then
                # Format test name as ^A$/^B$/^C$
                formatted_test_name="^$(echo "$test_name" | sed 's|/|\$/^|g')\$"
                echo "  # Reproduce locally:"
                echo "  ./hack/run-e2e-test.sh --persist $formatted_test_name"
            fi
            # Process each run URL
            while IFS= read -r url; do
                if [[ -n "$url" ]]; then
                    if [[ "$show_urls" == "true" ]]; then
                        echo "    # CI web page:"
			if command -v open >/dev/null 2>&1; then
			    echo "    open $url"
			else
			    echo "    $url"
			fi
                    fi
                    if [[ "$disable_helper_command" != "true" ]]; then
                        # Extract run ID from URL (format: https://github.com/owner/repo/actions/runs/12345)
                        run_id=$(echo "$url" | grep -o '/runs/[0-9]*' | cut -d'/' -f3)
                        echo "    # Print on stdout the CI logs of failed tests:"
                        echo "    gh -R $repo_info run view $run_id --log-failed"
                    fi
                fi
            done <<< "$(echo "$run_urls" | tr ',' '\n')"
            echo ""
        fi
    fi
done <<< "$sorted_tests"
