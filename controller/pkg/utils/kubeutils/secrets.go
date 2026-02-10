package kubeutils

import (
	"fmt"
	"strings"
	"unicode/utf8"

	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
)

// GetSecret fetches a Kubernetes secret by name and namespace using krt collection.
func GetSecret(secrets krt.Collection[*corev1.Secret], krtctx krt.HandlerContext, secretName, namespace string) (*corev1.Secret, error) {
	secretKey := namespace + "/" + secretName
	secret := krt.FetchOne(krtctx, secrets, krt.FilterKey(secretKey))
	if secret == nil {
		return nil, fmt.Errorf("secret %s not found", secretKey)
	}
	return *secret, nil
}

// GetSecretValue extracts a value from a Kubernetes secret, handling both Data and StringData fields.
// It prioritizes StringData over Data if both are present.
func GetSecretValue(secret *corev1.Secret, key string) (string, bool) {
	if value, exists := secret.Data[key]; exists && utf8.Valid(value) {
		return strings.TrimSpace(string(value)), true
	}

	return "", false
}

// GetSecretAuth extracts an authentication value from a Kubernetes secret.
// It looks for the "Authorization" field and strips the "Bearer " prefix if present.
func GetSecretAuth(secret *corev1.Secret) (string, bool) {
	if authValue, exists := GetSecretValue(secret, "Authorization"); exists {
		// Strip the "Bearer " prefix if present, as it will be added by the provider
		authValue = strings.TrimSpace(authValue)
		authKey := strings.TrimSpace(strings.TrimPrefix(authValue, "Bearer "))
		return authKey, authKey != ""
	}
	return "", false
}
