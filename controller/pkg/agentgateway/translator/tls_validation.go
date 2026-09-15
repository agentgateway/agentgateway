package translator

import (
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"fmt"

	lru "github.com/hashicorp/golang-lru/v2"
)

// tlsValidationCacheCapacity bounds cache memory while allowing up to 1024
// distinct, referenced TLS inputs to avoid repeated validation.
const tlsValidationCacheCapacity = 1024

type tlsValidationCacheKey struct {
	cert   [sha256.Size]byte
	key    [sha256.Size]byte
	caCert [sha256.Size]byte
	// CA presence controls whether CA validation is attempted, so nil and empty differ.
	caCertPresent bool
}

// tlsValidationResult stores values rather than a ConfigError pointer so callers
// cannot mutate the result retained by the cache.
type tlsValidationResult struct {
	invalid bool
	reason  ConfigErrorReason
	message string
}

// tlsValidator lazily caches validation results by content. Certificate rotation
// produces a different key and therefore requires no explicit invalidation.
type tlsValidator struct {
	cache *lru.Cache[tlsValidationCacheKey, tlsValidationResult]
}

var defaultTLSValidator = newTLSValidator(tlsValidationCacheCapacity)

func newTLSValidator(capacity int) *tlsValidator {
	cache, err := lru.New[tlsValidationCacheKey, tlsValidationResult](capacity)
	if err != nil {
		panic(err)
	}
	return &tlsValidator{cache: cache}
}

func validateTLS(certInfo *TLSInfo) *ConfigError {
	return defaultTLSValidator.validate(certInfo)
}

func (v *tlsValidator) validate(certInfo *TLSInfo) *ConfigError {
	if certInfo.IstioWorkloadCert || certInfo.Spiffe {
		return nil
	}

	key := newTLSValidationCacheKey(certInfo)
	if result, found := v.cache.Get(key); found {
		return result.configError()
	}

	result := newTLSValidationResult(validateTLSUncached(certInfo))
	v.cache.Add(key, result)
	return result.configError()
}

func newTLSValidationCacheKey(certInfo *TLSInfo) tlsValidationCacheKey {
	// Separate hashes preserve field boundaries and avoid retaining sensitive bytes.
	return tlsValidationCacheKey{
		cert:          sha256.Sum256(certInfo.Cert),
		key:           sha256.Sum256(certInfo.Key),
		caCert:        sha256.Sum256(certInfo.CaCert),
		caCertPresent: certInfo.CaCert != nil,
	}
}

func newTLSValidationResult(err *ConfigError) tlsValidationResult {
	if err == nil {
		return tlsValidationResult{}
	}
	return tlsValidationResult{
		invalid: true,
		reason:  err.Reason,
		message: err.Message,
	}
}

func (r tlsValidationResult) configError() *ConfigError {
	if !r.invalid {
		return nil
	}
	return &ConfigError{
		Reason:  r.reason,
		Message: r.message,
	}
}

func validateTLSUncached(certInfo *TLSInfo) *ConfigError {
	if _, err := tls.X509KeyPair(certInfo.Cert, certInfo.Key); err != nil {
		return &ConfigError{
			Reason:  InvalidTLS,
			Message: fmt.Sprintf("invalid certificate reference, the certificate is malformed: %v", err),
		}
	}
	if certInfo.CaCert != nil {
		if !x509.NewCertPool().AppendCertsFromPEM(certInfo.Cert) {
			return &ConfigError{
				Reason:  InvalidTLSCA,
				Message: fmt.Sprintf("invalid CA certificate reference, the bundle is malformed"),
			}
		}
	}
	return nil
}
