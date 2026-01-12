/**
 * TLS helpers shared across the UI.
 *
 * Keep these lists in sync with backend allowlist + parsing:
 * - `crates/agentgateway/src/transport/tls.rs` (`ALL_CIPHER_SUITES`, `cipher_suites_from_names`)
 * - `crates/agentgateway/proto/resource.proto` (`TLSConfig.CipherSuite`)
 */

export const TLS13_CIPHER_SUITES = [
  "TLS_AES_256_GCM_SHA384",
  "TLS_AES_128_GCM_SHA256",
  "TLS_CHACHA20_POLY1305_SHA256",
] as const;

export const TLS12_CIPHER_SUITES = [
  "TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384",
  "TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256",
  "TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256",
  "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384",
  "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256",
  "TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256",
] as const;

export const ALL_CIPHER_SUITES = [...TLS13_CIPHER_SUITES, ...TLS12_CIPHER_SUITES] as const;
