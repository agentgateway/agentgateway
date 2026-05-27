//go:build e2e

package e2e_test

import (
	"fmt"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"

	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/retry"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

func TestLocality(tt *testing.T) {
	t := New(tt)
	t.Apply(localitySetup...)
	workloadEntries := setupLocality(t)

	t.Run("PreferSameZone", func(t base.Test) {
		setupLocalityTest(t, workloadEntries)
		testPreferSameZone(t, workloadEntries)
	})
	t.Run("InternalTrafficPolicyLocal", func(t base.Test) {
		setupLocalityTest(t, workloadEntries)
		testInternalTrafficPolicyLocal(t)
	})
}

func setupLocality(t base.Test) []weSpec {
	// We deploy pods via YAML, then copy their IPs onto WorkloadEntries. WorkloadEntry
	// is easier to assign locality to without changing node labels.
	workloadEntries := []weSpec{
		{"we-zone-a", waitPodIP(t, "app="+backendZoneA), sameRegion + "/" + sameZone},
		{"we-zone-b", waitPodIP(t, "app="+backendZoneB), sameRegion + "/" + otherZone},
		{"we-region-b", waitPodIP(t, "app="+backendRegionB), otherRegion + "/" + sameZone},
	}
	resetWorkloadEntries(t, workloadEntries)

	assertions.EventuallyGatewayCondition(t, localityGatewayName, localityNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue)
	assertions.EventuallyHTTPRouteCondition(t, localityRouteName, localityNamespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue)

	testutils.Cleanup(t, func() {
		_ = t.TestInstallation.ClusterContext.ControllerClient.DeleteAllOf(t.Ctx, workloadEntry(), client.InNamespace(localityNamespace))
	})
	return workloadEntries
}

func setupLocalityTest(t base.Test, workloadEntries []weSpec) {
	resetWorkloadEntries(t, workloadEntries)
	resetService(t)
}

func testPreferSameZone(t base.Test, workloadEntries []weSpec) {
	setTrafficDistribution(t, "PreferSameZone")

	assertTrafficGoesTo(t, backendZoneA)
	deleteWorkloadEntry(t, workloadEntries[0].name)
	assertTrafficGoesTo(t, backendZoneB)
	deleteWorkloadEntry(t, workloadEntries[1].name)
	assertTrafficGoesTo(t, backendRegionB)
}

// TestInternalTrafficPolicyLocal verifies the policy is honored: WorkloadEntries
// have no node association, so with InternalTrafficPolicy: Local nothing is
// eligible and every request should 503.
func testInternalTrafficPolicyLocal(t base.Test) {
	setInternalTrafficPolicy(t, corev1.ServiceInternalTrafficPolicyLocal)
	assertServiceUnavailable(t)
}

type weSpec struct {
	name     string
	address  string
	locality string
}

func resetWorkloadEntries(t base.Test, entries []weSpec) {
	applyWorkloadEntries(t, entries)
}

func resetService(t base.Test) {
	updateService(t, func(svc *corev1.Service) {
		svc.Spec.TrafficDistribution = nil
		svc.Spec.InternalTrafficPolicy = nil
	})
}

func setTrafficDistribution(t base.Test, trafficDistribution string) {
	updateService(t, func(svc *corev1.Service) {
		svc.Spec.TrafficDistribution = new(trafficDistribution)
	})
}

func setInternalTrafficPolicy(t base.Test, policy corev1.ServiceInternalTrafficPolicy) {
	updateService(t, func(svc *corev1.Service) {
		svc.Spec.InternalTrafficPolicy = new(policy)
	})
}

func updateService(t base.Test, mutate func(*corev1.Service)) {
	svcs := t.TestInstallation.ClusterContext.Client.Kube().CoreV1().Services(localityNamespace)
	svc, err := svcs.Get(t.Ctx, localityServiceName, metav1.GetOptions{})
	assert.NoError(t, err)
	mutate(svc)
	_, err = svcs.Update(t.Ctx, svc, metav1.UpdateOptions{})
	assert.NoError(t, err)
}

func applyWorkloadEntries(t base.Test, entries []weSpec) {
	err := t.TestInstallation.ClusterContext.Client.ApplyYAMLContents("", workloadEntriesYAML(entries))
	assert.NoError(t, err)
}

func deleteWorkloadEntry(t base.Test, name string) {
	we := workloadEntry()
	we.SetName(name)
	we.SetNamespace(localityNamespace)
	err := t.TestInstallation.ClusterContext.ControllerClient.Delete(t.Ctx, we)
	err = client.IgnoreNotFound(err)
	assert.NoError(t, err)
}

func workloadEntry() *unstructured.Unstructured {
	obj := &unstructured.Unstructured{}
	obj.SetGroupVersionKind(schema.GroupVersionKind{
		Group:   "networking.istio.io",
		Version: "v1",
		Kind:    "WorkloadEntry",
	})
	return obj
}

func workloadEntriesYAML(entries []weSpec) string {
	var b strings.Builder
	for i, e := range entries {
		if i > 0 {
			b.WriteString("\n---\n")
		}
		fmt.Fprintf(&b, `apiVersion: networking.istio.io/v1
kind: WorkloadEntry
metadata:
  name: %s
  namespace: %s
  labels:
    app: locality-svc-workloadentry
spec:
  address: %s
  locality: %q
  ports:
    http: 80
`, e.name, localityNamespace, e.address, e.locality)
	}
	return b.String()
}

func waitPodIP(t base.Test, labelSelector string) string {
	var ip string
	retry.UntilSuccessOrFail(t, func() error {
		pods, err := t.TestInstallation.ClusterContext.Client.Kube().
			CoreV1().Pods(localityNamespace).
			List(t.Ctx, metav1.ListOptions{LabelSelector: labelSelector})
		if err != nil {
			return err
		}
		if len(pods.Items) != 1 {
			return fmt.Errorf("pod count = %d, want 1", len(pods.Items))
		}
		if pods.Items[0].Status.PodIP == "" {
			return fmt.Errorf("pod %s/%s has no pod IP", pods.Items[0].Namespace, pods.Items[0].Name)
		}
		ip = pods.Items[0].Status.PodIP
		return nil
	})
	return ip
}

func assertTrafficGoesTo(t base.Test, expectedBackends ...string) {
	const requestsPerAttempt = 20

	gw := localityGateway(t)
	addr := gw.ResolvedAddress()
	opts := append(base.GatewayAddressOptions(addr),
		curl.WithHostHeader(localityHostname),
		curl.WithPath("/"),
	)

	want := sets.New(expectedBackends...)
	retry.UntilSuccessOrFail(t, func() error {
		got := sets.New[string]()
		for i := range requestsPerAttempt {
			body, err := curlBody(opts...)
			if err != nil {
				return fmt.Errorf("request %d: %w", i, err)
			}
			for line := range strings.Lines(body) {
				name, ok := strings.CutPrefix(strings.TrimSpace(line), "Hostname=")
				if !ok {
					continue
				}
				for b := range want {
					if strings.HasPrefix(name, b+"-") {
						got.Insert(b)
					}
				}
			}
		}
		if !got.Equals(want) {
			return fmt.Errorf("got responses from %v, want %v", got, want)
		}
		return nil
	}, retry.Timeout(45*time.Second), retry.Delay(500*time.Millisecond))
}

func assertServiceUnavailable(t base.Test) {
	const requestsPerAttempt = 20

	gw := localityGateway(t)
	addr := gw.ResolvedAddress()
	opts := append(base.GatewayAddressOptions(addr),
		curl.WithHostHeader(localityHostname),
		curl.WithPath("/"),
	)

	retry.UntilSuccessOrFail(t, func() error {
		for i := range requestsPerAttempt {
			status, err := curlStatus(opts...)
			if err != nil {
				return fmt.Errorf("request %d: %w", i, err)
			}
			if status != http.StatusServiceUnavailable {
				return fmt.Errorf("request %d: got status %d, want 503", i, status)
			}
		}
		return nil
	}, retry.Timeout(45*time.Second), retry.Delay(500*time.Millisecond))
}

func localityGateway(t base.Test) base.Gateway {
	name := types.NamespacedName{Namespace: localityNamespace, Name: localityGatewayName}
	return base.Gateway{
		NamespacedName: name,
		Address:        base.ResolveGatewayAddress(t, t.Ctx, t.TestInstallation, name),
	}
}

func curlBody(opts ...curl.Option) (string, error) {
	resp, err := curl.ExecuteRequest(opts...)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	b, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", err
	}
	if resp.StatusCode != http.StatusOK {
		return string(b), fmt.Errorf("unexpected status %d", resp.StatusCode)
	}
	return string(b), nil
}

func curlStatus(opts ...curl.Option) (int, error) {
	resp, err := curl.ExecuteRequest(opts...)
	if err != nil {
		return 0, err
	}
	resp.Body.Close()
	return resp.StatusCode, nil
}
