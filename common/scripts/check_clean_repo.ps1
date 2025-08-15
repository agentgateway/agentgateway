# check_clean_repo.ps1
# Usage: .\check_clean_repo.ps1

# Check for uncommitted changes
$gitStatus = git status --porcelain

if ($gitStatus) {
    git status
    git diff
    Write-Host "ERROR: Some files need to be updated, please run 'make gen' and include any changed files in your PR"
    exit 1
}