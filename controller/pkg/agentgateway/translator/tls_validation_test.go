package translator

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/pem"
	"math/big"
	"sync"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func TestTLSValidationCacheKey(t *testing.T) {
	base := &TLSInfo{
		Cert:   []byte("certificate"),
		Key:    []byte("private key"),
		CaCert: []byte("CA certificate"),
	}
	baseKey := newTLSValidationCacheKey(base)

	assert.Equal(t, baseKey, newTLSValidationCacheKey(&TLSInfo{
		Cert:   append([]byte(nil), base.Cert...),
		Key:    append([]byte(nil), base.Key...),
		CaCert: append([]byte(nil), base.CaCert...),
	}))
	assert.NotEqual(t, baseKey, newTLSValidationCacheKey(&TLSInfo{
		Cert:   []byte("different certificate"),
		Key:    base.Key,
		CaCert: base.CaCert,
	}))
	assert.NotEqual(t, baseKey, newTLSValidationCacheKey(&TLSInfo{
		Cert:   base.Cert,
		Key:    []byte("different private key"),
		CaCert: base.CaCert,
	}))
	assert.NotEqual(t, baseKey, newTLSValidationCacheKey(&TLSInfo{
		Cert:   base.Cert,
		Key:    base.Key,
		CaCert: []byte("different CA certificate"),
	}))
	assert.NotEqual(t,
		newTLSValidationCacheKey(&TLSInfo{Cert: base.Cert, Key: base.Key}),
		newTLSValidationCacheKey(&TLSInfo{Cert: base.Cert, Key: base.Key, CaCert: []byte{}}),
	)
}

func TestTLSValidatorCachesValidAndInvalidResults(t *testing.T) {
	validator := newTLSValidator(2)
	valid := newValidTLSInfo(t)

	assert.Nil(t, validator.validate(valid))
	assert.Nil(t, validator.validate(valid))
	assert.Equal(t, 1, validator.cache.Len())

	invalid := &TLSInfo{Cert: []byte("invalid"), Key: []byte("invalid")}
	firstErr := validator.validate(invalid)
	require.NotNil(t, firstErr)
	originalMessage := firstErr.Message
	firstErr.Message = "mutated by caller"

	secondErr := validator.validate(invalid)
	require.NotNil(t, secondErr)
	assert.Equal(t, originalMessage, secondErr.Message)
	assert.NotSame(t, firstErr, secondErr)
	assert.Equal(t, 2, validator.cache.Len())
}

func TestTLSValidatorReturnsCachedResultWithoutRevalidating(t *testing.T) {
	validator := newTLSValidator(1)
	invalid := &TLSInfo{Cert: []byte("invalid"), Key: []byte("invalid")}
	validator.cache.Add(newTLSValidationCacheKey(invalid), tlsValidationResult{})

	assert.Nil(t, validator.validate(invalid))
}

func TestTLSValidatorBypassesWorkloadCertificates(t *testing.T) {
	tests := []struct {
		name    string
		tlsInfo *TLSInfo
	}{
		{
			name:    "Istio workload certificate",
			tlsInfo: &TLSInfo{IstioWorkloadCert: true},
		},
		{
			name:    "SPIFFE certificate",
			tlsInfo: &TLSInfo{Spiffe: true},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			validator := newTLSValidator(1)
			assert.Nil(t, validator.validate(tt.tlsInfo))
			assert.Zero(t, validator.cache.Len())
		})
	}
}

func TestTLSValidatorEvictsLeastRecentlyUsedResult(t *testing.T) {
	validator := newTLSValidator(2)
	first := &TLSInfo{Cert: []byte("first"), Key: []byte("key")}
	second := &TLSInfo{Cert: []byte("second"), Key: []byte("key")}
	third := &TLSInfo{Cert: []byte("third"), Key: []byte("key")}

	validator.validate(first)
	validator.validate(second)
	validator.validate(first)
	validator.validate(third)

	assert.True(t, validator.cache.Contains(newTLSValidationCacheKey(first)))
	assert.False(t, validator.cache.Contains(newTLSValidationCacheKey(second)))
	assert.True(t, validator.cache.Contains(newTLSValidationCacheKey(third)))
	assert.Equal(t, 2, validator.cache.Len())
}

func TestTLSValidatorConcurrentAccess(t *testing.T) {
	validator := newTLSValidator(2)
	valid := newValidTLSInfo(t)
	const goroutines = 100

	results := make(chan *ConfigError, goroutines)
	var wg sync.WaitGroup
	for range goroutines {
		wg.Go(func() {
			results <- validator.validate(valid)
		})
	}
	wg.Wait()
	close(results)

	for result := range results {
		assert.Nil(t, result)
	}
	assert.Equal(t, 1, validator.cache.Len())
}

func BenchmarkValidateTLS(b *testing.B) {
	tlsInfo := newValidTLSInfo(b)

	b.Run("uncached", func(b *testing.B) {
		for b.Loop() {
			if err := validateTLSUncached(tlsInfo); err != nil {
				b.Fatal(err.Message)
			}
		}
	})

	validator := newTLSValidator(1)
	require.Nil(b, validator.validate(tlsInfo))
	b.Run("cached", func(b *testing.B) {
		for b.Loop() {
			if err := validator.validate(tlsInfo); err != nil {
				b.Fatal(err.Message)
			}
		}
	})
}

func newValidTLSInfo(t testing.TB) *TLSInfo {
	t.Helper()

	publicKey, privateKey, err := ed25519.GenerateKey(rand.Reader)
	require.NoError(t, err)
	template := &x509.Certificate{
		SerialNumber: big.NewInt(1),
		Subject:      pkix.Name{CommonName: "example.test"},
		NotBefore:    time.Unix(0, 0),
		NotAfter:     time.Unix(1<<31, 0),
		KeyUsage:     x509.KeyUsageDigitalSignature,
	}
	certDER, err := x509.CreateCertificate(rand.Reader, template, template, publicKey, privateKey)
	require.NoError(t, err)
	keyDER, err := x509.MarshalPKCS8PrivateKey(privateKey)
	require.NoError(t, err)

	return &TLSInfo{
		Cert: pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: certDER}),
		Key:  pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: keyDER}),
	}
}
