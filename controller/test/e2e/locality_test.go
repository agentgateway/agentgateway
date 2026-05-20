//go:build e2e

package e2e_test

import (
	"fmt"
	"io"
	"net/http"
	"strings"
	"testing"
	"time"

	"github.com/onsi/gomega"
	"istio.io/istio/pkg/test/util/retry"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/utils/requestutils/curl"
	"github.com/agentgateway/agentgateway/controller/test/e2e/base"
)

type localitySuite struct {
	*base.BaseTestingSuite

	workloadEntries []weSpec
}

func newLocalitySuite(t *testing.T) *localitySuite {
	s := &localitySuite{BaseTestingSuite: New(t)}
	s.Setup = localitySetup
	s.SetupSuite()
	t.Cleanup(s.TearDownSuite)
	return s
}

func TestLocality(t *testing.T) {
	s := newLocalitySuite(t)
	s.Run("PreferSameZone", func() {
		s.SetupTest()
		s.testPreferSameZone()
	})
	s.Run("InternalTrafficPolicyLocal", func() {
		s.SetupTest()
		s.testInternalTrafficPolicyLocal()
	})
}

func (s *localitySuite) SetupSuite() {
	s.BaseTestingSuite.SetupSuite()

	// we deploy pods via the yamls, and then we need to copy their IPs onto WorkloadEntries
	// we do this because WorkloadEntry is easier to override locality on, without messing with node info
	zoneAIP := s.waitPodIP("app=" + backendZoneA)
	zoneBIP := s.waitPodIP("app=" + backendZoneB)
	regionBIP := s.waitPodIP("app=" + backendRegionB)
	s.workloadEntries = []weSpec{
		{"we-zone-a", zoneAIP, sameRegion + "/" + sameZone},
		{"we-zone-b", zoneBIP, sameRegion + "/" + otherZone},
		{"we-region-b", regionBIP, otherRegion + "/" + sameZone},
	}
	s.resetWorkloadEntries()

	s.TestInstallation.AssertionsT(s.T()).EventuallyGatewayCondition(
		s.Ctx, localityGatewayName, localityNamespace, gwv1.GatewayConditionProgrammed, metav1.ConditionTrue,
	)
	s.TestInstallation.AssertionsT(s.T()).EventuallyHTTPRouteCondition(
		s.Ctx, localityRouteName, localityNamespace, gwv1.RouteConditionAccepted, metav1.ConditionTrue,
	)
}

func (s *localitySuite) SetupTest() {
	s.resetWorkloadEntries()
	s.resetService()
}

func (s *localitySuite) TearDownSuite() {
	_ = s.TestInstallation.ClusterContext.Cli.RunCommand(
		s.Ctx, "-n", localityNamespace, "delete", "workloadentry", "--all", "--ignore-not-found=true",
	)
	s.BaseTestingSuite.TearDownSuite()
}

func (s *localitySuite) testPreferSameZone() {
	s.setTrafficDistribution("PreferSameZone")

	s.assertTrafficGoesTo(backendZoneA)
	s.deleteWorkloadEntry("we-zone-a")
	s.assertTrafficGoesTo(backendZoneB)
	s.deleteWorkloadEntry("we-zone-b")
	s.assertTrafficGoesTo(backendRegionB)
}

// TestInternalTrafficPolicyLocal verifies the policy is honored: WorkloadEntries
// have no node association, so with InternalTrafficPolicy: Local nothing is
// eligible and every request should 503.
func (s *localitySuite) testInternalTrafficPolicyLocal() {
	s.setInternalTrafficPolicy(corev1.ServiceInternalTrafficPolicyLocal)
	s.assertServiceUnavailable()
}

// ---------- helpers ----------

type weSpec struct {
	name     string
	address  string
	locality string
}

func (s *localitySuite) resetWorkloadEntries() {
	s.applyWorkloadEntries(s.workloadEntries)
}

func (s *localitySuite) resetService() {
	s.updateService(func(svc *corev1.Service) {
		svc.Spec.TrafficDistribution = nil
		svc.Spec.InternalTrafficPolicy = nil
	})
}

func (s *localitySuite) setTrafficDistribution(trafficDistribution string) {
	s.updateService(func(svc *corev1.Service) {
		svc.Spec.TrafficDistribution = new(trafficDistribution)
	})
}

func (s *localitySuite) setInternalTrafficPolicy(policy corev1.ServiceInternalTrafficPolicy) {
	s.updateService(func(svc *corev1.Service) {
		svc.Spec.InternalTrafficPolicy = new(policy)
	})
}

func (s *localitySuite) updateService(mutate func(*corev1.Service)) {
	svcs := s.TestInstallation.ClusterContext.Clientset.CoreV1().Services(localityNamespace)
	svc, err := svcs.Get(s.Ctx, localityServiceName, metav1.GetOptions{})
	s.Require().NoError(err)
	mutate(svc)
	_, err = svcs.Update(s.Ctx, svc, metav1.UpdateOptions{})
	s.Require().NoError(err)
}

func (s *localitySuite) applyWorkloadEntries(entries []weSpec) {
	err := s.TestInstallation.ClusterContext.IstioClient.ApplyYAMLContents("", workloadEntriesYAML(entries))
	s.Require().NoError(err)
}

func (s *localitySuite) deleteWorkloadEntry(name string) {
	err := s.TestInstallation.ClusterContext.Cli.RunCommand(
		s.Ctx, "-n", localityNamespace, "delete", "workloadentry", name, "--ignore-not-found=true",
	)
	s.Require().NoError(err)
}

// workloadEntriesYAML renders a set of WorkloadEntries, each labeled so the
// Service's selector picks it up.
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

func (s *localitySuite) waitPodIP(labelSelector string) string {
	var ip string
	s.TestInstallation.AssertionsT(s.T()).Gomega.Eventually(func(g gomega.Gomega) {
		pods, err := s.TestInstallation.ClusterContext.Clientset.
			CoreV1().Pods(localityNamespace).
			List(s.Ctx, metav1.ListOptions{LabelSelector: labelSelector})
		g.Expect(err).NotTo(gomega.HaveOccurred())
		g.Expect(pods.Items).To(gomega.HaveLen(1))
		g.Expect(pods.Items[0].Status.PodIP).NotTo(gomega.BeEmpty())
		ip = pods.Items[0].Status.PodIP
	}).WithTimeout(30 * time.Second).WithPolling(500 * time.Millisecond).Should(gomega.Succeed())
	return ip
}

func (s *localitySuite) assertTrafficGoesTo(expectedBackends ...string) {
	const requestsPerAttempt = 20

	gw := s.gateway()
	addr := gw.ResolvedAddress()
	opts := append(base.GatewayAddressOptions(addr),
		curl.WithHostHeader(localityHostname),
		curl.WithPath("/"),
	)

	want := sets.New(expectedBackends...)
	retry.UntilSuccessOrFail(s.T(), func() error {
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

func (s *localitySuite) assertServiceUnavailable() {
	const requestsPerAttempt = 20

	gw := s.gateway()
	addr := gw.ResolvedAddress()
	opts := append(base.GatewayAddressOptions(addr),
		curl.WithHostHeader(localityHostname),
		curl.WithPath("/"),
	)

	retry.UntilSuccessOrFail(s.T(), func() error {
		for i := range requestsPerAttempt {
			status, err := curlStatus(opts...)
			if err != nil {
				return fmt.Errorf("request %d: %w", i, err)
			}
			if status != 503 {
				return fmt.Errorf("request %d: got status %d, want 503", i, status)
			}
		}
		return nil
	}, retry.Timeout(45*time.Second), retry.Delay(500*time.Millisecond))
}

func (s *localitySuite) gateway() base.Gateway {
	name := types.NamespacedName{Namespace: localityNamespace, Name: localityGatewayName}
	return base.Gateway{
		NamespacedName: name,
		Address:        base.ResolveGatewayAddress(s.Ctx, s.TestInstallation, name),
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
