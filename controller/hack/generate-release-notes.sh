#!/usr/bin/env bash

set -o errexit
set -o nounset

# Track execution time
SECONDS=0

# Default values
PREVIOUS_TAG=""
CURRENT_TAG="HEAD"
OUTPUT_DIR="${OUTPUT_DIR:-_output}"
OUTPUT_FILE="${OUTPUT_DIR}/RELEASE_NOTES.md"
REPO_OVERRIDE=""
TEMP_DIR=""
declare -A CONTRIBUTORS_MAP

# Help message
function show_help() {
    echo "Usage: $0 [options]"
    echo "Options:"
    echo "  -p, --previous-tag <tag>     Previous release tag (required)"
    echo "  -c, --current-tag <tag>      Current release tag (default: HEAD)"
    echo "  -r, --repo <owner/repo>      GitHub repository (default: kgateway-dev/kgateway)"
    echo "  -o, --output <file>          Output file (default: _output/RELEASE_NOTES.md)"
    echo "  -h, --help                   Show this help message"
    echo ""
    echo "Environment variables:"
    echo "  GITHUB_TOKEN               GitHub API token (required)"
    echo ""
    echo "Examples:"
    echo "  $0 -p v2.0.0 -c v2.1.0"
    echo "  $0 -p v2.0.0-rc.1 -c v2.0.0-rc.2 -o changelog.md"
}

function cleanup() {
    if [ -n "$TEMP_DIR" ] && [ -d "$TEMP_DIR" ]; then
        rm -rf "$TEMP_DIR" || echo "Warning: Failed to cleanup temporary directory $TEMP_DIR"
    fi
}

# Get section title for a given kind
function get_section_title() {
    local kind="$1"
    case "$kind" in
        "breaking_change") echo "Breaking Changes" ;;
        "feature") echo "New Features" ;;
        "fix") echo "Bug Fixes" ;;
        "deprecation") echo "Deprecations" ;;
        "documentation") echo "Documentation" ;;
        "cleanup") echo "Cleanup" ;;
        "install") echo "Installation Changes" ;;
        "bump") echo "Dependency Updates" ;;
        *) echo "" ;;
    esac
}

# Check if a release note is effectively "NONE"
function is_none_release_note() {
    local note="$1"
    # Convert to lowercase and trim whitespace
    local normalized=$(echo "$note" | tr '[:upper:]' '[:lower:]' | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')

    # Check if empty or "none"
    if [ -z "$normalized" ] || [ "$normalized" = "none" ]; then
        return 0  # true in bash
    fi
    return 1  # false in bash
}

# Extract release note from PR body
function extract_release_note() {
    local body="$1"
    local in_block=0
    local note=""

    # Strip Windows carriage returns
    body=$(echo "$body" | tr -d '\r')

    while IFS= read -r line; do
        if [[ "$line" =~ ^[[:space:]]*\`\`\`release-note[[:space:]]*$ ]]; then
            in_block=1
        elif [[ "$line" =~ ^[[:space:]]*\`\`\`[[:space:]]*$ ]] && [ "$in_block" -eq 1 ]; then
            in_block=0
            break
        elif [ "$in_block" -eq 1 ]; then
            if [ -n "$note" ]; then
                note+=$'\n'
            fi
            note+="$line"
        fi
    done <<< "$body"

    # Trim leading/trailing whitespace while preserving internal formatting
    note=$(echo "$note" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')
    echo "$note"
}

# Format a release note for markdown output
function format_release_note() {
    local note="$1"
    local pr_link="$2"

    # Handle single line vs multiline notes differently
    if [[ "$note" =~ $'\n' ]]; then
        # Multiline note: format as bullet point with proper indentation
        # First line gets the bullet point
        local first_line=$(echo "$note" | head -n1)
        local rest_lines=$(echo "$note" | tail -n +2)

        echo "* ${first_line}"
        # Indent continuation lines with 2 spaces
        if [ -n "$rest_lines" ]; then
            echo "$rest_lines" | sed 's/^/  /'
        fi
        # Add PR link on a separate indented line
        echo "  ${pr_link}"
    else
        # Single line note: add note and PR link on same line
        echo "* ${note} ${pr_link}"
    fi
}

# Process PRs for the repository
function process_prs() {
    local owner="$1"
    local repo="$2"
    local start_ref="$3"
    local end_ref="$4"
    local pr_numbers_file="$5"

    # Get PR numbers from commit messages
    # First, get the git log output
    local git_log_output
    git_log_output=$(git log "$start_ref..$end_ref" --pretty=format:"%s%n%b")

    # Check if git log has any output
    if [ -z "$git_log_output" ]; then
        echo "No commits found between $start_ref and $end_ref"
        return
    fi

    # Extract PR numbers from the git log output
    echo "$git_log_output" | \
        grep -o -E "(#|\\()[0-9]+\\)?" | \
        tr -d '#()' | \
        sort -un > "$pr_numbers_file"

    if [ ! -s "$pr_numbers_file" ]; then
        echo "No PR numbers found in commit messages between $start_ref and $end_ref"
        return
    fi

    echo "Processing PRs from $owner/$repo..."

    # Process each PR
    while read -r PR_NUMBER; do
        # Fetch PR data using GitHub API
        PR_DATA=$(curl -s -H "Authorization: token $GITHUB_TOKEN" \
            -H "Accept: application/vnd.github.v3+json" \
            "https://api.github.com/repos/$owner/$repo/pulls/$PR_NUMBER")

        # Check if PR exists
        if [[ $(echo "$PR_DATA" | jq -r '.message // empty') == "Not Found" ]]; then
            echo "PR #$PR_NUMBER not found, trying issue endpoint..."
            # Try to fetch as an issue instead
            PR_DATA=$(curl -s -H "Authorization: token $GITHUB_TOKEN" \
                -H "Accept: application/vnd.github.v3+json" \
                "https://api.github.com/repos/$owner/$repo/issues/$PR_NUMBER")
            if [[ $(echo "$PR_DATA" | jq -r '.message // empty') == "Not Found" ]]; then
                echo "Issue #$PR_NUMBER not found either, skipping"
                continue
            fi
        fi

        # Extract PR author and track in contributors map
        PR_AUTHOR=$(echo "$PR_DATA" | jq -r '.user.login // empty')
        if [ -n "$PR_AUTHOR" ] && [ "$PR_AUTHOR" != "Copilot" ]; then
            CONTRIBUTORS_MAP["$PR_AUTHOR"]=1
        fi

        # Extract PR title and body
        PR_TITLE=$(echo "$PR_DATA" | jq -r .title)
        PR_BODY=$(echo "$PR_DATA" | jq -r '.body // empty')

        # Extract release note
        RELEASE_NOTE=$(extract_release_note "$PR_BODY")

        echo "Extracted release note for PR #$PR_NUMBER: '$RELEASE_NOTE'"

        # Skip if release note is "NONE" or empty
        if is_none_release_note "$RELEASE_NOTE"; then
            echo "Skipping PR #$PR_NUMBER - release note is none/empty"
            continue
        fi

        # Get PR labels
        LABELS=$(echo "$PR_DATA" | jq -r '.labels[].name')

        # Find the kind label
        KIND=""
        while read -r LABEL; do
            if [[ "$LABEL" =~ ^kind/ ]]; then
                KIND=${LABEL#kind/}
                break
            fi
        done <<< "$LABELS"

        # Skip if no kind label
        if [[ -z "$KIND" ]]; then
            echo "PR #$PR_NUMBER missing kind label, skipping"
            continue
        fi

        echo "Found PR #$PR_NUMBER ($KIND): $PR_TITLE"

        # Get section title
        SECTION_TITLE=$(get_section_title "$KIND")
        if [ -n "$SECTION_TITLE" ]; then
            local pr_link="([#$PR_NUMBER](https://github.com/$owner/$repo/pull/$PR_NUMBER))"
            local formatted_note=$(format_release_note "$RELEASE_NOTE" "$pr_link")
            echo "Formatted note for PR #$PR_NUMBER: '$formatted_note'"
            echo "$formatted_note" >> "$TEMP_DIR/$KIND.txt"
        fi
    done < "$pr_numbers_file"
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -p|--previous-tag)
            PREVIOUS_TAG="$2"
            shift 2
            ;;
        -c|--current-tag)
            CURRENT_TAG="$2"
            shift 2
            ;;
        -o|--output)
            OUTPUT_FILE="$2"
            shift 2
            ;;
        -r|--repo)
            REPO_OVERRIDE="$2"
            shift 2
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            show_help
            exit 1
            ;;
    esac
done

# Validate required parameters
if [ -z "$PREVIOUS_TAG" ]; then
    echo "Error: Previous tag is required"
    show_help
    exit 1
fi

if [ -z "${GITHUB_TOKEN:-}" ]; then
    echo "Error: GITHUB_TOKEN environment variable is required"
    echo "Export it with: export GITHUB_TOKEN=<your_token>"
    exit 1
fi

# Create temporary directory for PR data
TEMP_DIR=$(mktemp -d)
trap cleanup EXIT

# Get repository information (defaults to upstream, can be overridden for forks)
if [ -n "$REPO_OVERRIDE" ]; then
    OWNER=$(echo "$REPO_OVERRIDE" | cut -d'/' -f1)
    REPO=$(echo "$REPO_OVERRIDE" | cut -d'/' -f2)
else
    OWNER="kgateway-dev"
    REPO="kgateway"
fi

echo "Repository: $OWNER/$REPO"
echo "Previous tag: $PREVIOUS_TAG"
echo "Current tag/ref: $CURRENT_TAG"

# Process PRs
echo "Fetching PRs between $PREVIOUS_TAG and $CURRENT_TAG..."
process_prs "$OWNER" "$REPO" "$PREVIOUS_TAG" "$CURRENT_TAG" "$TEMP_DIR/prs.txt"

# Check if we found any release notes
FOUND_NOTES=0
for KIND in breaking_change feature fix deprecation documentation cleanup install bump; do
    if [[ -f "$TEMP_DIR/$KIND.txt" ]]; then
        FOUND_NOTES=1
        break
    fi
done

# Ensure output directory exists
mkdir -p "$(dirname "$OUTPUT_FILE")"

if [[ $FOUND_NOTES -eq 0 ]]; then
    echo "No release notes found in any PRs"
    cat > "$OUTPUT_FILE" << EOF
    ## Release Notes

    No release notes generated
EOF
    exit 0
fi

# Initialize the output file
cat > "$OUTPUT_FILE" << EOF
## Release Notes

### Changes since $PREVIOUS_TAG
EOF

# Generate the final release notes
for KIND in breaking_change feature fix deprecation documentation cleanup install bump; do
    if [[ -f "$TEMP_DIR/$KIND.txt" ]]; then
        SECTION_TITLE=$(get_section_title "$KIND")
        echo -e "\n#### $SECTION_TITLE\n" >> "$OUTPUT_FILE"
        cat "$TEMP_DIR/$KIND.txt" >> "$OUTPUT_FILE"
    fi
done

# Generate contributors section
if [ ${#CONTRIBUTORS_MAP[@]} -gt 0 ]; then
    echo -e "\n## Contributors\n" >> "$OUTPUT_FILE"
    echo "Thanks to all the contributors who made this release possible:" >> "$OUTPUT_FILE"
    echo "" >> "$OUTPUT_FILE"
    # Sort contributors alphabetically (case-insensitive) and display as avatar grid
    for contributor in $(printf '%s\n' "${!CONTRIBUTORS_MAP[@]}" | sort -f); do
        echo -n "<a href=\"https://github.com/${contributor}\"><img src=\"https://github.com/${contributor}.png\" width=\"50\" alt=\"@${contributor}\"></a> " >> "$OUTPUT_FILE"
    done
    echo "" >> "$OUTPUT_FILE"
fi

echo "Release notes have been generated in $OUTPUT_FILE (${SECONDS}s)"
