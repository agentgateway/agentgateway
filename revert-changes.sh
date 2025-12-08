#!/usr/bin/env bash
set -e

echo "Revirtiendo cambios en el código del Agent Gateway..."

cd "$(dirname "$0")"

# Revertir archivos modificados
git restore crates/agentgateway/src/http/jwt.rs
git restore crates/agentgateway/src/serdes.rs
git restore crates/agentgateway/src/types/local.rs

echo "✅ Cambios revertidos exitosamente"
echo ""
echo "Archivos revertidos:"
echo "  - crates/agentgateway/src/http/jwt.rs"
echo "  - crates/agentgateway/src/serdes.rs"
echo "  - crates/agentgateway/src/types/local.rs"
echo ""
echo "Verificar estado:"
git status




