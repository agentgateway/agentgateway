package migrate

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"maps"
	"slices"

	"github.com/spf13/pflag"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	agentgateway "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
)

// virtualkeysMigratedLabelKey is the configMapSelector matchLabels key, used
// since the API has no ref-by-name alternative.
const virtualkeysMigratedLabelKey = "agentgateway.dev/migrated-from-secret"

func init() {
	flags := &struct{ policy string }{}
	registry["virtualkeys-to-configmap"] = Migration{
		ID: "virtualkeys-to-configmap",
		RegisterFlags: func(fs *pflag.FlagSet) {
			fs.StringVar(&flags.policy, "policy", "", "virtualkeys-to-configmap: only migrate the named AgentgatewayPolicy (default: all policies in the namespace)")
		},
		Run: func(ctx context.Context, out, status io.Writer, kubeClient kubeutil.CLIClient, namespace string, write bool) error {
			return runVirtualkeysToConfigMap(ctx, out, status, kubeClient, namespace, flags.policy, !write)
		},
	}
}

func runVirtualkeysToConfigMap(ctx context.Context, out, status io.Writer, kubeClient kubeutil.CLIClient, namespace, policyName string, dryRun bool) error {
	policies, err := virtualkeysLoadPolicies(ctx, kubeClient, namespace, policyName)
	if err != nil {
		return err
	}

	var secretsToRemove []string
	anyMigrated := false
	for _, policy := range policies {
		migrated, secrets, err := virtualkeysMigratePolicy(ctx, out, status, kubeClient, policy, dryRun)
		if err != nil {
			return fmt.Errorf("policy %s/%s: %w", policy.Namespace, policy.Name, err)
		}
		if migrated {
			anyMigrated = true
		}
		secretsToRemove = append(secretsToRemove, secrets...)
	}

	if !anyMigrated {
		fmt.Fprintln(status, "no AgentgatewayPolicy resources using secretRef/secretSelector API key credentials were found")
		return nil
	}

	slices.Sort(secretsToRemove)
	secretsToRemove = slices.Compact(secretsToRemove)
	if len(secretsToRemove) > 0 {
		verb := "can"
		if dryRun {
			verb = "will be able to"
		}
		fmt.Fprintf(status, "\nThe following Secrets are no longer referenced by migrated AgentgatewayPolicy resources and %s be removed, if nothing else references them:\n", verb)
		for _, s := range secretsToRemove {
			fmt.Fprintf(status, "  - %s\n", s)
		}
	}

	return nil
}

func virtualkeysLoadPolicies(ctx context.Context, kubeClient kubeutil.CLIClient, namespace, name string) ([]agentgateway.AgentgatewayPolicy, error) {
	client := kubeClient.Agentgateway().AgentgatewayAgentgateway().AgentgatewayPolicies(namespace)
	if name != "" {
		policy, err := client.Get(ctx, name, metav1.GetOptions{})
		if err != nil {
			return nil, fmt.Errorf("failed to get AgentgatewayPolicy %s/%s: %w", namespace, name, err)
		}
		return []agentgateway.AgentgatewayPolicy{*policy}, nil
	}

	list, err := client.List(ctx, metav1.ListOptions{})
	if err != nil {
		return nil, fmt.Errorf("failed to list AgentgatewayPolicy resources in namespace %q: %w", namespace, err)
	}
	return list.Items, nil
}

// virtualkeysMigratePolicy migrates one policy, returning whether it migrated
// and any "ns/name" Secrets no longer referenced as a result.
func virtualkeysMigratePolicy(ctx context.Context, out, status io.Writer, kubeClient kubeutil.CLIClient, policy agentgateway.AgentgatewayPolicy, dryRun bool) (bool, []string, error) {
	if policy.Spec.Traffic == nil || policy.Spec.Traffic.APIKeyAuthentication == nil {
		return false, nil, nil
	}
	ak := policy.Spec.Traffic.APIKeyAuthentication

	var secretNames []string
	switch {
	case ak.SecretRef != nil:
		if ak.SecretRef.Kind != "" && ak.SecretRef.Kind != "Secret" {
			return false, nil, nil
		}
		secretNames = []string{string(ak.SecretRef.Name)}
	case ak.SecretSelector != nil:
		list, err := kubeClient.Kube().CoreV1().Secrets(policy.Namespace).List(ctx, metav1.ListOptions{
			LabelSelector: metav1.FormatLabelSelector(&metav1.LabelSelector{MatchLabels: ak.SecretSelector.MatchLabels}),
		})
		if err != nil {
			return false, nil, fmt.Errorf("failed to list Secrets for secretSelector: %w", err)
		}
		for _, s := range list.Items {
			secretNames = append(secretNames, s.Name)
		}
	default:
		// Already configMap-backed, or nothing set.
		return false, nil, nil
	}

	labels := map[string]string{virtualkeysMigratedLabelKey: policy.Name}
	var secretsToRemove []string
	for _, secretName := range secretNames {
		secret, err := kubeClient.Kube().CoreV1().Secrets(policy.Namespace).Get(ctx, secretName, metav1.GetOptions{})
		if err != nil {
			return false, nil, fmt.Errorf("failed to get Secret %s/%s: %w", policy.Namespace, secretName, err)
		}

		configMap, err := virtualkeysBuildConfigMap(secret, policy.Name, labels)
		if err != nil {
			return false, nil, fmt.Errorf("failed to convert Secret %s/%s: %w", policy.Namespace, secretName, err)
		}

		if dryRun {
			if err := printYAML(out, configMap); err != nil {
				return false, nil, err
			}
		} else if _, err := kubeClient.Kube().CoreV1().ConfigMaps(policy.Namespace).Create(ctx, configMap, metav1.CreateOptions{}); err != nil {
			if !apierrors.IsAlreadyExists(err) {
				return false, nil, fmt.Errorf("failed to create ConfigMap %s/%s: %w", configMap.Namespace, configMap.Name, err)
			}
			fmt.Fprintf(status, "ConfigMap %s/%s already exists, skipping creation\n", configMap.Namespace, configMap.Name)
		} else {
			fmt.Fprintf(status, "created ConfigMap %s/%s (from Secret %s)\n", configMap.Namespace, configMap.Name, secretName)
		}

		secretsToRemove = append(secretsToRemove, fmt.Sprintf("%s/%s", policy.Namespace, secretName))
	}

	updated := policy.DeepCopy()
	updated.TypeMeta = metav1.TypeMeta{APIVersion: agentgateway.GroupVersion.String(), Kind: "AgentgatewayPolicy"}
	updated.Spec.Traffic.APIKeyAuthentication.SecretRef = nil
	updated.Spec.Traffic.APIKeyAuthentication.SecretSelector = nil
	updated.Spec.Traffic.APIKeyAuthentication.ConfigMapSelector = &agentgateway.ConfigMapSelector{MatchLabels: labels}

	if dryRun {
		if err := printYAML(out, updated); err != nil {
			return false, nil, err
		}
	} else if _, err := kubeClient.Agentgateway().AgentgatewayAgentgateway().AgentgatewayPolicies(policy.Namespace).Update(ctx, updated, metav1.UpdateOptions{}); err != nil {
		return false, nil, fmt.Errorf("failed to update AgentgatewayPolicy %s/%s: %w", updated.Namespace, updated.Name, err)
	} else {
		fmt.Fprintf(status, "updated AgentgatewayPolicy %s/%s to use configMapSelector\n", updated.Namespace, updated.Name)
	}

	return true, secretsToRemove, nil
}

// virtualkeysAPIKeyEntry mirrors pkg/agentgateway/plugins.APIKeyEntry's JSON shape.
type virtualkeysAPIKeyEntry struct {
	Key      string          `json:"key,omitempty"`
	KeyHash  string          `json:"keyHash,omitempty"`
	Metadata json.RawMessage `json:"metadata,omitempty"`
}

// virtualkeysBuildConfigMap converts a Secret to its ConfigMap equivalent.
// The name is scoped by policyName so policies sharing a Secret don't collide on one ConfigMap.
func virtualkeysBuildConfigMap(secret *corev1.Secret, policyName string, labels map[string]string) (*corev1.ConfigMap, error) {
	data := make(map[string]string, len(secret.Data)+len(secret.StringData))
	merged := make(map[string][]byte, len(secret.Data)+len(secret.StringData))
	maps.Copy(merged, secret.Data)
	for k, v := range secret.StringData {
		merged[k] = []byte(v)
	}

	keys := make([]string, 0, len(merged))
	for k := range merged {
		keys = append(keys, k)
	}
	slices.Sort(keys)

	for _, k := range keys {
		entry, err := virtualkeysToKeyHashEntry(merged[k])
		if err != nil {
			return nil, fmt.Errorf("key %q: %w", k, err)
		}
		out, err := json.Marshal(entry)
		if err != nil {
			return nil, fmt.Errorf("key %q: %w", k, err)
		}
		data[k] = string(out)
	}

	return &corev1.ConfigMap{
		TypeMeta: metav1.TypeMeta{APIVersion: corev1.SchemeGroupVersion.String(), Kind: "ConfigMap"},
		ObjectMeta: metav1.ObjectMeta{
			Name:      secret.Name + "-" + policyName + "-configmap",
			Namespace: secret.Namespace,
			Labels:    labels,
		},
		Data: data,
	}, nil
}

// virtualkeysToKeyHashEntry parses a raw key or key/keyHash JSON value into a
// keyHash-only entry, hashing raw keys with sha256.
func virtualkeysToKeyHashEntry(v []byte) (virtualkeysAPIKeyEntry, error) {
	var entry virtualkeysAPIKeyEntry
	trimmed := bytes.TrimSpace(v)
	if len(trimmed) == 0 {
		return virtualkeysAPIKeyEntry{}, fmt.Errorf("empty value")
	}
	if trimmed[0] != '{' {
		entry = virtualkeysAPIKeyEntry{Key: string(trimmed)}
	} else if err := json.Unmarshal(trimmed, &entry); err != nil {
		return virtualkeysAPIKeyEntry{}, fmt.Errorf("invalid JSON: %w", err)
	}

	if entry.KeyHash != "" {
		return virtualkeysAPIKeyEntry{KeyHash: entry.KeyHash, Metadata: entry.Metadata}, nil
	}
	if entry.Key == "" {
		return virtualkeysAPIKeyEntry{}, fmt.Errorf("one of key or keyHash must be set")
	}

	sum := sha256.Sum256([]byte(entry.Key))
	return virtualkeysAPIKeyEntry{KeyHash: "sha256:" + hex.EncodeToString(sum[:]), Metadata: entry.Metadata}, nil
}
