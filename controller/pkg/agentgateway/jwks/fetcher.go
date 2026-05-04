package jwks

import (
	"context"
	"net/http"

	"github.com/go-jose/go-jose/v4"
	"sigs.k8s.io/controller-runtime/pkg/log"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotecache"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
)

var fetcherLogger = logging.New("jwks_fetcher")

// Fetcher fetches and periodically refreshes remote JWKS keysets.
// Fetched keysets are stored in JwksCache and updates are sent to subscribers.
type Fetcher = remotecache.Fetcher[SharedJwksRequest, Keyset]

func NewFetcher(cache *JwksCache) *Fetcher {
	driver := &JwksDriver{DefaultClient: remotehttp.NewDefaultFetchClient()}
	return remotecache.NewFetcher[SharedJwksRequest, Keyset](cache, driver, fetcherLogger)
}

type JwksDriver struct {
	DefaultClient *http.Client
}

func (d *JwksDriver) Fetch(ctx context.Context, source SharedJwksRequest) (Keyset, error) {
	client, err := remotehttp.PickClient(d.DefaultClient, source.Target, source.TLSConfig, source.ProxyTLSConfig)
	if err != nil {
		return Keyset{}, err
	}

	log.FromContext(ctx).Info("fetching jwks", "url", source.Target.URL)

	jwks, err := remotehttp.FetchJSON[jose.JSONWebKeySet](ctx, client, source.Target, "JWKS")
	if err != nil {
		return Keyset{}, err
	}

	return buildKeyset(source.RequestKey, source.Target.URL, jwks)
}
