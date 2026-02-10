package plugins

import (
	"errors"
	"fmt"
	"strings"

	"github.com/agentgateway/agentgateway/go/api"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/api/v1alpha1/agentgateway"
	"github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/utils"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/translator/sslutils"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

// NewBackendTLSPlugin creates a new BackendTLSPolicy plugin
func NewBackendTLSPlugin(agw *AgwCollections) AgwPlugin {
	policyCol := krt.NewManyCollection(agw.BackendTLSPolicies, func(krtctx krt.HandlerContext, btls *gwv1.BackendTLSPolicy) []AgwPolicy {
		return translatePoliciesForBackendTLS(krtctx, agw.ConfigMaps, agw.Backends, btls)
	}, agw.KrtOpts.ToOptions("agentgateway/BackendTLSPolicy")...)
	return AgwPlugin{
		ContributesPolicies: map[schema.GroupKind]PolicyPlugin{
			wellknown.BackendTLSPolicyGVK.GroupKind(): {
				Policies: policyCol,
			},
		},
		ExtraHasSynced: func() bool {
			return policyCol.HasSynced()
		},
	}
}

// translatePoliciesForService generates backend TLS policies
func translatePoliciesForBackendTLS(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	backends krt.Collection[*agentgateway.AgentgatewayBackend],
	btls *gwv1.BackendTLSPolicy,
) []AgwPolicy {
	logger := logger.With("plugin_kind", "backendtls")
	var policies []AgwPolicy

	for _, target := range btls.Spec.TargetRefs {
		var policyTarget *api.PolicyTarget

		switch string(target.Kind) {
		case wellknown.AgentgatewayBackendGVK.Kind:
			backendRef := types.NamespacedName{
				Name:      string(target.Name),
				Namespace: btls.Namespace,
			}
			backend := krt.FetchOne(krtctx, backends, krt.FilterObjectName(backendRef))
			if backend == nil || *backend == nil {
				logger.Error("backend not found; skipping policy", "backend", backendRef, "policy", kubeutils.NamespacedNameFrom(btls))
				continue
			}
			// The target defaults to <backend-namespace>/<backend-name>.
			// If SectionName is specified to select a specific target in the Backend,
			// the target becomes <backend-namespace>/<backend-name>/<section-name>
			policyTarget = &api.PolicyTarget{
				Kind: utils.BackendTarget(btls.Namespace, string(target.Name), target.SectionName),
			}
		case wellknown.ServiceKind:
			policyTarget = &api.PolicyTarget{
				Kind: utils.ServiceTarget(btls.Namespace, string(target.Name), (*string)(target.SectionName)),
			}
		case wellknown.InferencePoolKind:
			policyTarget = &api.PolicyTarget{
				Kind: utils.InferencePoolTarget(btls.Namespace, string(target.Name), (*string)(target.SectionName)),
			}
		default:
			logger.Warn("unsupported target kind", "kind", target.Kind, "policy", btls.Name)
			continue
		}
		caCert, err := getBackendTLSCACert(krtctx, cfgmaps, btls)
		if err != nil {
			logger.Error("error getting backend TLS CA cert", "policy", kubeutils.NamespacedNameFrom(btls), "error", err)
			return nil
		}

		policy := &api.Policy{
			Key:    btls.Namespace + "/" + btls.Name + backendTlsPolicySuffix + attachmentName(policyTarget),
			Name:   TypedResourceName(wellknown.BackendTLSPolicyKind, btls),
			Target: policyTarget,
			Kind: &api.Policy_Backend{
				Backend: &api.BackendPolicySpec{
					Kind: &api.BackendPolicySpec_BackendTls{
						BackendTls: &api.BackendPolicySpec_BackendTLS{
							Root: caCert,
							// Used for mTLS, not part of the spec currently
							Cert: nil,
							Key:  nil,
							// Validation.Hostname is a required value and validated with CEL
							Hostname: ptr.Of(string(btls.Spec.Validation.Hostname)),
						},
					},
				}},
		}
		policies = append(policies, AgwPolicy{policy})
	}

	return policies
}

func getBackendTLSCACert(
	krtctx krt.HandlerContext,
	cfgmaps krt.Collection[*corev1.ConfigMap],
	btls *gwv1.BackendTLSPolicy,
) ([]byte, error) {
	validation := btls.Spec.Validation
	if wk := validation.WellKnownCACertificates; wk != nil {
		switch kind := *wk; kind {
		case gwv1.WellKnownCACertificatesSystem:
			return nil, nil

		default:
			return nil, fmt.Errorf("unsupported wellKnownCACertificates: %v", kind)
		}
	}

	// One of WellKnownCACertificates or CACertificateRefs will always be specified (CEL validated)
	if len(validation.CACertificateRefs) == 0 {
		// should never happen as this is CEL validated. Only here to prevent panic in tests
		return nil, errors.New("BackendTLSPolicy must specify either wellKnownCACertificates or caCertificateRefs")
	}
	var sb strings.Builder
	for _, ref := range validation.CACertificateRefs {
		if ref.Group != gwv1.Group(wellknown.ConfigMapGVK.Group) || ref.Kind != gwv1.Kind(wellknown.ConfigMapGVK.Kind) {
			return nil, fmt.Errorf("BackendTLSPolicy's validation.caCertificateRefs must be a ConfigMap reference; got %s", ref)
		}
		nn := types.NamespacedName{
			Name:      string(ref.Name),
			Namespace: btls.Namespace,
		}
		cfgmap := krt.FetchOne(krtctx, cfgmaps, krt.FilterObjectName(nn))
		if cfgmap == nil {
			return nil, fmt.Errorf("ConfigMap %s not found", nn)
		}
		caCert, err := sslutils.GetCACertFromConfigMap(ptr.Flatten(cfgmap))
		if err != nil {
			return nil, fmt.Errorf("error extracting CA cert from ConfigMap %s: %w", nn, err)
		}
		if sb.Len() > 0 {
			sb.WriteString("\n")
		}
		sb.WriteString(caCert)
	}
	return []byte(sb.String()), nil
}
