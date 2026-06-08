package jwks

import (
	"crypto/tls"
	"encoding/json"
	"testing"
	"time"

	"github.com/stretchr/testify/require"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

func TestJwksSourceMarshalJSONOmitsTLSConfigObjects(t *testing.T) {
	source := JwksSource{
		OwnerKey: JwksOwnerID{
			Kind:      OwnerKindPolicy,
			Namespace: "agentgateway-base",
			Name:      "route-policy",
			Path:      "spec.traffic.jwtAuthentication.providers[0].jwks.remote",
		},
		RequestKey:     remotehttp.FetchKey("request-key"),
		Target:         remotehttp.FetchTarget{URL: "https://dummy-idp.default.svc/org-one/keys"},
		TLSConfig:      &tls.Config{InsecureSkipVerify: true},     //nolint:gosec // purely test data
		ProxyTLSConfig: &tls.Config{ServerName: "proxy.internal"}, //nolint:gosec // purely test data
		TTL:            5 * time.Minute,
	}

	payload, err := json.Marshal(source)
	require.NoError(t, err)
	require.JSONEq(t, `{
		"ownerKey": {
			"Kind": "AgentgatewayPolicy",
			"Namespace": "agentgateway-base",
			"Name": "route-policy",
			"Path": "spec.traffic.jwtAuthentication.providers[0].jwks.remote"
		},
		"requestKey": "request-key",
		"target": {
			"url": "https://dummy-idp.default.svc/org-one/keys",
			"transport": {},
			"proxyTransport": {}
		},
		"hasTLSConfig": true,
		"hasProxyTLSConfig": true,
		"ttl": 300000000000
	}`, string(payload))
}

func TestSharedJwksRequestMarshalJSONOmitsTLSConfigObjects(t *testing.T) {
	request := SharedJwksRequest{
		RequestKey:     remotehttp.FetchKey("request-key"),
		Target:         remotehttp.FetchTarget{URL: "https://dummy-idp.default.svc/org-one/keys"},
		TLSConfig:      &tls.Config{InsecureSkipVerify: true},     //nolint:gosec // purely test data
		ProxyTLSConfig: &tls.Config{ServerName: "proxy.internal"}, //nolint:gosec // purely test data
		TTL:            5 * time.Minute,
	}

	payload, err := json.Marshal(request)
	require.NoError(t, err)
	require.JSONEq(t, `{
		"requestKey": "request-key",
		"target": {
			"url": "https://dummy-idp.default.svc/org-one/keys",
			"transport": {},
			"proxyTransport": {}
		},
		"hasTLSConfig": true,
		"hasProxyTLSConfig": true,
		"ttl": 300000000000
	}`, string(payload))
}
