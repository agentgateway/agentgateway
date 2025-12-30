#!/usr/bin/env bash
#
# Build script for WASM security guards
#

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}Building WASM Security Guards${NC}"
echo ""

# Check prerequisites
echo "Checking prerequisites..."

if ! command -v rustc &> /dev/null; then
    echo -e "${RED}Error: Rust not installed${NC}"
    echo "Install from: https://rustup.rs"
    exit 1
fi

if ! rustup target list | grep -q "wasm32-wasi (installed)"; then
    echo -e "${YELLOW}Installing wasm32-wasi target...${NC}"
    rustup target add wasm32-wasi
fi

if ! command -v wasm-tools &> /dev/null; then
    echo -e "${YELLOW}Installing wasm-tools...${NC}"
    cargo install wasm-tools
fi

echo -e "${GREEN}✓ Prerequisites OK${NC}"
echo ""

# Build each guard
for guard_dir in */; do
    # Skip if not a guard directory (must have Cargo.toml)
    if [ ! -f "$guard_dir/Cargo.toml" ]; then
        continue
    fi

    guard_name="${guard_dir%/}"
    echo -e "${GREEN}Building $guard_name...${NC}"

    cd "$guard_dir"

    # Build WASM module
    echo "  Compiling..."
    cargo build --target wasm32-wasi --release --quiet

    # Convert to Component Model
    echo "  Creating component..."
    wasm-tools component new \
        "target/wasm32-wasi/release/${guard_name//-/_}.wasm" \
        -o "${guard_name}.component.wasm"

    # Optimize (if wasm-opt is available)
    if command -v wasm-opt &> /dev/null; then
        echo "  Optimizing..."
        wasm-opt -Oz \
            -o "${guard_name}.wasm" \
            "${guard_name}.component.wasm"
        rm "${guard_name}.component.wasm"
    else
        mv "${guard_name}.component.wasm" "${guard_name}.wasm"
    fi

    # Show size
    size=$(wc -c < "${guard_name}.wasm" | awk '{print int($1/1024)"KB"}')
    echo -e "  ${GREEN}✓ Built: ${guard_name}.wasm ($size)${NC}"

    cd ..
    echo ""
done

echo -e "${GREEN}All guards built successfully!${NC}"
echo ""
echo "To use a guard, reference it in your config:"
echo ""
echo "  security_guards:"
echo "    - id: my-guard"
echo "      type: wasm"
echo "      module_path: ./examples/wasm-guards/simple-pattern-guard/simple-pattern-guard.wasm"
echo ""
