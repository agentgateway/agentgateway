package jwks

import (
	"context"
	"encoding/json"
	"net/http"
	"time"

	"github.com/go-jose/go-jose/v4"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// Fetcher fetches and periodically refreshes remote JWKS keysets.
// Fetched keysets are published as KRT-visible Results.
type Fetcher = remotecache.Fetcher[SharedJwksRequest, Keyset]

// NewFetcher constructs a Fetcher backed by a fresh JwksDriver. The driver is
// returned so tests can swap its DefaultClient.
func NewFetcher(results *JwksResults) (*Fetcher, *JwksDriver) {
	driver := &JwksDriver{DefaultClient: remotehttp.NewDefaultFetchClient()}
	return remotecache.NewFetcher[SharedJwksRequest, Keyset](results, driver.Fetch, logger), driver
}

type JwksDriver struct {
	DefaultClient *http.Client
}

func (d *JwksDriver) Fetch(ctx context.Context, source SharedJwksRequest) (Keyset, error) {
	client, err := remotehttp.PickClient(d.DefaultClient, source.Target, source.TLSConfig, source.ProxyTLSConfig)
	if err != nil {
		return Keyset{}, err
	}

	logger.InfoContext(ctx, "fetching jwks", "url", source.Target.URL)
	_, jwks, err := remotehttp.FetchJWKSBody(ctx, client, source.Target.URL, "JWKS")
	if err != nil {
		return Keyset{}, err
	}
	return buildKeyset(source.RequestKey, source.Target.URL, jwks)
}

func buildKeyset(requestKey remotehttp.FetchKey, requestURL string, jwks jose.JSONWebKeySet) (Keyset, error) {
	serializedJwks, err := json.Marshal(jwks)
	if err != nil {
		return Keyset{}, err
	}
	return Keyset{
		RequestKey: requestKey,
		URL:        requestURL,
		FetchedAt:  time.Now(),
		JwksJSON:   string(serializedJwks),
	}, nil
}
