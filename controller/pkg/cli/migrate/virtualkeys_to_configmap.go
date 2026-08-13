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
	"strings"

	"github.com/spf13/pflag"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
)

// virtualkeysAPIGroup is discovered at runtime, so this migration keeps working if agentgateway
// adds new Kinds to the group later, without needing code changes here.
const virtualkeysAPIGroup = "agentgateway.dev"

// virtualkeysMigratedLabelKey is the configMapSelector matchLabels key (no ref-by-name alternative exists).
const virtualkeysMigratedLabelKey = "agentgateway.dev/migrated-from-secret"

func init() {
	flags := &struct{ policy string }{}
	registry["virtualkeys-to-configmap"] = Migration{
		ID: "virtualkeys-to-configmap",
		RegisterFlags: func(fs *pflag.FlagSet) {
			fs.StringVar(&flags.policy, "policy", "", "virtualkeys-to-configmap: only migrate the named resource (default: all matching resources in the namespace)")
		},
		Run: func(ctx context.Context, out, status io.Writer, kubeClient kubeutil.CLIClient, namespace string, write bool) error {
			return runVirtualkeysToConfigMap(ctx, out, status, kubeClient, namespace, flags.policy, !write)
		},
	}
}

// virtualkeysCandidate pairs a discovered object with the GVR it came from, needed to write it back.
type virtualkeysCandidate struct {
	gvr    schema.GroupVersionResource
	object unstructured.Unstructured
}

// virtualkeysAPIKeyAuth is the apiKeyAuthentication subset this migration duck-types against.
type virtualkeysAPIKeyAuth struct {
	SecretRef *struct {
		Name string `json:"name"`
		Kind string `json:"kind,omitempty"`
	} `json:"secretRef,omitempty"`
	SecretSelector *struct {
		MatchLabels map[string]string `json:"matchLabels"`
	} `json:"secretSelector,omitempty"`
}

func runVirtualkeysToConfigMap(ctx context.Context, out, status io.Writer, kubeClient kubeutil.CLIClient, namespace, name string, dryRun bool) error {
	candidates, err := virtualkeysLoadCandidates(ctx, kubeClient, namespace, name)
	if err != nil {
		return err
	}

	var secretsToRemove []string
	anyMigrated := false
	for _, candidate := range candidates {
		migrated, secrets, err := virtualkeysMigratePolicy(ctx, out, status, kubeClient, candidate, dryRun)
		if err != nil {
			return fmt.Errorf("%s %s/%s: %w", candidate.object.GetKind(), candidate.object.GetNamespace(), candidate.object.GetName(), err)
		}
		if migrated {
			anyMigrated = true
		}
		secretsToRemove = append(secretsToRemove, secrets...)
	}

	if !anyMigrated {
		fmt.Fprintf(status, "no resources with an apiKeyAuthentication using secretRef/secretSelector were found in the %s API group\n", virtualkeysAPIGroup)
		return nil
	}

	slices.Sort(secretsToRemove)
	secretsToRemove = slices.Compact(secretsToRemove)
	if len(secretsToRemove) > 0 {
		verb := "can"
		if dryRun {
			verb = "will be able to"
		}
		fmt.Fprintf(status, "\nThe following Secrets are no longer referenced by migrated resources and %s be removed, if nothing else references them:\n", verb)
		for _, s := range secretsToRemove {
			fmt.Fprintf(status, "  - %s\n", s)
		}
	}

	return nil
}

// virtualkeysDiscoverGVRs returns every namespaced resource served under virtualkeysAPIGroup, across all its versions.
func virtualkeysDiscoverGVRs(kubeClient kubeutil.CLIClient) ([]schema.GroupVersionResource, error) {
	groups, err := kubeClient.Kube().Discovery().ServerGroups()
	if err != nil {
		return nil, fmt.Errorf("failed to discover API groups: %w", err)
	}

	var gvrs []schema.GroupVersionResource
	for _, group := range groups.Groups {
		if group.Name != virtualkeysAPIGroup {
			continue
		}
		for _, v := range group.Versions {
			gv, err := schema.ParseGroupVersion(v.GroupVersion)
			if err != nil {
				return nil, fmt.Errorf("invalid group version %q: %w", v.GroupVersion, err)
			}
			resources, err := kubeClient.Kube().Discovery().ServerResourcesForGroupVersion(v.GroupVersion)
			if err != nil {
				return nil, fmt.Errorf("failed to discover resources for %s: %w", v.GroupVersion, err)
			}
			for _, r := range resources.APIResources {
				if !r.Namespaced || strings.Contains(r.Name, "/") { // skip cluster-scoped resources and subresources
					continue
				}
				gvrs = append(gvrs, gv.WithResource(r.Name))
			}
		}
	}
	return gvrs, nil
}

func virtualkeysLoadCandidates(ctx context.Context, kubeClient kubeutil.CLIClient, namespace, name string) ([]virtualkeysCandidate, error) {
	gvrs, err := virtualkeysDiscoverGVRs(kubeClient)
	if err != nil {
		return nil, err
	}

	var candidates []virtualkeysCandidate
	for _, gvr := range gvrs {
		client := kubeClient.Dynamic().Resource(gvr).Namespace(namespace)
		if name != "" {
			obj, err := client.Get(ctx, name, metav1.GetOptions{})
			if apierrors.IsNotFound(err) {
				continue
			}
			if err != nil {
				return nil, fmt.Errorf("failed to get %s %s/%s: %w", gvr.Resource, namespace, name, err)
			}
			candidates = append(candidates, virtualkeysCandidate{gvr: gvr, object: *obj})
			continue
		}

		list, err := client.List(ctx, metav1.ListOptions{})
		if err != nil {
			return nil, fmt.Errorf("failed to list %s in namespace %q: %w", gvr.Resource, namespace, err)
		}
		for _, obj := range list.Items {
			candidates = append(candidates, virtualkeysCandidate{gvr: gvr, object: obj})
		}
	}
	return candidates, nil
}

// virtualkeysMigratePolicy migrates one candidate, returning whether it migrated and any secrets no longer referenced.
func virtualkeysMigratePolicy(ctx context.Context, out, status io.Writer, kubeClient kubeutil.CLIClient, candidate virtualkeysCandidate, dryRun bool) (bool, []string, error) {
	policy := candidate.object

	akRaw, found, err := unstructured.NestedMap(policy.Object, "spec", "traffic", "apiKeyAuthentication")
	if err != nil {
		return false, nil, fmt.Errorf("reading spec.traffic.apiKeyAuthentication: %w", err)
	}
	if !found {
		return false, nil, nil
	}
	var ak virtualkeysAPIKeyAuth
	if err := runtime.DefaultUnstructuredConverter.FromUnstructured(akRaw, &ak); err != nil {
		// Has an apiKeyAuthentication block, but not one this migration understands - skip it.
		return false, nil, nil
	}

	var secretNames []string
	switch {
	case ak.SecretRef != nil:
		if ak.SecretRef.Kind != "" && ak.SecretRef.Kind != "Secret" {
			return false, nil, nil
		}
		secretNames = []string{ak.SecretRef.Name}
	case ak.SecretSelector != nil:
		list, err := kubeClient.Kube().CoreV1().Secrets(policy.GetNamespace()).List(ctx, metav1.ListOptions{
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

	labels := map[string]string{virtualkeysMigratedLabelKey: policy.GetName()}
	var secretsToRemove []string
	for _, secretName := range secretNames {
		secret, err := kubeClient.Kube().CoreV1().Secrets(policy.GetNamespace()).Get(ctx, secretName, metav1.GetOptions{})
		if err != nil {
			return false, nil, fmt.Errorf("failed to get Secret %s/%s: %w", policy.GetNamespace(), secretName, err)
		}

		configMap, err := virtualkeysBuildConfigMap(secret, policy.GetName(), labels)
		if err != nil {
			return false, nil, fmt.Errorf("failed to convert Secret %s/%s: %w", policy.GetNamespace(), secretName, err)
		}

		if dryRun {
			if err := printYAML(out, configMap); err != nil {
				return false, nil, err
			}
		} else if _, err := kubeClient.Kube().CoreV1().ConfigMaps(policy.GetNamespace()).Create(ctx, configMap, metav1.CreateOptions{}); err != nil {
			if !apierrors.IsAlreadyExists(err) {
				return false, nil, fmt.Errorf("failed to create ConfigMap %s/%s: %w", configMap.Namespace, configMap.Name, err)
			}
			fmt.Fprintf(status, "ConfigMap %s/%s already exists, skipping creation\n", configMap.Namespace, configMap.Name)
		} else {
			fmt.Fprintf(status, "created ConfigMap %s/%s (from Secret %s)\n", configMap.Namespace, configMap.Name, secretName)
		}

		secretsToRemove = append(secretsToRemove, fmt.Sprintf("%s/%s", policy.GetNamespace(), secretName))
	}

	updated := policy.DeepCopy()
	unstructured.RemoveNestedField(updated.Object, "spec", "traffic", "apiKeyAuthentication", "secretRef")
	unstructured.RemoveNestedField(updated.Object, "spec", "traffic", "apiKeyAuthentication", "secretSelector")
	if err := unstructured.SetNestedStringMap(updated.Object, labels, "spec", "traffic", "apiKeyAuthentication", "configMapSelector", "matchLabels"); err != nil {
		return false, nil, fmt.Errorf("failed to set configMapSelector: %w", err)
	}

	if dryRun {
		if err := printYAML(out, updated); err != nil {
			return false, nil, err
		}
	} else if _, err := kubeClient.Dynamic().Resource(candidate.gvr).Namespace(updated.GetNamespace()).Update(ctx, updated, metav1.UpdateOptions{}); err != nil {
		return false, nil, fmt.Errorf("failed to update %s %s/%s: %w", updated.GetKind(), updated.GetNamespace(), updated.GetName(), err)
	} else {
		fmt.Fprintf(status, "updated %s %s/%s to use configMapSelector\n", updated.GetKind(), updated.GetNamespace(), updated.GetName())
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
