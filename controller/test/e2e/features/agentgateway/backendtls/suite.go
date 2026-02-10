//go:build e2e

package backendtls

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"path/filepath"

	"github.com/onsi/gomega"
	"github.com/stretchr/testify/suite"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/utils/ptr"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/shared"
	reports "github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/reporter"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/tests/base"
	"github.com/kgateway-dev/kgateway/v2/test/gomega/matchers"
	"github.com/kgateway-dev/kgateway/v2/test/helpers"
)

const (
	namespace = "agentgateway-base"
)

var (
	configMapManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata/configmap.yaml")

	backendTlsPolicy = &gwv1.BackendTLSPolicy{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "tls-policy",
			Namespace: namespace,
		},
	}
	nginxMeta = metav1.ObjectMeta{
		Name:      "backend",
		Namespace: namespace,
	}
	nginx2Meta = metav1.ObjectMeta{
		Name:      "backend2",
		Namespace: namespace,
	}
	svcGroup = ""
	svcKind  = "Service"

	// test cases
	testCases = map[string]*base.TestCase{}
)

type tsuite struct {
	*base.BaseTestingSuite
}

func NewTestingSuite(ctx context.Context, testInst *e2e.TestInstallation) suite.TestingSuite {
	setup := base.TestCase{
		Manifests: []string{filepath.Join(fsutils.MustGetThisDir(), "testdata/configmap.yaml"), filepath.Join(fsutils.MustGetThisDir(), "testdata/base.yaml")},
	}
	return &tsuite{
		BaseTestingSuite: base.NewBaseTestingSuite(ctx, testInst, setup, testCases, base.WithMinGwApiVersion(base.GwApiRequireBackendTLSPolicy)),
	}
}

func (s *tsuite) TestBackendTLSPolicyAndStatus() {
	// Load the BackendTLSPolicy before proceeding with tests
	err := s.TestInstallation.ClusterContext.Client.Get(s.Ctx, client.ObjectKeyFromObject(backendTlsPolicy), backendTlsPolicy)
	s.Require().NoError(err)

	tt := []struct {
		host string
	}{
		{
			host: "example.com",
		},
		{
			host: "example2.com",
		},
	}
	for _, tc := range tt {
		common.BaseGateway.Send(
			s.T(),
			&matchers.HttpResponse{
				StatusCode: http.StatusOK,
			},
			curl.WithHostHeader(tc.host),
			curl.WithPath("/"),
		)
	}

	// TODO: move to testing an in-cluster backend, not google.com
	common.BaseGateway.Send(
		s.T(),
		&matchers.HttpResponse{
			StatusCode: http.StatusMovedPermanently,
		},
		curl.WithHostHeader("foo.com"),
		curl.WithPath("/"),
	)
	agentgateway := true
	if agentgateway {
		// Agentgateway currently doesn't support Statuses for BackendTLSPolicy
		s.T().Log("Skipping status assertions for Agentgateway as they are not currently supported")
		return
	}
	s.assertPolicyStatus(metav1.Condition{
		Type:               string(shared.PolicyConditionAccepted),
		Status:             metav1.ConditionTrue,
		Reason:             string(shared.PolicyReasonValid),
		Message:            reports.PolicyAcceptedMsg,
		ObservedGeneration: backendTlsPolicy.Generation,
	})
	s.assertPolicyStatus(metav1.Condition{
		Type:               string(shared.PolicyConditionAttached),
		Status:             metav1.ConditionTrue,
		Reason:             string(shared.PolicyReasonAttached),
		Message:            reports.PolicyAttachedMsg,
		ObservedGeneration: backendTlsPolicy.Generation,
	})

	// delete configmap so we can assert status updates correctly
	err = s.TestInstallation.Actions.Kubectl().DeleteFile(s.Ctx, configMapManifest)
	s.Require().NoError(err)

	s.assertPolicyStatus(metav1.Condition{
		Type:               string(gwv1.PolicyConditionAccepted),
		Status:             metav1.ConditionFalse,
		Reason:             string(gwv1.PolicyReasonInvalid),
		Message:            fmt.Sprintf("%s: %s/ca", ErrConfigMapNotFound, namespace),
		ObservedGeneration: backendTlsPolicy.Generation,
	})
}

func (s *tsuite) assertPolicyStatus(inCondition metav1.Condition) {
	currentTimeout, pollingInterval := helpers.GetTimeouts()
	p := s.TestInstallation.AssertionsT(s.T())
	p.Gomega.Eventually(func(g gomega.Gomega) {
		tlsPol := &gwv1.BackendTLSPolicy{}
		objKey := client.ObjectKeyFromObject(backendTlsPolicy)
		err := s.TestInstallation.ClusterContext.Client.Get(s.Ctx, objKey, tlsPol)
		g.Expect(err).NotTo(gomega.HaveOccurred(), "failed to get BackendTLSPolicy %s", objKey)

		g.Expect(tlsPol.Status.Ancestors).To(gomega.HaveLen(2), "ancestors didn't have length of 2")

		expectedAncestorRefs := []gwv1.ParentReference{
			{
				Group:     (*gwv1.Group)(&svcGroup),
				Kind:      (*gwv1.Kind)(&svcKind),
				Namespace: ptr.To(gwv1.Namespace(nginxMeta.Namespace)),
				Name:      gwv1.ObjectName(nginxMeta.Name),
			},
			{
				Group:     (*gwv1.Group)(&svcGroup),
				Kind:      (*gwv1.Kind)(&svcKind),
				Namespace: ptr.To(gwv1.Namespace(nginx2Meta.Namespace)),
				Name:      gwv1.ObjectName(nginx2Meta.Name),
			},
		}

		for i, ancestor := range tlsPol.Status.Ancestors {
			expectedRef := expectedAncestorRefs[i]
			g.Expect(ancestor.AncestorRef).To(gomega.BeEquivalentTo(expectedRef))

			g.Expect(ancestor.Conditions).To(gomega.HaveLen(2), "ancestors conditions wasn't length of 2")
			cond := meta.FindStatusCondition(ancestor.Conditions, inCondition.Type)
			g.Expect(cond).NotTo(gomega.BeNil(), "policy should have accepted condition")
			g.Expect(cond.Status).To(gomega.Equal(inCondition.Status), "policy accepted condition should be true")
			g.Expect(cond.Reason).To(gomega.Equal(inCondition.Reason), "policy reason should be accepted")
			g.Expect(cond.Message).To(gomega.Equal(inCondition.Message))
			g.Expect(cond.ObservedGeneration).To(gomega.Equal(inCondition.ObservedGeneration))
		}
	}, currentTimeout, pollingInterval).Should(gomega.Succeed())
}

var (
	ErrConfigMapNotFound = errors.New("ConfigMap not found")
)
