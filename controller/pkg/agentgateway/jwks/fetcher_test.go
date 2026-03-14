package jwks

import (
	"context"
	"encoding/json"
	"fmt"
	"testing"
	"time"

	"github.com/go-jose/go-jose/v4"
	"github.com/stretchr/testify/assert"
)

func TestAddKeysetToFetcher(t *testing.T) {
	expected := testSource()

	f := newFetcher(newCache())
	assert.NoError(t, f.AddOrUpdateKeyset(expected))

	f.mu.Lock()
	defer f.mu.Unlock()

	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.Equal(t, expected.RequestKey, fetch.requestKey)
	state, ok := f.requests[expected.RequestKey]
	assert.True(t, ok)
	assert.Equal(t, expected, state.source)
	assert.Len(t, f.schedule, 1)
}

func TestRemoveKeysetFromFetcher(t *testing.T) {
	source := testSource()
	f := newFetcher(newCache())

	assert.NoError(t, f.AddOrUpdateKeyset(source))
	f.cache.artifacts[source.RequestKey] = Artifact{RequestKey: source.RequestKey, URL: source.Request.URL, JwksJSON: "jwks"}

	f.RemoveKeyset(source.RequestKey)

	f.mu.Lock()
	_, ok := f.requests[source.RequestKey]
	assert.Empty(t, f.schedule)
	f.mu.Unlock()
	assert.False(t, ok)
	_, ok = f.cache.GetJwks(source.RequestKey)
	assert.False(t, ok)
}

func TestAddOrUpdateKeysetReplacesExistingScheduleEntry(t *testing.T) {
	f := newFetcher(newCache())
	source := testSource()

	assert.NoError(t, f.AddOrUpdateKeyset(source))
	assert.NoError(t, f.AddOrUpdateKeyset(source))

	f.mu.Lock()
	defer f.mu.Unlock()

	assert.Len(t, f.schedule, 1)
	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.Equal(t, source.RequestKey, fetch.requestKey)
	assert.Equal(t, uint64(2), fetch.generation)
}

func TestFetcherWithEmptyJwksFetchSchedule(t *testing.T) {
	ctx := t.Context()

	f := newFetcher(newCache())
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

	f := newFetcher(newCache())
	source := testSource()
	assert.NoError(t, f.AddOrUpdateKeyset(source))
	updates := f.SubscribeToUpdates()

	expectedJwks := jose.JSONWebKeySet{}
	err := json.Unmarshal([]byte(sampleJWKS), &expectedJwks)
	assert.NoError(t, err)

	f.defaultJwksClient = stubJwksClient{
		t:           t,
		expectedReq: source.Request,
		result:      expectedJwks,
	}
	go f.maybeFetchJwks(ctx)

	assert.EventuallyWithT(t, func(c *assert.CollectT) {
		select {
		case actual := <-updates:
			_, ok := actual[source.RequestKey]
			assert.True(c, ok)
			artifact, ok := f.cache.GetJwks(source.RequestKey)
			assert.True(c, ok)
			assert.Equal(c, sampleJWKS, artifact.JwksJSON)
		default:
			assert.Fail(c, "no updates")
		}
	}, 2*time.Second, 100*time.Millisecond)

	f.mu.Lock()
	defer f.mu.Unlock()
	fetch := f.schedule.Peek()
	assert.NotNil(t, fetch)
	assert.WithinDuration(t, time.Now().Add(5*time.Minute), fetch.at, 3*time.Second)
}

func TestFetchJwksWithError(t *testing.T) {
	ctx := t.Context()

	f := newFetcher(newCache())
	source := testSource()
	assert.NoError(t, f.AddOrUpdateKeyset(source))
	updates := f.SubscribeToUpdates()

	f.defaultJwksClient = stubJwksClient{
		t:           t,
		expectedReq: source.Request,
		err:         fmt.Errorf("boom!"),
	}
	go f.maybeFetchJwks(ctx)

	assert.Never(t, func() bool {
		select {
		case <-updates:
			return true
		default:
			return false
		}
	}, 250*time.Millisecond, 10*time.Millisecond)

	f.mu.Lock()
	defer f.mu.Unlock()
	retry := f.schedule.Peek()
	assert.NotNil(t, retry)
	assert.WithinDuration(t, time.Now().Add(200*time.Millisecond), retry.at, 2*time.Second)
	assert.Equal(t, 1, retry.retryAttempt)
	assert.Equal(t, source.RequestKey, retry.requestKey)
}

func TestFetcherDiscardedFetchDoesNotRepopulateRemovedKeyset(t *testing.T) {
	ctx := t.Context()

	f := newFetcher(newCache())
	source := testSource()
	assert.NoError(t, f.AddOrUpdateKeyset(source))

	expectedJwks := jose.JSONWebKeySet{}
	err := json.Unmarshal([]byte(sampleJWKS), &expectedJwks)
	assert.NoError(t, err)

	started := make(chan struct{})
	release := make(chan struct{})
	f.defaultJwksClient = stubJwksClient{
		t:           t,
		expectedReq: source.Request,
		result:      expectedJwks,
		started:     started,
		release:     release,
	}

	done := make(chan struct{})
	go func() {
		f.maybeFetchJwks(ctx)
		close(done)
	}()

	<-started
	f.RemoveKeyset(source.RequestKey)
	close(release)
	<-done

	_, ok := f.cache.GetJwks(source.RequestKey)
	assert.False(t, ok)
}

func TestNotifySubscribersMergesPendingRequestKeyUpdates(t *testing.T) {
	f := newFetcher(newCache())
	updates := f.SubscribeToUpdates()
	first := testSource()
	second := testSourceWithURL("https://test/other-jwks")

	f.notifySubscribers(map[RequestKey]struct{}{first.RequestKey: {}})
	f.notifySubscribers(map[RequestKey]struct{}{second.RequestKey: {}})

	actual := <-updates
	_, hasFirst := actual[first.RequestKey]
	_, hasSecond := actual[second.RequestKey]
	assert.True(t, hasFirst)
	assert.True(t, hasSecond)
}

func TestNextRetryDelayCapsWithoutOverflow(t *testing.T) {
	assert.Equal(t, 200*time.Millisecond, nextRetryDelay(0))
	assert.Equal(t, maxRetryDelay, nextRetryDelay(7))
	assert.Equal(t, maxRetryDelay, nextRetryDelay(36))
}

func testSource() JwksSource {
	return testSourceWithURL("https://test/jwks")
}

func testSourceWithURL(requestURL string) JwksSource {
	request := Request{URL: requestURL}
	return JwksSource{
		OwnerKey: JwksOwnerID{
			Kind:      OwnerKindPolicy,
			Namespace: "default",
			Name:      "test",
			Path:      "spec.traffic.jwtAuthentication.providers[0].jwks.remote",
		},
		RequestKey: request.Key(),
		Request:    request,
		TTL:        5 * time.Minute,
	}
}

type stubJwksClient struct {
	t           *testing.T
	expectedReq Request
	result      jose.JSONWebKeySet
	err         error
	started     chan<- struct{}
	release     <-chan struct{}
}

func (s stubJwksClient) FetchJwks(_ context.Context, req Request) (jose.JSONWebKeySet, error) {
	assert.Equal(s.t, s.expectedReq, req)
	if s.started != nil {
		close(s.started)
	}
	if s.release != nil {
		<-s.release
	}
	return s.result, s.err
}

var sampleJWKS = `{"keys":[{"use":"sig","kty":"RSA","kid":"JWxVLtipR-Q6wF2zmQKEoxbFhqwibK2aKNLyRqNxdj4","alg":"RS256","n":"5ApthhEwr6U00Coa0_572OytJXbVZKgl-myirM2m4GSrVfaKus41GEPHHXMzyGDPgHU7Rb4o0yzB-obkgz0zo2jnjv1zSx88BgdhhdE0BX2ULFDj67jVYdFZdCOoBr1_xJ5LEjQArHxfywZxW4a0egc3JaIwo-3qSSlRnD1KV2uzTG9FoDpvJLn1ZzdMgoTHuxIMla6WdgPDswVD8nrQM0I_1VGyGC0l2dICUEiqN0QrZen--U70J6EU6hd8vi_9qmALhjoSEASH2Z2sHco4Shv_aVx0BM-zN5UJWz4VF51Ag_KgcePS5Co7iVM0FUwMNWauWhPDPLWiXoUJvUWVPw","e":"AQAB","x5c":["MIICozCCAYsCBgGYyKDydjANBgkqhkiG9w0BAQsFADAVMRMwEQYDVQQDDAprYWdlbnQtZGV2MB4XDTI1MDgyMDE3NTU0N1oXDTM1MDgyMDE3NTcyN1owFTETMBEGA1UEAwwKa2FnZW50LWRldjCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAOQKbYYRMK+lNNAqGtP+e9jsrSV21WSoJfpsoqzNpuBkq1X2irrONRhDxx1zM8hgz4B1O0W+KNMswfqG5IM9M6No5479c0sfPAYHYYXRNAV9lCxQ4+u41WHRWXQjqAa9f8SeSxI0AKx8X8sGcVuGtHoHNyWiMKPt6kkpUZw9Sldrs0xvRaA6byS59Wc3TIKEx7sSDJWulnYDw7MFQ/J60DNCP9VRshgtJdnSAlBIqjdEK2Xp/vlO9CehFOoXfL4v/apgC4Y6EhAEh9mdrB3KOEob/2lcdATPszeVCVs+FRedQIPyoHHj0uQqO4lTNBVMDDVmrloTwzy1ol6FCb1FlT8CAwEAATANBgkqhkiG9w0BAQsFAAOCAQEAxElyp6gak62xC3yEw0nRUZNI0nsu0Oeow8ZwbmfTSa2hRKFQQe2sjMzm6L4Eyg2IInVn0spkw9BVJ07i8mDmvChjRNra7t6CX1dIykUUtxtNwglX0YBRjMl/heG7dC/dyDRVW6EUrPopMQ9QibzmH5XOBLDanTfK6tPwe5ezG5JF3JCx2Z3dtmAMtpCp7Nnr/gj48z7j4V8EHSB8hgITHBPcLOmiVglS3LF2/D+PK6efRWnVaDtcPmuh/0JmdmKxwJcvvuZD7tp5UFRbw9cgx5Pvv+mOWVCp/E2L+P17Gu0C/MC4Wnbn3Pi6Tgt0GNUMngCCyBnfcTpljUddW6Kheg=="],"x5t":"SmEthIFV9ehf3ggduek6QLfXxyU","x5t#S256":"XNGenWvGVC_sxSOTW0j_d7zwQlbGzkFj5XGCgPrLNJA"},{"use":"enc","kty":"RSA","kid":"hb2m-EP6nG_ktqHJOna_rnadxRaOtzArOecAJlNSmqU","alg":"RSA-OAEP","n":"xYU8uN6rXI6l6LAQ5inpylE4qiFqshbV92VnPrUO8gNff_TuZjvq19f0zXpVnnu88bCL5Q6DjRqRP4a2brAsYYBjSjwKGF3dd7jda6uavU1br2NFppZ6GSisOlKuKqMAUitQuYgAzYP-E2FasQOskrZ8HQ8S8hff7rNZH84VL5lNwTMHiwL1O8jBmxJE-ABM0To-2a9YosRkRa_uVzY720lSAir1UNiUSR1PypS2ixWyO04AVMJf8JgYU8rsUHNkZenYSRySzYzIxE57RCYnuZoc1hSVBtN2cFXXSqTwGMI7tfzTAtG11Z7zkiWmP0Tk7xabh5xfdXhZtJfHT6id5w","e":"AQAB","x5c":["MIICozCCAYsCBgGYyKD0zDANBgkqhkiG9w0BAQsFADAVMRMwEQYDVQQDDAprYWdlbnQtZGV2MB4XDTI1MDgyMDE3NTU0OFoXDTM1MDgyMDE3NTcyOFowFTETMBEGA1UEAwwKa2FnZW50LWRldjCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAMWFPLjeq1yOpeiwEOYp6cpROKoharIW1fdlZz61DvIDX3/07mY76tfX9M16VZ57vPGwi+UOg40akT+Gtm6wLGGAY0o8Chhd3Xe43Wurmr1NW69jRaaWehkorDpSriqjAFIrULmIAM2D/hNhWrEDrJK2fB0PEvIX3+6zWR/OFS+ZTcEzB4sC9TvIwZsSRPgATNE6PtmvWKLEZEWv7lc2O9tJUgIq9VDYlEkdT8qUtosVsjtOAFTCX/CYGFPK7FBzZGXp2Ekcks2MyMROe0QmJ7maHNYUlQbTdnBV10qk8BjCO7X80wLRtdWe85Ilpj9E5O8Wm4ecX3V4WbSXx0+onecCAwEAATANBgkqhkiG9w0BAQsFAAOCAQEAWuRnoKtKhCqLaz3Ze2q8hRykke7JwNrNxqDPn7eToa1MKsfsrtE678kzXhnfdivK/1F/8dr7Thn/WX7ZUJW2jsmbP1sCJjK02yY2setJ1jJKvJZcib8y7LAsqoACYZ4FM/KLrdywGn7KSenqWCLRMqeT04dWlmJexEszb5fgCKCFIZLKjaGJZIuLhsJBLyYHEVFpacr69cZ/ZjNpshHIiV0l/I434vcW39S9+uMfxf1glLTEPifmwK4gMRem3QQLqK21vBcjuS0GBQXQinaztcNaiu1invyTZd5s+3u5yORsip6YhbGhe08TbbtN7yLlZFITDQL4oFrXVGXX+4dp8w=="],"x5t":"BMlhx-2TUdiyftY8aR_zt7xECEI","x5t#S256":"YTTj8SxySpGgVFl5ZQqniLPnmg0gWHgBhissHXQCZ8k"}]}`
