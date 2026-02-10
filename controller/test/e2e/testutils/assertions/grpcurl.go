//go:build e2e

package assertions

import (
	"context"
	"fmt"
	"time"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	. "github.com/onsi/gomega"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/grpcurl"
	"github.com/kgateway-dev/kgateway/v2/test/helpers"
)

// AssertEventualGrpcurlSuccess checks that a grpcurl command eventually succeeds (exit code 0).
// It returns the StdOut and StdErr from the successful command.
// podOpts should be configured with the correct Name, Namespace, and Container for the grpcurl client pod.
func (p *Provider) AssertEventualGrpcurlSuccess(
	ctx context.Context,
	podOpts kubectl.PodExecOptions,
	grpcurlOptions []grpcurl.Option,
	timeout ...time.Duration,
) (stdout, stderr string) {
	// Ensure the grpcurl client pod is running.
	p.EventuallyObjectsExist(ctx, &corev1.Pod{
		ObjectMeta: metav1.ObjectMeta{Name: podOpts.Name, Namespace: podOpts.Namespace},
	})

	currentTimeout, pollingInterval := helpers.GetTimeouts(timeout...)

	var resp *kubectl.GrpcurlResponse
	p.Gomega.Eventually(func(g Gomega) {
		var err error
		resp, err = p.clusterContext.Cli.GrpcurlFromPod(ctx, podOpts, grpcurlOptions...)
		if err != nil {
			errMsg := fmt.Sprintf("grpcurl command failed. Stdout: %s, Stderr: %s, Error: %v", resp.StdOut, resp.StdErr, err)
			fmt.Println(errMsg)                         // Log for test visibility
			g.Expect(err).NotTo(HaveOccurred(), errMsg) // This will cause the Eventually to retry
			return
		}
		// If err is nil, kubectl.ExecWithOptions implies exit code 0.
		fmt.Printf("grpcurl command succeeded. Stdout: %s, Stderr: %s\n", resp.StdOut, resp.StdErr)
	}).
		WithTimeout(currentTimeout).
		WithPolling(pollingInterval).
		WithContext(ctx).
		Should(Succeed(), "grpcurl command did not succeed eventually")

	// Ensure resp is not nil before accessing its fields, though Gomega.Eventually should ensure it ran at least once successfully.
	if resp != nil {
		return resp.StdOut, resp.StdErr
	}
	return "", ""
}
