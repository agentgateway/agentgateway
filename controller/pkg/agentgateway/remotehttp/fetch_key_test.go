package remotehttp

import "testing"

func TestRequestKeyIncludesTransportSemantics(t *testing.T) {
	t.Parallel()

	strict := Request{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			Verification: VerificationModeStrict,
			CABundleHash: "ca-a",
		},
	}
	hostname := Request{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			Verification: VerificationModeHostname,
			CABundleHash: "ca-a",
		},
	}
	differentCA := Request{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			Verification: VerificationModeStrict,
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

func TestRequestKeyPreservesALPNOrder(t *testing.T) {
	t.Parallel()

	first := Request{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			NextProtos: []string{"h2", "http/1.1"},
		},
	}
	second := Request{
		URL: "https://issuer.example/jwks",
		Transport: TransportFingerprint{
			NextProtos: []string{"http/1.1", "h2"},
		},
	}

	if first.Key() == second.Key() {
		t.Fatalf("expected ALPN order to produce a distinct request key")
	}
}
