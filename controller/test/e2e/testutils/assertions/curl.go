//go:build e2e

package assertions

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os/exec"
	"strings"
	"time"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	. "github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	e2edefaults "github.com/kgateway-dev/kgateway/v2/test/e2e/defaults"
	"github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
	"github.com/kgateway-dev/kgateway/v2/test/gomega/transforms"
	"github.com/kgateway-dev/kgateway/v2/test/helpers"
)

const (
	GatewayProxyName = "gateway-proxy"
)

func (p *Provider) AssertEventualCurlReturnResponse(
	ctx context.Context,
	podOpts kubectl.PodExecOptions,
	curlOptions []curl.Option,
	expectedResponse *matchers.HttpResponse,
	timeout ...time.Duration,
) *http.Response {
	// We rely on the curlPod to execute a curl, therefore we must assert that it actually exists
	p.EventuallyObjectsExist(ctx, &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name: podOpts.Name, Namespace: podOpts.Namespace,
		},
	})

	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)

	var curlHttpResponse *http.Response
	var cachedBodyBytes []byte
	p.Gomega.Eventually(func(g Gomega) {
		curlResponse, err := p.clusterContext.Cli.CurlFromPod(ctx, podOpts, curlOptions...)
		fmt.Printf("want:\n%+v\nstdout:\n%s\nstderr:%s\n\n", expectedResponse, curlResponse.StdOut, curlResponse.StdErr)
		// Ignore certain errors that are expected to occur when the curl command times out
		if err != nil {
			var exitErr *exec.ExitError
			if errors.As(err, &exitErr) && exitErr.ExitCode() == expectedResponse.IgnoreExitCode {
				fmt.Printf("Ignoring curl exit code %d: %v\n", expectedResponse.IgnoreExitCode, err)
			} else {
				g.Expect(err).NotTo(HaveOccurred())
			}
		}

		// Do the transform in a separate step instead of a WithTransform to avoid having to do it twice
		//nolint:bodyclose // The caller of this assertion should be responsible for ensuring the body close - if the response is not needed for the test, AssertEventualCurlResponse should be used instead
		curlHttpResponse = transforms.WithCurlResponse(curlResponse)

		// Read the body, clone and put it back into the response so the gomega matcher
		// will still work but the return response object will also has the body
		bodyBytes, err := io.ReadAll(curlHttpResponse.Body)
		curlHttpResponse.Body.Close()
		g.Expect(err).NotTo(HaveOccurred())
		cachedBodyBytes = make([]byte, len(bodyBytes))
		copy(cachedBodyBytes, bodyBytes)
		curlHttpResponse.Body = io.NopCloser(bytes.NewBuffer(bodyBytes))

		g.Expect(curlHttpResponse).To(matchers.HaveHttpResponse(expectedResponse))
		fmt.Printf("success: %+v", curlResponse)
	}).
		WithTimeout(currentTimeout).
		WithPolling(pollingInterval).
		WithContext(ctx).
		Should(Succeed(), "failed to get expected response")

	if len(cachedBodyBytes) > 0 {
		curlHttpResponse.Body.Close()
		curlHttpResponse.Body = io.NopCloser(bytes.NewBuffer(cachedBodyBytes))
	}
	return curlHttpResponse
}

// We can't use one function and ignore the response because the response body must be closed
func (p *Provider) AssertEventualCurlResponse(
	ctx context.Context,
	podOpts kubectl.PodExecOptions,
	curlOptions []curl.Option,
	expectedResponse *matchers.HttpResponse,
	timeout ...time.Duration,
) {
	resp := p.AssertEventualCurlReturnResponse(ctx, podOpts, curlOptions, expectedResponse, timeout...)
	resp.Body.Close()
}

func (p *Provider) assertCurlReturnResponse(
	ctx context.Context,
	podOpts kubectl.PodExecOptions,
	curlOptions []curl.Option,
	expectedResponse *matchers.HttpResponse,
) *http.Response {
	// We rely on the curlPod to execute a curl, therefore we must assert that it actually exists
	p.EventuallyObjectsExist(ctx, &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name: podOpts.Name, Namespace: podOpts.Namespace,
		},
	})

	// Rely on default timeouts set in CurlFromPod
	curlResponse, err := p.clusterContext.Cli.CurlFromPod(ctx, podOpts, curlOptions...)
	fmt.Printf("want:\n%+v\nstdout:\n%s\nstderr:%s\n\n", expectedResponse, curlResponse.StdOut, curlResponse.StdErr)

	// Ignore certain errors that are expected to occur when the curl command times out
	if err != nil {
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) && exitErr.ExitCode() == expectedResponse.IgnoreExitCode {
			fmt.Printf("Ignoring curl exit code %d: %v\n", expectedResponse.IgnoreExitCode, err)
		} else {
			Expect(err).NotTo(HaveOccurred())
		}
	}

	// Do the transform in a separate step instead of a WithTransform to avoid having to do it twice
	curlHttpResponse := transforms.WithCurlResponse(curlResponse)
	Expect(curlHttpResponse).To(matchers.HaveHttpResponse(expectedResponse))
	fmt.Printf("success: %+v", curlResponse)

	return curlHttpResponse
}

func (p *Provider) assertCurlResponse(
	ctx context.Context,
	podOpts kubectl.PodExecOptions,
	curlOptions []curl.Option,
	expectedResponse *matchers.HttpResponse,
) {
	resp := p.assertCurlReturnResponse(ctx, podOpts, curlOptions, expectedResponse)
	resp.Body.Close()
}

// AssertEventuallyConsistentCurlResponse asserts that the response from a curl command
// eventually and then consistently matches the expected response
func (p *Provider) AssertEventuallyConsistentCurlResponse(
	ctx context.Context,
	podOpts kubectl.PodExecOptions,
	curlOptions []curl.Option,
	expectedResponse *matchers.HttpResponse,
	timeout ...time.Duration,
) {
	p.AssertEventualCurlResponse(ctx, podOpts, curlOptions, expectedResponse)

	pollTimeout := 3 * time.Second
	pollInterval := 1 * time.Second
	if len(timeout) > 0 {
		pollTimeout, pollInterval = helpers.GetTimeouts(timeout...)
	}

	p.Gomega.Consistently(func(g Gomega) {
		p.assertCurlResponse(ctx, podOpts, curlOptions, expectedResponse)
	}).
		WithTimeout(pollTimeout).
		WithPolling(pollInterval).
		WithContext(ctx).
		Should(Succeed())
}

// AssertEventualCurlError asserts that the response from a curl command is an error such as `Failed to connect`
// as opposed to an http error from the server. This is useful when testing that a service is not reachable,
// for example to validate that a delete operation has taken effect.
func (p *Provider) AssertEventualCurlError(
	ctx context.Context,
	podOpts kubectl.PodExecOptions,
	curlOptions []curl.Option,
	expectedErrorCode int, // This is an application error code not an HTTP error code
	timeout ...time.Duration,
) {
	// We rely on the curlPod to execute a curl, therefore we must assert that it actually exists
	p.EventuallyObjectsExist(ctx, &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{
			Name: podOpts.Name, Namespace: podOpts.Namespace,
		},
	})

	pollTimeout := 5 * time.Second
	pollInterval := 500 * time.Millisecond
	if len(timeout) > 0 {
		pollTimeout, pollInterval = helpers.GetTimeouts(timeout...)
	}

	testMessage := fmt.Sprintf("Expected curl error %d", expectedErrorCode)

	p.Gomega.Eventually(func(g Gomega) {
		curlResponse, err := p.clusterContext.Cli.CurlFromPod(ctx, podOpts, curlOptions...)

		if err == nil {
			if curlResponse == nil { // This is not expected to happen, but adding for safety/future-proofing
				fmt.Printf("wanted curl error, got no error and no response\n")
				testMessage = fmt.Sprintf("Expected curl error %d, got no error and no response\n", expectedErrorCode)
			} else {
				fmt.Printf("wanted curl error, got response:\nstdout:\n%s\nstderr:%s\n", curlResponse.StdOut, curlResponse.StdErr)
				curlHttpResponse := transforms.WithCurlResponse(curlResponse)
				testMessage = fmt.Sprintf("failed to get a curl error, got response code: %d", curlHttpResponse.StatusCode)
				curlHttpResponse.Body.Close()
			}
			g.Expect(err).To(HaveOccurred())
		}

		if expectedErrorCode > 0 {
			expectedCurlError := fmt.Sprintf("exit status %d", expectedErrorCode)
			fmt.Printf("wanted curl error: %s, got error: %s\n", expectedCurlError, err.Error())
			testMessage = fmt.Sprintf("Expected curl error: %s, got: %s", expectedCurlError, err.Error())
			g.Expect(err.Error()).To(Equal(expectedCurlError))
		} else {
			fmt.Printf("wanted any curl error, got error: %s\n", err.Error())
		}
	}).
		WithTimeout(pollTimeout).
		WithPolling(pollInterval).
		WithContext(ctx).
		Should(Succeed(), testMessage)
}

func (p *Provider) generateCurlOpts(host string) []curl.Option {
	curlOpts := []curl.Option{
		curl.WithHost(kubeutils.ServiceFQDN(metav1.ObjectMeta{Name: GatewayProxyName, Namespace: p.installContext.InstallNamespace})),
		curl.WithPort(80),
		curl.Silent(),
	}

	path := ""
	parts := strings.SplitN(host, "/", 2)
	host = parts[0]
	if len(parts) > 1 {
		path = parts[1] + path
	}

	parts = strings.SplitN(host, "@", 2)
	if len(parts) > 1 {
		host = parts[1]
		auth := strings.Split(parts[0], ":")
		curlOpts = append(curlOpts, curl.WithBasicAuth(auth[0], auth[1]))
	}

	if host != "" {
		curlOpts = append(curlOpts,
			curl.WithHostHeader(host),
		)
	}

	if path != "" {
		curlOpts = append(curlOpts,
			curl.WithPath(path),
		)
	}

	return curlOpts
}

func (p *Provider) generateCurlOptsWithHeaders(host string, headers map[string]string) []curl.Option {
	curlOpts := p.generateCurlOpts(host)
	for k, v := range headers {
		curlOpts = append(curlOpts, curl.WithHeader(k, v))
	}
	return curlOpts
}

func (p *Provider) CurlConsistentlyRespondsWithStatus(ctx context.Context, host string, status int) {
	curlOptsHeader := p.generateCurlOpts(host)

	p.AssertEventuallyConsistentCurlResponse(
		ctx,
		e2edefaults.CurlPodExecOpt,
		curlOptsHeader,
		&matchers.HttpResponse{StatusCode: status},
		10*time.Second,
		100*time.Millisecond,
	)
}

func (p *Provider) CurlEventuallyRespondsWithStatus(ctx context.Context, host string, status int) {
	curlOptsHeader := p.generateCurlOpts(host)

	p.AssertEventualCurlResponse(
		ctx,
		e2edefaults.CurlPodExecOpt,
		curlOptsHeader,
		&matchers.HttpResponse{StatusCode: status},
	)
}

func (p *Provider) CurlRespondsWithStatus(ctx context.Context, host string, status int) {
	curlOptsHeader := p.generateCurlOpts(host)

	p.assertCurlResponse(
		ctx,
		e2edefaults.CurlPodExecOpt,
		curlOptsHeader,
		&matchers.HttpResponse{StatusCode: status},
	)
}

func (p *Provider) CurlWithHeadersConsistentlyRespondsWithStatus(ctx context.Context, host string, headers map[string]string, status int) {
	curlOptsHeader := p.generateCurlOptsWithHeaders(host, headers)

	p.AssertEventuallyConsistentCurlResponse(
		ctx,
		e2edefaults.CurlPodExecOpt,
		curlOptsHeader,
		&matchers.HttpResponse{StatusCode: status},
		10*time.Second,
		100*time.Millisecond,
	)
}

func (p *Provider) CurlWithHeadersEventuallyRespondsWithStatus(ctx context.Context, host string, headers map[string]string, status int) {
	curlOptsHeader := p.generateCurlOptsWithHeaders(host, headers)

	p.AssertEventualCurlResponse(
		ctx,
		e2edefaults.CurlPodExecOpt,
		curlOptsHeader,
		&matchers.HttpResponse{StatusCode: status},
	)
}

func (p *Provider) CurlWithHeadersRespondsWithStatus(ctx context.Context, host string, headers map[string]string, status int) {
	curlOptsHeader := p.generateCurlOptsWithHeaders(host, headers)

	p.assertCurlResponse(
		ctx,
		e2edefaults.CurlPodExecOpt,
		curlOptsHeader,
		&matchers.HttpResponse{StatusCode: status},
	)
}
