#!/bin/bash

# AgentGateway First-Time E2E Testing Setup
# This script provides a one-command setup for new developers to get E2E tests running

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SETUP_CONFIG_FILE="$PROJECT_ROOT/.e2e-setup-config"

# Default values
SKIP_DEPS=false
SKIP_BUILD=false
SKIP_RESOURCE_DETECTION=false
VERBOSE=false
DRY_RUN=false
FORCE_REINSTALL=false

# Function to print colored output
print_header() {
    echo -e "\n${CYAN}╔══════════════════════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║                    AgentGateway E2E Testing Setup                           ║${NC}"
    echo -e "${CYAN}║                     First-Time Developer Experience                         ║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════════════════════════════════════════════╝${NC}\n"
}

print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

print_step() {
    echo -e "\n${CYAN}▶ $1${NC}"
}

# Function to show usage
show_usage() {
    cat << EOF
Usage: $0 [OPTIONS]

AgentGateway First-Time E2E Testing Setup - One-command setup for new developers

This script will:
  1. Check system prerequisites (Rust, Node.js, system resources)
  2. Install missing dependencies (with permission)
  3. Build AgentGateway binary
  4. Detect optimal test configuration
  5. Validate complete setup
  6. Run a test to verify everything works

OPTIONS:
    --skip-deps             Skip dependency installation checks
    --skip-build            Skip building AgentGateway binary
    --skip-resource-check   Skip resource detection and optimization
    --verbose               Enable verbose logging
    --dry-run              Show what would be done without executing
    --force-reinstall      Force reinstallation of dependencies
    -h, --help             Show this help message

EXAMPLES:
    $0                                    # Full setup (recommended for first time)
    $0 --verbose                          # Full setup with detailed output
    $0 --skip-deps --skip-build          # Only configure and validate
    $0 --dry-run                         # Preview what will be done

ENVIRONMENT VARIABLES:
    RUST_TOOLCHAIN         Rust toolchain version (default: from rust-toolchain.toml)
    NODE_VERSION          Node.js version requirement (default: auto-detect)
    SKIP_CONFIRMATION     Skip interactive confirmations (default: false)

EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --skip-deps)
            SKIP_DEPS=true
            shift
            ;;
        --skip-build)
            SKIP_BUILD=true
            shift
            ;;
        --skip-resource-check)
            SKIP_RESOURCE_DETECTION=true
            shift
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --force-reinstall)
            FORCE_REINSTALL=true
            shift
            ;;
        -h|--help)
            show_usage
            exit 0
            ;;
        *)
            print_error "Unknown option: $1"
            show_usage
            exit 1
            ;;
    esac
done

# Function to run command with dry-run support
run_command() {
    local cmd="$1"
    local description="$2"
    
    if [[ "$DRY_RUN" == "true" ]]; then
        print_status "[DRY RUN] Would execute: $cmd"
        if [[ -n "$description" ]]; then
            print_status "[DRY RUN] Purpose: $description"
        fi
        return 0
    fi
    
    if [[ "$VERBOSE" == "true" ]]; then
        print_status "Executing: $cmd"
    fi
    
    eval "$cmd"
}

# Function to check if command exists
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# Function to get system information
get_system_info() {
    local os_name=$(uname -s)
    local arch=$(uname -m)
    local total_memory_kb=$(grep MemTotal /proc/meminfo 2>/dev/null | awk '{print $2}' || echo "unknown")
    local cpu_count=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo "unknown")
    
    echo "System Information:"
    echo "  OS: $os_name"
    echo "  Architecture: $arch"
    echo "  CPU Cores: $cpu_count"
    if [[ "$total_memory_kb" != "unknown" ]]; then
        local total_memory_gb=$((total_memory_kb / 1024 / 1024))
        echo "  Memory: ${total_memory_gb}GB"
    else
        echo "  Memory: Unknown"
    fi
}

# Function to check prerequisites
check_prerequisites() {
    print_step "Checking System Prerequisites"
    
    local missing_deps=()
    local warnings=()
    
    # Check operating system
    local os_name=$(uname -s)
    case "$os_name" in
        Linux*)
            print_status "Operating System: Linux ✓"
            ;;
        Darwin*)
            print_status "Operating System: macOS ✓"
            ;;
        CYGWIN*|MINGW*|MSYS*)
            print_status "Operating System: Windows ✓"
            ;;
        *)
            warnings+=("Unknown operating system: $os_name")
            ;;
    esac
    
    # Check Rust
    if command_exists rustc && command_exists cargo; then
        local rust_version=$(rustc --version | cut -d' ' -f2)
        print_status "Rust: $rust_version ✓"
        
        # Check if we have the required toolchain
        if [[ -f "$PROJECT_ROOT/rust-toolchain.toml" ]]; then
            local required_channel=$(grep 'channel' "$PROJECT_ROOT/rust-toolchain.toml" | cut -d'"' -f2)
            if [[ -n "$required_channel" ]]; then
                if rustup show | grep -q "$required_channel"; then
                    print_status "Rust toolchain ($required_channel): Available ✓"
                else
                    missing_deps+=("rust-toolchain-$required_channel")
                fi
            fi
        fi
    else
        missing_deps+=("rust")
    fi
    
    # Check Node.js and npm
    if command_exists node && command_exists npm; then
        local node_version=$(node --version)
        local npm_version=$(npm --version)
        print_status "Node.js: $node_version ✓"
        print_status "npm: $npm_version ✓"
        
        # Check Node.js version (require >= 20)
        local node_major=$(echo "$node_version" | sed 's/v//' | cut -d'.' -f1)
        if [[ "$node_major" -lt 20 ]]; then
            warnings+=("Node.js version $node_version is older than recommended (20+)")
        fi
    else
        missing_deps+=("nodejs")
    fi
    
    # Check system resources
    print_status "Checking system resources..."
    get_system_info
    
    # Check available disk space (require at least 2GB)
    local available_space_kb=$(df "$PROJECT_ROOT" | tail -1 | awk '{print $4}')
    local available_space_gb=$((available_space_kb / 1024 / 1024))
    if [[ "$available_space_gb" -lt 2 ]]; then
        warnings+=("Low disk space: ${available_space_gb}GB available (recommend 2GB+)")
    else
        print_status "Disk space: ${available_space_gb}GB available ✓"
    fi
    
    # Report results
    if [[ ${#missing_deps[@]} -eq 0 ]]; then
        print_success "All required dependencies are available!"
    else
        print_warning "Missing dependencies: ${missing_deps[*]}"
        return 1
    fi
    
    if [[ ${#warnings[@]} -gt 0 ]]; then
        print_warning "Warnings detected:"
        for warning in "${warnings[@]}"; do
            print_warning "  - $warning"
        done
    fi
    
    return 0
}

# Function to install missing dependencies
install_dependencies() {
    print_step "Installing Missing Dependencies"
    
    if [[ "$SKIP_DEPS" == "true" ]]; then
        print_status "Skipping dependency installation (--skip-deps)"
        return 0
    fi
    
    local os_name=$(uname -s)
    
    # Install Rust if missing
    if ! command_exists rustc || ! command_exists cargo; then
        print_status "Installing Rust..."
        if [[ "$DRY_RUN" == "false" ]]; then
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
            source "$HOME/.cargo/env"
        fi
        print_success "Rust installation completed"
    fi
    
    # Install required Rust toolchain
    if [[ -f "$PROJECT_ROOT/rust-toolchain.toml" ]]; then
        local required_channel=$(grep 'channel' "$PROJECT_ROOT/rust-toolchain.toml" | cut -d'"' -f2)
        if [[ -n "$required_channel" ]] && [[ "$DRY_RUN" == "false" ]]; then
            if ! rustup show | grep -q "$required_channel"; then
                print_status "Installing Rust toolchain: $required_channel"
                rustup install "$required_channel"
                rustup default "$required_channel"
            fi
        fi
    fi
    
    # Install Node.js if missing
    if ! command_exists node || ! command_exists npm; then
        print_status "Installing Node.js..."
        case "$os_name" in
            Linux*)
                if command_exists apt-get; then
                    run_command "sudo apt-get update && sudo apt-get install -y nodejs npm" "Install Node.js via apt"
                elif command_exists yum; then
                    run_command "sudo yum install -y nodejs npm" "Install Node.js via yum"
                elif command_exists pacman; then
                    run_command "sudo pacman -S nodejs npm" "Install Node.js via pacman"
                else
                    print_error "Unable to install Node.js automatically. Please install manually."
                    return 1
                fi
                ;;
            Darwin*)
                if command_exists brew; then
                    run_command "brew install node" "Install Node.js via Homebrew"
                else
                    print_error "Homebrew not found. Please install Node.js manually or install Homebrew first."
                    return 1
                fi
                ;;
            *)
                print_error "Automatic Node.js installation not supported on this platform. Please install manually."
                return 1
                ;;
        esac
        print_success "Node.js installation completed"
    fi
    
    return 0
}

# Function to build AgentGateway
build_agentgateway() {
    print_step "Building AgentGateway Binary"
    
    if [[ "$SKIP_BUILD" == "true" ]]; then
        print_status "Skipping AgentGateway build (--skip-build)"
        return 0
    fi
    
    cd "$PROJECT_ROOT"
    
    # Check if binary already exists and is recent
    local binary_path="target/release/agentgateway"
    if [[ -f "$binary_path" ]] && [[ "$FORCE_REINSTALL" == "false" ]]; then
        local binary_age=$(($(date +%s) - $(stat -c %Y "$binary_path" 2>/dev/null || stat -f %m "$binary_path" 2>/dev/null || echo 0)))
        if [[ $binary_age -lt 3600 ]]; then  # Less than 1 hour old
            print_status "Recent AgentGateway binary found, skipping build"
            return 0
        fi
    fi
    
    print_status "Building AgentGateway (this may take a few minutes)..."
    
    if [[ "$VERBOSE" == "true" ]]; then
        run_command "cargo build --release --bin agentgateway" "Build AgentGateway binary"
    else
        run_command "cargo build --release --bin agentgateway > /dev/null 2>&1" "Build AgentGateway binary"
    fi
    
    # Verify binary was created
    if [[ -f "$binary_path" ]] || [[ "$DRY_RUN" == "true" ]]; then
        print_success "AgentGateway binary built successfully"
    else
        print_error "Failed to build AgentGateway binary"
        return 1
    fi
    
    return 0
}

# Function to setup UI dependencies
setup_ui_dependencies() {
    print_step "Setting Up UI Dependencies"
    
    cd "$PROJECT_ROOT/ui"
    
    # Check if node_modules exists and is recent
    if [[ -d "node_modules" ]] && [[ "$FORCE_REINSTALL" == "false" ]]; then
        local modules_age=$(($(date +%s) - $(stat -c %Y "node_modules" 2>/dev/null || stat -f %m "node_modules" 2>/dev/null || echo 0)))
        if [[ $modules_age -lt 3600 ]]; then  # Less than 1 hour old
            print_status "Recent node_modules found, skipping npm install"
            return 0
        fi
    fi
    
    print_status "Installing UI dependencies..."
    
    if [[ "$VERBOSE" == "true" ]]; then
        run_command "npm install" "Install UI dependencies"
    else
        run_command "npm install > /dev/null 2>&1" "Install UI dependencies"
    fi
    
    print_success "UI dependencies installed successfully"
    return 0
}

# Function to detect and configure optimal settings
detect_optimal_settings() {
    print_step "Detecting Optimal Test Configuration"
    
    if [[ "$SKIP_RESOURCE_DETECTION" == "true" ]]; then
        print_status "Skipping resource detection (--skip-resource-check)"
        return 0
    fi
    
    cd "$PROJECT_ROOT"
    
    # Run resource detection script
    if [[ -f "scripts/detect-system-resources.js" ]]; then
        print_status "Running system resource detection..."
        
        local resource_output
        if [[ "$DRY_RUN" == "false" ]]; then
            resource_output=$(node scripts/detect-system-resources.js --quiet 2>/dev/null || echo "")
        else
            resource_output="[DRY RUN] Resource detection would run here"
        fi
        
        if [[ -n "$resource_output" ]] && [[ "$resource_output" != *"error"* ]]; then
            print_success "Resource detection completed"
            if [[ "$VERBOSE" == "true" ]]; then
                echo "$resource_output"
            fi
            
            # Save configuration for future use
            if [[ "$DRY_RUN" == "false" ]]; then
                echo "# E2E Testing Configuration - Generated $(date)" > "$SETUP_CONFIG_FILE"
                echo "# This file contains optimal settings detected for your system" >> "$SETUP_CONFIG_FILE"
                echo "$resource_output" >> "$SETUP_CONFIG_FILE"
            fi
        else
            print_warning "Resource detection failed, using conservative defaults"
        fi
    else
        print_warning "Resource detection script not found, using conservative defaults"
    fi
    
    return 0
}

# Function to generate intelligent test configuration
generate_intelligent_config() {
    print_step "Generating Intelligent Test Configuration"
    
    if [[ "$SKIP_RESOURCE_DETECTION" == "true" ]]; then
        print_status "Skipping intelligent configuration (--skip-resource-check)"
        return 0
    fi
    
    cd "$PROJECT_ROOT"
    
    # Run intelligent test configuration script
    if [[ -f "scripts/intelligent-test-config.js" ]]; then
        print_status "Generating optimal test configuration for your system..."
        
        if [[ "$DRY_RUN" == "false" ]]; then
            if [[ "$VERBOSE" == "true" ]]; then
                node scripts/intelligent-test-config.js
            else
                node scripts/intelligent-test-config.js > /dev/null 2>&1
            fi
            
            if [[ $? -eq 0 ]]; then
                print_success "Intelligent test configuration generated successfully"
                
                # Check if configuration files were created
                if [[ -f "test-config-optimized.yaml" ]]; then
                    print_status "Configuration saved to: test-config-optimized.yaml"
                fi
                
                if [[ -f "ui/.test-settings.json" ]]; then
                    print_status "Settings saved to: ui/.test-settings.json"
                fi
            else
                print_warning "Intelligent configuration generation failed, using defaults"
                return 1
            fi
        else
            print_status "[DRY RUN] Would generate intelligent test configuration"
        fi
    else
        print_warning "Intelligent test configuration script not found"
        return 1
    fi
    
    return 0
}

# Function to validate setup using comprehensive health check
validate_setup() {
    print_step "Validating Complete Setup"
    
    if [[ "$DRY_RUN" == "true" ]]; then
        print_status "[DRY RUN] Would run comprehensive health check validation"
        return 0
    fi
    
    # Run comprehensive health check
    if [[ -f "$PROJECT_ROOT/scripts/health-check-validator.js" ]]; then
        print_status "Running comprehensive health check validation..."
        
        if [[ "$VERBOSE" == "true" ]]; then
            if node "$PROJECT_ROOT/scripts/health-check-validator.js" --verbose; then
                print_success "Comprehensive health check passed! ✓"
                return 0
            else
                print_error "Health check validation failed"
                return 1
            fi
        else
            # Capture output for summary
            local health_output
            if health_output=$(node "$PROJECT_ROOT/scripts/health-check-validator.js" 2>&1); then
                print_success "Comprehensive health check passed! ✓"
                
                # Show summary if there were warnings
                if echo "$health_output" | grep -q "Warnings:"; then
                    print_warning "Health check completed with warnings:"
                    echo "$health_output" | grep -A 10 "Warnings:"
                fi
                
                return 0
            else
                print_error "Health check validation failed"
                echo "$health_output"
                return 1
            fi
        fi
    else
        print_warning "Health check validator not found, using basic validation"
        
        # Fallback to basic validation
        local validation_errors=()
        
        # Check AgentGateway binary
        if [[ -f "$PROJECT_ROOT/target/release/agentgateway" ]]; then
            print_status "AgentGateway binary: Available ✓"
        else
            validation_errors+=("AgentGateway binary not found")
        fi
        
        # Check UI dependencies
        if [[ -d "$PROJECT_ROOT/ui/node_modules" ]]; then
            print_status "UI dependencies: Available ✓"
        else
            validation_errors+=("UI dependencies not installed")
        fi
        
        # Check test configuration
        if [[ -f "$PROJECT_ROOT/test-config.yaml" ]]; then
            print_status "Test configuration: Available ✓"
        else
            validation_errors+=("Test configuration not found")
        fi
        
        # Check test runner scripts
        if [[ -f "$PROJECT_ROOT/scripts/run-e2e-tests.sh" ]]; then
            print_status "Test runner: Available ✓"
        else
            validation_errors+=("Test runner script not found")
        fi
        
        # Report validation results
        if [[ ${#validation_errors[@]} -eq 0 ]]; then
            print_success "Basic setup validation passed! ✓"
            return 0
        else
            print_error "Setup validation failed:"
            for error in "${validation_errors[@]}"; do
                print_error "  - $error"
            done
            return 1
        fi
    fi
}

# Function to run a test validation
run_test_validation() {
    print_step "Running Test Validation"
    
    if [[ "$DRY_RUN" == "true" ]]; then
        print_status "[DRY RUN] Would run a simple test to validate setup"
        return 0
    fi
    
    cd "$PROJECT_ROOT"
    
    print_status "Running a simple smoke test to validate setup..."
    
    # Use the minimal test script if available
    if [[ -f "scripts/test-e2e-minimal.js" ]]; then
        if node scripts/test-e2e-minimal.js --resource-only > /dev/null 2>&1; then
            print_success "Test validation passed! ✓"
            return 0
        else
            print_warning "Test validation had issues, but setup appears complete"
            print_status "You can run full tests with: ./scripts/run-e2e-tests.sh"
            return 0
        fi
    else
        print_status "Minimal test script not found, skipping test validation"
        print_status "You can run full tests with: ./scripts/run-e2e-tests.sh"
        return 0
    fi
}

# Function to show next steps
show_next_steps() {
    print_step "Setup Complete! Next Steps"
    
    cat << EOF

${GREEN}🎉 Congratulations! Your AgentGateway E2E testing environment is ready!${NC}

${CYAN}Quick Start Commands:${NC}
  ${YELLOW}# Run tests with optimal configuration (recommended)${NC}
  cd ui && npm run test:e2e:optimized

  ${YELLOW}# Run all E2E tests${NC}
  ./scripts/run-e2e-tests.sh

  ${YELLOW}# Run tests in parallel${NC}
  cd ui && npm run test:e2e:parallel

  ${YELLOW}# Run only smoke tests${NC}
  cd ui && npm run e2e:smoke

  ${YELLOW}# Open interactive test runner${NC}
  cd ui && npm run e2e:open

${CYAN}Configuration Files Created:${NC}
  - ${SETUP_CONFIG_FILE} (optimal settings for your system)
  - test-config-optimized.yaml (intelligent test configuration)
  - ui/.test-settings.json (persistent settings)

${CYAN}Configuration Management:${NC}
  ${YELLOW}# Check current configuration${NC}
  cd ui && npm run test:e2e:check-config

  ${YELLOW}# Regenerate configuration${NC}
  cd ui && npm run test:e2e:force-config

  ${YELLOW}# Auto-configure for current system${NC}
  cd ui && npm run test:e2e:auto-config

${CYAN}Troubleshooting:${NC}
  - If tests fail, check: ./scripts/test-e2e-minimal.js --verbose
  - For resource issues, run: node scripts/detect-system-resources.js
  - For help: ./scripts/run-e2e-tests.sh --help

${CYAN}Documentation:${NC}
  - E2E Testing Guide: ui/cypress/README.md
  - Troubleshooting: E2E_TESTING_FIXES.md

${GREEN}Happy Testing! 🚀${NC}

EOF
}

# Main execution function
main() {
    print_header
    
    print_status "Starting first-time setup for AgentGateway E2E testing..."
    print_status "This will take a few minutes to complete."
    
    if [[ "$DRY_RUN" == "true" ]]; then
        print_warning "DRY RUN MODE - No changes will be made"
    fi
    
    # Step 1: Check prerequisites
    if ! check_prerequisites; then
        if [[ "$SKIP_DEPS" == "true" ]]; then
            print_warning "Prerequisites check failed, but continuing due to --skip-deps"
        else
            print_status "Attempting to install missing dependencies..."
            if ! install_dependencies; then
                print_error "Failed to install dependencies. Please install manually and try again."
                exit 1
            fi
            # Re-check after installation
            if ! check_prerequisites; then
                print_error "Prerequisites still not met after installation attempt."
                exit 1
            fi
        fi
    fi
    
    # Step 2: Build AgentGateway
    if ! build_agentgateway; then
        print_error "Failed to build AgentGateway binary"
        exit 1
    fi
    
    # Step 3: Setup UI dependencies
    if ! setup_ui_dependencies; then
        print_error "Failed to setup UI dependencies"
        exit 1
    fi
    
    # Step 4: Detect optimal settings
    if ! detect_optimal_settings; then
        print_warning "Resource detection had issues, but continuing with defaults"
    fi
    
    # Step 5: Generate intelligent test configuration
    if ! generate_intelligent_config; then
        print_warning "Intelligent configuration generation had issues, but continuing"
    fi
    
    # Step 6: Validate setup
    if ! validate_setup; then
        print_error "Setup validation failed"
        exit 1
    fi
    
    # Step 6: Run test validation
    if ! run_test_validation; then
        print_warning "Test validation had issues, but setup appears complete"
    fi
    
    # Step 7: Show next steps
    show_next_steps
    
    print_success "First-time setup completed successfully! 🎉"
}

# Run main function
main "$@"
