package remotehttp

import (
	"testing"

	"github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
)

func TestRequestKeyIncludesTransportSemantics(t *testing.T) {
	t.Parallel()

	strict := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			CABundleHash: "ca-a",
		},
	}
	hostname := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			Verification: agentgateway.InsecureTLSModeHostname,
			CABundleHash: "ca-a",
		},
	}
	differentCA := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			CABundleHash: "ca-b",
		},
	}

	if strict.Key() == hostname.Key() {
		t.Fatalf("expected hostname verification to produce a distinct request key")
	}
	if strict.Key() == differentCA.Key() {
		t.Fatalf("expected different CA bundles to produce a distinct request key")
	}
}

func TestRequestKeyPreservesVerificationFingerprintCompatibility(t *testing.T) {
	t.Parallel()

	url := "https://issuer.example/jwks"

	strict := FetchTarget{
		URL: url,
		Transport: TransportFingerprint{
			CABundleHash: "ca-a",
		},
	}
	hostname := FetchTarget{
		URL: url,
		Transport: TransportFingerprint{
			Verification: agentgateway.InsecureTLSModeHostname,
			CABundleHash: "ca-a",
		},
	}
	insecure := FetchTarget{
		URL: url,
		Transport: TransportFingerprint{
			Verification: agentgateway.InsecureTLSModeAll,
			CABundleHash: "ca-a",
		},
	}

	if strict.Key() != FetchKey("e3d906b06f588b422b6b382d625e070f9642c2afdb5797d1ce0f12c2b8fe8ad1") {
		t.Fatalf("strict verification fingerprint changed: %s", strict.Key())
	}
	if hostname.Key() != FetchKey("e87ac6c445ca5765c464dace139a702f32324ff57f3a4dd1e212c087a10c5639") {
		t.Fatalf("hostname verification fingerprint changed: %s", hostname.Key())
	}
	if insecure.Key() != FetchKey("3698988ca86642973e494bc25a7517e57294088173ba7b1e1bd2af0f91de216a") {
		t.Fatalf("insecure verification fingerprint changed: %s", insecure.Key())
	}
}

func TestRequestKeyPreservesPlainHTTPCompatibility(t *testing.T) {
	t.Parallel()

	request := FetchTarget{
		URL: "http://keycloak.default.svc.cluster.local:7080/realms/mcp/protocol/openid-connect/certs",
	}

	if request.Key() != FetchKey("8934a9b40d194d588c6a049a782dd1c45bd4821a7e8288210f373f4b89ce765a") {
		t.Fatalf("plain HTTP fingerprint changed: %s", request.Key())
	}
}

func TestRequestKeyDistinguishesByProxyURL(t *testing.T) {
	t.Parallel()

	noProxy := FetchTarget{URL: "https://issuer.example/jwks"}
	withProxy := FetchTarget{URL: "https://issuer.example/jwks", ProxyURL: "http://proxy:8080"}
	differentProxy := FetchTarget{URL: "https://issuer.example/jwks", ProxyURL: "http://other-proxy:3128"}

	if noProxy.Key() == withProxy.Key() {
		t.Fatalf("expected proxy URL to produce a distinct request key")
	}
	if withProxy.Key() == differentProxy.Key() {
		t.Fatalf("expected different proxy URLs to produce distinct request keys")
	}
}

func TestRequestKeyPreservesALPNOrder(t *testing.T) {
	t.Parallel()

	first := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			NextProtos: []string{"h2", "http/1.1"},
		},
	}
	second := FetchTarget{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			NextProtos: []string{"http/1.1", "h2"},
		},
	}

	if first.Key() == second.Key() {
		t.Fatalf("expected ALPN order to produce a distinct request key")
	}
}
