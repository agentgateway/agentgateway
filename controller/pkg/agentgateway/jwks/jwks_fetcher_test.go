package jwks

import (
	"encoding/json"
	"fmt"
	"testing"
	"time"

	"github.com/go-jose/go-jose/v4"
	"github.com/golang/mock/gomock"
	"github.com/stretchr/testify/assert"

	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/jwks/mocks"
)

func TestAddKeysetToFetcher(t *testing.T) {
	expectedKeysetSource := JwksSource{JwksURL: "https://test/jwks", Ttl: 5 * time.Minute, Deleted: false}

	f := NewJwksFetcher(NewJwksCache())
	f.AddOrUpdateKeyset(expectedKeysetSource)

	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.Equal(t, *fetch.keysetSource, expectedKeysetSource)
	assert.WithinDuration(t, time.Now(), fetch.at, 2*time.Second)
	assert.Equal(t, *f.keysetSources["https://test/jwks"], expectedKeysetSource)
}

func TestRemoveKeysetFromFetcher(t *testing.T) {
	f := NewJwksFetcher(NewJwksCache())

	f.AddOrUpdateKeyset(JwksSource{JwksURL: "https://test/jwks", Ttl: 5 * time.Minute})
	keysetSource := f.keysetSources["https://test/jwks"]
	assert.NotNil(t, keysetSource)
	f.cache.jwks["https://test/jwks"] = "jwks"

	f.RemoveKeyset(JwksSource{JwksURL: "https://test/jwks"})
	assert.NotContains(t, f.keysetSources, "https://test/jwks")
	assert.NotContains(t, f.cache.jwks, "https://test/jwks")
	assert.True(t, keysetSource.Deleted)
}

func TestFetcherWithEmptyJwksFetchSchedule(t *testing.T) {
	ctx := t.Context()

	f := NewJwksFetcher(NewJwksCache())
	updates := f.SubscribeToUpdates()
	go f.maybeFetchJwks(ctx)

	assert.Never(t, func() bool {
		select {
		case <-updates:
			return true
		default:
			return false
		}
	}, 1*time.Second, 100*time.Millisecond)
}

func TestSuccessfulJwksFetch(t *testing.T) {
	ctx := t.Context()

	f := NewJwksFetcher(NewJwksCache())
	ctrl := gomock.NewController(t)
	jwksClient := mocks.NewMockJwksHttpClient(ctrl)
	f.defaultJwksClient = jwksClient

	f.AddOrUpdateKeyset(JwksSource{JwksURL: "https://test/jwks", Ttl: 5 * time.Minute})
	updates := f.SubscribeToUpdates()

	expectedJwks := jose.JSONWebKeySet{}
	err := json.Unmarshal(([]byte)(jwks), &expectedJwks)
	assert.NoError(t, err)

	jwksClient.EXPECT().
		FetchJwks(gomock.Any(), gomock.Eq("https://test/jwks")).
		Return(expectedJwks, nil)
	go f.maybeFetchJwks(ctx)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		select {
		case actual := <-updates:
			cache := NewJwksCache()
			assert.NoError(c, cache.LoadJwksFromStores(actual))
			assert.Equal(c, jwks, cache.jwks["https://test/jwks"])
		default:
			assert.Fail(c, "no updates")
		}
	}, 2*time.Second, 100*time.Millisecond)

	f.mu.Lock()
	defer f.mu.Unlock()
	// check that we scheduled next fetch
	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.WithinDuration(t, time.Now().Add(5*time.Minute), fetch.at, 3*time.Second)
}

// jwks were fetched, but there were no changes to keysets
// we still notify subscribers that a fetch happened (we always sync jwks to ConfigMaps)
func TestSuccessfulJwksFetchButNoChanges(t *testing.T) {
	ctx := t.Context()

	f := NewJwksFetcher(NewJwksCache())
	ctrl := gomock.NewController(t)
	jwksClient := mocks.NewMockJwksHttpClient(ctrl)
	f.defaultJwksClient = jwksClient

	f.AddOrUpdateKeyset(JwksSource{JwksURL: "https://test/jwks", Ttl: 5 * time.Minute})
	f.cache.jwks["https://test/jwks"] = jwks
	updates := f.SubscribeToUpdates()

	existingJwks := jose.JSONWebKeySet{}
	err := json.Unmarshal(([]byte)(jwks), &existingJwks)
	assert.NoError(t, err)

	jwksClient.EXPECT().
		FetchJwks(gomock.Any(), gomock.Eq("https://test/jwks")).
		Return(existingJwks, nil)
	go f.maybeFetchJwks(ctx)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		select {
		case actual := <-updates:
			cache := NewJwksCache()
			assert.NoError(c, cache.LoadJwksFromStores(actual))
			assert.Equal(c, jwks, cache.jwks["https://test/jwks"])
		default:
			assert.Fail(c, "no updates")
		}
	}, 2*time.Second, 100*time.Millisecond)

	f.mu.Lock()
	defer f.mu.Unlock()
	// check that we scheduled next fetch
	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.WithinDuration(t, time.Now().Add(5*time.Minute), fetch.at, 3*time.Second)
}

func TestFetchJwksWithError(t *testing.T) {
	ctx := t.Context()

	f := NewJwksFetcher(NewJwksCache())
	ctrl := gomock.NewController(t)
	jwksClient := mocks.NewMockJwksHttpClient(ctrl)
	f.defaultJwksClient = jwksClient

	f.AddOrUpdateKeyset(JwksSource{JwksURL: "https://test/jwks", Ttl: 5 * time.Minute})
	updates := f.SubscribeToUpdates()

	jwksClient.EXPECT().
		FetchJwks(gomock.Any(), gomock.Eq("https://test/jwks")).
		Return(jose.JSONWebKeySet{}, fmt.Errorf("boom!"))
	go f.maybeFetchJwks(ctx)

	assert.Never(t, func() bool {
		select {
		case <-updates:
			return true
		default:
			return false
		}
	}, 1*time.Second, 100*time.Millisecond)

	f.mu.Lock()
	defer f.mu.Unlock()
	// check that we scheduled a retry
	retry := f.schedule.Peek()
	assert.NotNil(t, retry)
	assert.WithinDuration(t, time.Now().Add(5*time.Second), retry.at, 2*time.Second)
	assert.Equal(t, retry.retryAttempt, 1)
	assert.Equal(t, retry.keysetSource.JwksURL, "https://test/jwks")
}

var jwks = `{"keys":[{"use":"sig","kty":"RSA","kid":"JWxVLtipR-Q6wF2zmQKEoxbFhqwibK2aKNLyRqNxdj4","alg":"RS256","n":"5ApthhEwr6U00Coa0_572OytJXbVZKgl-myirM2m4GSrVfaKus41GEPHHXMzyGDPgHU7Rb4o0yzB-obkgz0zo2jnjv1zSx88BgdhhdE0BX2ULFDj67jVYdFZdCOoBr1_xJ5LEjQArHxfywZxW4a0egc3JaIwo-3qSSlRnD1KV2uzTG9FoDpvJLn1ZzdMgoTHuxIMla6WdgPDswVD8nrQM0I_1VGyGC0l2dICUEiqN0QrZen--U70J6EU6hd8vi_9qmALhjoSEASH2Z2sHco4Shv_aVx0BM-zN5UJWz4VF51Ag_KgcePS5Co7iVM0FUwMNWauWhPDPLWiXoUJvUWVPw","e":"AQAB","x5c":["MIICozCCAYsCBgGYyKDydjANBgkqhkiG9w0BAQsFADAVMRMwEQYDVQQDDAprYWdlbnQtZGV2MB4XDTI1MDgyMDE3NTU0N1oXDTM1MDgyMDE3NTcyN1owFTETMBEGA1UEAwwKa2FnZW50LWRldjCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAOQKbYYRMK+lNNAqGtP+e9jsrSV21WSoJfpsoqzNpuBkq1X2irrONRhDxx1zM8hgz4B1O0W+KNMswfqG5IM9M6No5479c0sfPAYHYYXRNAV9lCxQ4+u41WHRWXQjqAa9f8SeSxI0AKx8X8sGcVuGtHoHNyWiMKPt6kkpUZw9Sldrs0xvRaA6byS59Wc3TIKEx7sSDJWulnYDw7MFQ/J60DNCP9VRshgtJdnSAlBIqjdEK2Xp/vlO9CehFOoXfL4v/apgC4Y6EhAEh9mdrB3KOEob/2lcdATPszeVCVs+FRedQIPyoHHj0uQqO4lTNBVMDDVmrloTwzy1ol6FCb1FlT8CAwEAATANBgkqhkiG9w0BAQsFAAOCAQEAxElyp6gak62xC3yEw0nRUZNI0nsu0Oeow8ZwbmfTSa2hRKFQQe2sjMzm6L4Eyg2IInVn0spkw9BVJ07i8mDmvChjRNra7t6CX1dIykUUtxtNwglX0YBRjMl/heG7dC/dyDRVW6EUrPopMQ9QibzmH5XOBLDanTfK6tPwe5ezG5JF3JCx2Z3dtmAMtpCp7Nnr/gj48z7j4V8EHSB8hgITHBPcLOmiVglS3LF2/D+PK6efRWnVaDtcPmuh/0JmdmKxwJcvvuZD7tp5UFRbw9cgx5Pvv+mOWVCp/E2L+P17Gu0C/MC4Wnbn3Pi6Tgt0GNUMngCCyBnfcTpljUddW6Kheg=="],"x5t":"SmEthIFV9ehf3ggduek6QLfXxyU","x5t#S256":"XNGenWvGVC_sxSOTW0j_d7zwQlbGzkFj5XGCgPrLNJA"},{"use":"enc","kty":"RSA","kid":"hb2m-EP6nG_ktqHJOna_rnadxRaOtzArOecAJlNSmqU","alg":"RSA-OAEP","n":"xYU8uN6rXI6l6LAQ5inpylE4qiFqshbV92VnPrUO8gNff_TuZjvq19f0zXpVnnu88bCL5Q6DjRqRP4a2brAsYYBjSjwKGF3dd7jda6uavU1br2NFppZ6GSisOlKuKqMAUitQuYgAzYP-E2FasQOskrZ8HQ8S8hff7rNZH84VL5lNwTMHiwL1O8jBmxJE-ABM0To-2a9YosRkRa_uVzY720lSAir1UNiUSR1PypS2ixWyO04AVMJf8JgYU8rsUHNkZenYSRySzYzIxE57RCYnuZoc1hSVBtN2cFXXSqTwGMI7tfzTAtG11Z7zkiWmP0Tk7xabh5xfdXhZtJfHT6id5w","e":"AQAB","x5c":["MIICozCCAYsCBgGYyKD0zDANBgkqhkiG9w0BAQsFADAVMRMwEQYDVQQDDAprYWdlbnQtZGV2MB4XDTI1MDgyMDE3NTU0OFoXDTM1MDgyMDE3NTcyOFowFTETMBEGA1UEAwwKa2FnZW50LWRldjCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAMWFPLjeq1yOpeiwEOYp6cpROKoharIW1fdlZz61DvIDX3/07mY76tfX9M16VZ57vPGwi+UOg40akT+Gtm6wLGGAY0o8Chhd3Xe43Wurmr1NW69jRaaWehkorDpSriqjAFIrULmIAM2D/hNhWrEDrJK2fB0PEvIX3+6zWR/OFS+ZTcEzB4sC9TvIwZsSRPgATNE6PtmvWKLEZEWv7lc2O9tJUgIq9VDYlEkdT8qUtosVsjtOAFTCX/CYGFPK7FBzZGXp2Ekcks2MyMROe0QmJ7maHNYUlQbTdnBV10qk8BjCO7X80wLRtdWe85Ilpj9E5O8Wm4ecX3V4WbSXx0+onecCAwEAATANBgkqhkiG9w0BAQsFAAOCAQEAWuRnoKtKhCqLaz3Ze2q8hRykke7JwNrNxqDPn7eToa1MKsfsrtE678kzXhnfdivK/1F/8dr7Thn/WX7ZUJW2jsmbP1sCJjK02yY2setJ1jJKvJZcib8y7LAsqoACYZ4FM/KLrdywGn7KSenqWCLRMqeT04dWlmJexEszb5fgCKCFIZLKjaGJZIuLhsJBLyYHEVFpacr69cZ/ZjNpshHIiV0l/I434vcW39S9+uMfxf1glLTEPifmwK4gMRem3QQLqK21vBcjuS0GBQXQinaztcNaiu1invyTZd5s+3u5yORsip6YhbGhe08TbbtN7yLlZFITDQL4oFrXVGXX+4dp8w=="],"x5t":"BMlhx-2TUdiyftY8aR_zt7xECEI","x5t#S256":"YTTj8SxySpGgVFl5ZQqniLPnmg0gWHgBhissHXQCZ8k"}]}`
