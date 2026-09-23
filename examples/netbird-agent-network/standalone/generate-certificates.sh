#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "${SCRIPT_DIR}"

if [[ ! -f .env ]]; then
  echo "copy env.example to .env and update it first" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1091
source .env
set +a

if ! command -v openssl >/dev/null 2>&1; then
  echo "required command not found: openssl" >&2
  exit 1
fi

for variable in NETBIRD_MANAGEMENT_DOMAIN NETBIRD_PROXY_DOMAIN; do
  if [[ -z "${!variable:-}" || "${!variable}" == *example.com ]]; then
    echo "set ${variable} in .env to a hostname you control" >&2
    exit 1
  fi
done

CERT_DIR=runtime/certs
if [[ -f ${CERT_DIR}/ca.crt && -f ${CERT_DIR}/management/tls.crt && \
      -f ${CERT_DIR}/management/tls.key && -f ${CERT_DIR}/proxy/tls.crt && \
      -f ${CERT_DIR}/proxy/tls.key ]]; then
  if ! openssl verify -CAfile "${CERT_DIR}/ca.crt" \
      "${CERT_DIR}/management/tls.crt" "${CERT_DIR}/proxy/tls.crt" \
      >/dev/null 2>&1 || \
    ! openssl x509 -in "${CERT_DIR}/management/tls.crt" -noout \
      -checkhost "${NETBIRD_MANAGEMENT_DOMAIN}" >/dev/null 2>&1 || \
    ! openssl x509 -in "${CERT_DIR}/proxy/tls.crt" -noout \
      -checkhost "endpoint.${NETBIRD_PROXY_DOMAIN}" >/dev/null 2>&1; then
    echo "existing certificates do not cover the configured domains" >&2
    echo "remove ${CERT_DIR} or install matching certificate files" >&2
    exit 1
  fi
  echo "Reusing certificates in ${CERT_DIR}. Remove the directory to regenerate them."
  exit 0
fi

mkdir -p "${CERT_DIR}/management" "${CERT_DIR}/proxy"
chmod 700 "${CERT_DIR}" "${CERT_DIR}/management" "${CERT_DIR}/proxy"

openssl req -x509 -newkey rsa:3072 -sha256 -nodes -days 3650 \
  -subj "/CN=NetBird agentgateway example CA" \
  -keyout "${CERT_DIR}/ca.key" -out "${CERT_DIR}/ca.crt" >/dev/null 2>&1

issue_certificate() {
  local name=$1
  local common_name=$2
  local san=$3
  local directory="${CERT_DIR}/${name}"

  openssl req -newkey rsa:3072 -sha256 -nodes \
    -subj "/CN=${common_name}" \
    -addext "subjectAltName=DNS:${san}" \
    -keyout "${directory}/tls.key" \
    -out "${directory}/tls.csr" >/dev/null 2>&1
  openssl x509 -req -sha256 -days 365 \
    -in "${directory}/tls.csr" \
    -CA "${CERT_DIR}/ca.crt" -CAkey "${CERT_DIR}/ca.key" -CAcreateserial \
    -copy_extensions copyall -out "${directory}/leaf.crt" >/dev/null 2>&1
  cp "${directory}/leaf.crt" "${directory}/tls.crt"
  rm "${directory}/tls.csr" "${directory}/leaf.crt"
}

issue_certificate management "${NETBIRD_MANAGEMENT_DOMAIN}" \
  "${NETBIRD_MANAGEMENT_DOMAIN}"
issue_certificate proxy "*.${NETBIRD_PROXY_DOMAIN}" \
  "*.${NETBIRD_PROXY_DOMAIN}"

chmod 600 "${CERT_DIR}/ca.key" \
  "${CERT_DIR}/management/tls.key" "${CERT_DIR}/proxy/tls.key"
chmod 644 "${CERT_DIR}/ca.crt" \
  "${CERT_DIR}/management/tls.crt" "${CERT_DIR}/proxy/tls.crt"

echo "Generated a demo CA and one-year server certificates in ${CERT_DIR}."
