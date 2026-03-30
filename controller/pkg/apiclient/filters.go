package apiclient

import (
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/fields"
)

// SecretsFieldSelector is an optimization to avoid excessive secret bloat.
// We only care about TLS certificates.
// Unfortunately, it is not as simple as selecting type=kubernetes.io/tls; we support generic types.
// Its also likely users have started to use random types and expect them to continue working.
// This makes the assumption we will never care about Helm secrets or SA token secrets - two common
// large secrets in clusters.
// This is a best effort optimization only; the code would behave correctly if we watched all secrets.
var SecretsFieldSelector = fields.AndSelectors(
	fields.OneTermNotEqualSelector("type", "helm.sh/release.v1"),
	fields.OneTermNotEqualSelector("type", string(corev1.SecretTypeServiceAccountToken))).String()
