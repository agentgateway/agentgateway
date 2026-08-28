#!/usr/bin/env bash
# Keeps the FIPS TLS guarantee enforceable from one place.
#
# crypto::tls is the only source of rustls providers used by production code.
# Its factories fail closed in a FIPS build, which means every config built from
# one of those providers is FIPS by construction. This check prevents production
# code from bypassing that factory or changing the rustls settings on which the
# guarantee depends.
set -euo pipefail

cd "$(dirname "$0")/.."

# Scan crate sources, excluding test-helper directories and the central provider
# module.
sources=()
while IFS= read -r source; do
  sources+=("$source")
done < <(
  find crates -path '*/src/*' -name '*.rs' \
    ! -path '*/src/test_helpers/*' \
    ! -path 'crates/agentgateway/src/crypto/tls.rs' | sort
)

status=0
report() { echo "ERROR: $1" >&2; echo "       $2" >&2; status=1; }

# No config construction that relies on an installed process default.
while IFS=: read -r file line _; do
  [[ -z "${file:-}" ]] && continue
  report "${file}:${line} builds a rustls config without an explicit provider." \
    "Use builder_with_provider with a crypto::tls provider."
done < <(grep -nE '(Client|Server)Config::builder\(\)|builder_with_details\(|builder_with_protocol_versions\(' "${sources[@]}" 2>/dev/null || true)

# No provider assembly, backend defaults, provider mutation, or FIPS escape hatches.
while IFS=: read -r file line text; do
  [[ -z "${file:-}" ]] && continue
  # jsonwebtoken installs its own JWT provider; it does not build rustls configs.
  grep -q 'jsonwebtoken::crypto' <<< "${text}" && continue
  report "${file}:${line} obtains a provider outside crypto::tls or alters a FIPS precondition." \
    "Providers must come from crypto::tls; require_ems and ECH must not be set."
done < <(grep -nE '(^[[:space:]]*|[=(:,][[:space:]]*|return[[:space:]]+)CryptoProvider[[:space:]]*\{|::default_provider\(\)|install_default\(|CryptoProvider::get_default|\.(cipher_suites|kx_groups|signature_verification_algorithms|secure_random|key_provider)[[:space:]]*=[^=]|require_ems|ech_mode|with_ech' "${sources[@]}" 2>/dev/null || true)

if [[ ${status} -eq 0 ]]; then
  # Config builders only: verifier builders also match the bare call.
  sites=$(grep -cE '(Client|Server)Config::builder_with_provider\(' "${sources[@]}" 2>/dev/null | awk -F: '{n+=$2} END {print n+0}')
  echo "Verified ${sites} rustls config sites: no provider bypasses or FIPS escape hatches outside crypto::tls."
fi
exit ${status}
