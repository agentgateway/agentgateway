//go:build e2e

package e2e_test

import (
	"fmt"
	"io"
	"net"
	"strings"
	"testing"
	"time"

	"istio.io/istio/pkg/test/util/retry"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
)

func TestHTTPHeaderCase(tt *testing.T) {
	t := New(tt)

	t.Apply(manifest("headercase", "preserve.yaml"))

	gateway := sharedGateway(t, "http", 1)
	assertions.EventuallyHTTPRouteCondition(t, "headercase", base.Namespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)
	assertions.EventuallyHTTPRouteCondition(t, "headercase", base.Namespace, gwv1.RouteConditionResolvedRefs, metav1.ConditionTrue)
	assertions.EventuallyAgwPolicyCondition(t, "headercase-preserve", base.Namespace, "Accepted", metav1.ConditionTrue)

	retry.UntilSuccessOrFail(t, func() error {
		body, err := sendRawHeaderCaseRequest(gateway)
		if err != nil {
			return err
		}
		if !strings.Contains(body, "\nCaSE: preserve-me\n") {
			return fmt.Errorf("backend did not receive preserved header case; raw request was:\n%s", body)
		}
		if !strings.Contains(body, "\nCaSe: preserve-me2\n") {
			return fmt.Errorf("backend did not receive preserved header case; raw request was:\n%s", body)
		}
		if strings.Contains(body, "\ncaSE: preserve-me\n") {
			return fmt.Errorf("backend received lowercased header case; raw request was:\n%s", body)
		}
		return nil
	}, retry.Timeout(30*time.Second), retry.Delay(250*time.Millisecond))
}

func sendRawHeaderCaseRequest(gateway base.Gateway) (string, error) {
	address := gateway.ResolvedAddress()
	var target string
	if host, port, err := net.SplitHostPort(address); err == nil {
		if strings.EqualFold(host, "localhost") {
			host = "127.0.0.1"
		}
		target = net.JoinHostPort(host, port)
	} else {
		target = net.JoinHostPort(address, "80")
	}

	conn, err := net.DialTimeout("tcp", target, 5*time.Second)
	if err != nil {
		return "", err
	}
	defer conn.Close()

	_ = conn.SetDeadline(time.Now().Add(10 * time.Second))
	_, err = io.WriteString(conn, "GET / HTTP/1.1\r\nHost: headercase.example.com\r\nCaSE: preserve-me\nCaSe: preserve-me2\r\nConnection: close\r\n\r\n")
	if err != nil {
		return "", err
	}

	raw, err := io.ReadAll(conn)
	if err != nil {
		return "", err
	}
	parts := strings.SplitN(string(raw), "\r\n\r\n", 2)
	if len(parts) != 2 {
		return "", fmt.Errorf("raw response missing header terminator: %q", string(raw))
	}
	if !strings.Contains(parts[0], " 200 ") {
		return "", fmt.Errorf("raw response was not 200: %s", parts[0])
	}
	return parts[1], nil
}
