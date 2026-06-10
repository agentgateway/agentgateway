//go:build e2e

package e2e_test

import (
	"bufio"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"net"
	"net/http"
	"strconv"
	"testing"
	"time"

	"github.com/onsi/gomega"
	"istio.io/istio/pkg/test/util/retry"
	"k8s.io/apimachinery/pkg/types"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/gomega/matchers"
)

const dynamicCARootCAPEM = `-----BEGIN CERTIFICATE-----
MIIBezCCASCgAwIBAgIRAOnmoc9aVSZkyJ59U9r+6KAwCgYIKoZIzj0EAwIwGzEZ
MBcGA1UEAxMQYWdlbnRnYXRld2F5LmRldjAeFw0yNTEwMTUxOTQzMzZaFw0zNTEw
MTMxOTQzMzZaMBsxGTAXBgNVBAMTEGFnZW50Z2F0ZXdheS5kZXYwWTATBgcqhkjO
PQIBBggqhkjOPQMBBwNCAAScPuAg65+9D2YuOrFl4xAYOB6h2460QhZTIStE1PHP
MIOUJAAqBdAWAH5JG4UiVUH/tKYEd73CfaBsHSNrOJlLo0UwQzAOBgNVHQ8BAf8E
BAMCAQYwEgYDVR0TAQH/BAgwBgEB/wIBATAdBgNVHQ4EFgQUcwtMh/9FfJvcR9JU
bISOus7YDMowCgYIKoZIzj0EAwIDSQAwRgIhAL2agfEI9TBl060Y0aGQ7SX69aLC
7/ifjLmH38SGOWCJAiEA63NRyf5oz6rzvvIHpK8OM2hSHqWQFQnhBTCbyzNAe5U=
-----END CERTIFICATE-----`

func TestAgentgatewayDynamicCATLS(tt *testing.T) {
	t := New(tt)

	t.Run("IssuesSNICertificatesAndCaches", func(t base.Test) {
		testDynamicCAIssuesSNICertificatesAndCaches(t)
	})
	t.Run("RejectsInvalidCA", func(t base.Test) {
		testDynamicCARejectsInvalidCA(t)
	})
}

func testDynamicCAIssuesSNICertificatesAndCaches(t base.Test) {
	t.Apply(manifest("dynamic-ca-cert", "dynamic-ca-cert.yaml"))
	gateway := agentgatewayFeatureGateway(t, "dynamic-ca-cert-gateway")
	g := gomega.NewWithT(t)

	first := dynamicCAHTTPSRequest(t, gateway, "first.dynamic-ca-cert.example.com", true)
	g.Expect(first.StatusCode).To(gomega.Equal(http.StatusOK))
	g.Expect(first.Leaf.DNSNames).To(gomega.ContainElement("first.dynamic-ca-cert.example.com"))

	cached := dynamicCAHTTPSRequest(t, gateway, "first.dynamic-ca-cert.example.com", true)
	g.Expect(cached.StatusCode).To(gomega.Equal(http.StatusOK))
	g.Expect(cached.Leaf.Raw).To(gomega.Equal(first.Leaf.Raw), "repeated SNI should reuse cached leaf certificate")

	second := dynamicCAHTTPSRequest(t, gateway, "second.dynamic-ca-cert.example.com", true)
	g.Expect(second.StatusCode).To(gomega.Equal(http.StatusOK))
	g.Expect(second.Leaf.DNSNames).To(gomega.ContainElement("second.dynamic-ca-cert.example.com"))
	g.Expect(second.Leaf.Raw).NotTo(gomega.Equal(first.Leaf.Raw), "different SNI should trigger a different generated leaf certificate")
}

func testDynamicCARejectsInvalidCA(t base.Test) {
	t.Apply(manifest("dynamic-ca-cert", "dynamic-ca-cert-invalid-ca.yaml"))
	gateway := agentgatewayFeatureGatewayAddress(t, "dynamic-ca-cert-invalid-ca-gateway")

	retry.UntilSuccessOrFail(t, func() error {
		_, err := dialDynamicCAGateway(t, gateway, "invalid.dynamic-ca-cert.example.com", true)
		if err == nil {
			return fmt.Errorf("expected TLS handshake to fail with invalid Dynamic CA certificate source")
		}
		return nil
	}, retry.Timeout(30*time.Second))
}

type dynamicCAHTTPSResult struct {
	StatusCode int
	Leaf       *x509.Certificate
}

func dynamicCAHTTPSRequest(t base.Test, gateway base.Gateway, sni string, verify bool) dynamicCAHTTPSResult {
	t.Helper()

	var result dynamicCAHTTPSResult
	retry.UntilSuccessOrFail(t, func() error {
		conn, err := dialDynamicCAGateway(t, gateway, sni, verify)
		if err != nil {
			return err
		}
		defer conn.Close()

		req, err := http.NewRequest(http.MethodGet, "https://"+sni+"/status/200", nil)
		if err != nil {
			return err
		}
		if err := req.Write(conn); err != nil {
			return err
		}
		resp, err := http.ReadResponse(bufio.NewReader(conn), req)
		if err != nil {
			return err
		}
		defer resp.Body.Close()
		if ok, err := matchers.HaveHttpResponse(base.ExpectOK()).Match(resp); err != nil {
			return err
		} else if !ok {
			return fmt.Errorf("unexpected response status %d", resp.StatusCode)
		}
		state := conn.ConnectionState()
		if len(state.PeerCertificates) == 0 {
			return fmt.Errorf("TLS peer did not present certificates")
		}
		result = dynamicCAHTTPSResult{
			StatusCode: resp.StatusCode,
			Leaf:       state.PeerCertificates[0],
		}
		return nil
	}, retry.Timeout(30*time.Second))

	return result
}

func dialDynamicCAGateway(t base.Test, gateway base.Gateway, sni string, verify bool) (*tls.Conn, error) {
	t.Helper()

	rootCAs := x509.NewCertPool()
	if !rootCAs.AppendCertsFromPEM([]byte(dynamicCARootCAPEM)) {
		t.Fatal("failed to load Dynamic CA root CA")
	}

	tlsConfig := &tls.Config{
		ServerName: sni,
		MinVersion: tls.VersionTLS12,
	}
	if verify {
		tlsConfig.RootCAs = rootCAs
	} else {
		tlsConfig.InsecureSkipVerify = true //nolint:gosec // test asserts handshake failure for invalid server config
	}

	dialer := &net.Dialer{Timeout: 5 * time.Second}
	return tls.DialWithDialer(dialer, "tcp", gatewayAddressForRemotePort(t, gateway, 443), tlsConfig)
}

func gatewayAddressForRemotePort(t base.Test, gateway base.Gateway, remotePort int) string {
	t.Helper()

	address := gateway.ResolvedAddress()
	host, port, err := net.SplitHostPort(address)
	if err == nil {
		return net.JoinHostPort(host, port)
	}
	return net.JoinHostPort(address, strconv.Itoa(gateway.PortForRemote(remotePort)))
}

func agentgatewayFeatureGateway(t base.Test, name string) base.Gateway {
	t.Helper()
	t.GatewayReady(name, base.Namespace)
	return agentgatewayFeatureGatewayAddress(t, name)
}

func agentgatewayFeatureGatewayAddress(t base.Test, name string) base.Gateway {
	t.Helper()
	namespacedName := types.NamespacedName{Name: name, Namespace: base.Namespace}
	return base.Gateway{
		NamespacedName: namespacedName,
		Address:        base.ResolveGatewayAddress(t, t.Ctx, t.TestInstallation, namespacedName),
	}
}
