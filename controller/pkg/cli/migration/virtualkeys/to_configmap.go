package virtualkeys

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"maps"
	"slices"

	"github.com/spf13/cobra"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	agentgateway "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/printer"
)

// migratedLabel marks a ConfigMap created by this migration, and is used as
// the configMapSelector's matchLabels so the policy picks it up without
// needing a per-ConfigMap reference field.
const migratedLabelKey = "agentgateway.dev/migrated-from-secret"

type toConfigMapFlags struct {
	namespace string
	policy    string
	dryRun    bool
}

func toConfigMapCmd() *cobra.Command {
	f := &toConfigMapFlags{}

	cmd := &cobra.Command{
		Use:   "to-configmap",
		Short: "Migrate API key credentials from Secrets to ConfigMaps",
		Long: `Migrate API key (virtual key) credentials referenced by AgentgatewayPolicy
resources from Kubernetes Secrets to ConfigMaps.

For every AgentgatewayPolicy whose apiKeyAuthentication uses secretRef or
secretSelector, this command:
  - reads the referenced Secret(s)
  - creates an equivalent ConfigMap for each Secret, hashing any raw API keys
    with sha256 (ConfigMaps only support keyHash, since they aren't
    confidential)
  - updates the policy to use configMapSelector instead of secretRef/secretSelector
  - prints the Secrets that are no longer referenced, which are now safe to remove

Use --dry-run to preview the changes without modifying the cluster.`,
		Example: `agctl migration virtualkeys to-configmap -n my-namespace
agctl migration virtualkeys to-configmap -n my-namespace --policy my-api-key-policy
agctl migration virtualkeys to-configmap -n my-namespace --dry-run`,
		Args:         cobra.NoArgs,
		SilenceUsage: true,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runToConfigMap(cmd, f)
		},
	}

	cmd.Flags().StringVarP(&f.namespace, "namespace", "n", "", "Namespace to migrate policies in")
	cmd.Flags().StringVar(&f.policy, "policy", "", "Only migrate the named AgentgatewayPolicy (default: all policies in the namespace)")
	cmd.Flags().BoolVar(&f.dryRun, "dry-run", false, "Print the changes that would be made without applying them")

	return cmd
}

func runToConfigMap(cmd *cobra.Command, f *toConfigMapFlags) error {
	ctx := cmd.Context()

	namespace, err := kubeutil.LoadNamespace(f.namespace)
	if err != nil {
		return err
	}

	kubeClient, err := kubeutil.NewCLIClient()
	if err != nil {
		return err
	}

	policies, err := loadPolicies(ctx, kubeClient, namespace, f.policy)
	if err != nil {
		return err
	}

	var secretsToRemove []string
	anyMigrated := false
	for _, policy := range policies {
		migrated, secrets, err := migratePolicy(ctx, cmd, kubeClient, policy, f.dryRun)
		if err != nil {
			return fmt.Errorf("policy %s/%s: %w", policy.Namespace, policy.Name, err)
		}
		if migrated {
			anyMigrated = true
		}
		secretsToRemove = append(secretsToRemove, secrets...)
	}

	if !anyMigrated {
		fmt.Fprintln(cmd.OutOrStdout(), "no AgentgatewayPolicy resources using secretRef/secretSelector API key credentials were found")
		return nil
	}

	slices.Sort(secretsToRemove)
	secretsToRemove = slices.Compact(secretsToRemove)
	if len(secretsToRemove) > 0 {
		verb := "can"
		if f.dryRun {
			verb = "will be able to"
		}
		fmt.Fprintf(cmd.OutOrStdout(), "\nThe following Secrets are no longer referenced and %s be removed:\n", verb)
		for _, s := range secretsToRemove {
			fmt.Fprintf(cmd.OutOrStdout(), "  - %s\n", s)
		}
	}

	return nil
}

func loadPolicies(ctx context.Context, kubeClient kubeutil.CLIClient, namespace, name string) ([]agentgateway.AgentgatewayPolicy, error) {
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

// migratePolicy migrates a single policy's API key credentials, returning
// whether it was migrated and the names ("namespace/name") of Secrets that
// are no longer referenced as a result.
func migratePolicy(ctx context.Context, cmd *cobra.Command, kubeClient kubeutil.CLIClient, policy agentgateway.AgentgatewayPolicy, dryRun bool) (bool, []string, error) {
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

	labels := map[string]string{migratedLabelKey: policy.Name}
	var secretsToRemove []string
	for _, secretName := range secretNames {
		secret, err := kubeClient.Kube().CoreV1().Secrets(policy.Namespace).Get(ctx, secretName, metav1.GetOptions{})
		if err != nil {
			return false, nil, fmt.Errorf("failed to get Secret %s/%s: %w", policy.Namespace, secretName, err)
		}

		configMap, err := buildConfigMap(secret, labels)
		if err != nil {
			return false, nil, fmt.Errorf("failed to convert Secret %s/%s: %w", policy.Namespace, secretName, err)
		}

		if dryRun {
			fmt.Fprintf(cmd.OutOrStdout(), "would create ConfigMap %s/%s (from Secret %s):\n", configMap.Namespace, configMap.Name, secretName)
			if err := printYAML(cmd, configMap); err != nil {
				return false, nil, err
			}
		} else if _, err := kubeClient.Kube().CoreV1().ConfigMaps(policy.Namespace).Create(ctx, configMap, metav1.CreateOptions{}); err != nil {
			if !apierrors.IsAlreadyExists(err) {
				return false, nil, fmt.Errorf("failed to create ConfigMap %s/%s: %w", configMap.Namespace, configMap.Name, err)
			}
			fmt.Fprintf(cmd.OutOrStdout(), "ConfigMap %s/%s already exists, skipping creation\n", configMap.Namespace, configMap.Name)
		} else {
			fmt.Fprintf(cmd.OutOrStdout(), "created ConfigMap %s/%s (from Secret %s)\n", configMap.Namespace, configMap.Name, secretName)
		}

		secretsToRemove = append(secretsToRemove, fmt.Sprintf("%s/%s", policy.Namespace, secretName))
	}

	updated := policy.DeepCopy()
	updated.Spec.Traffic.APIKeyAuthentication.SecretRef = nil
	updated.Spec.Traffic.APIKeyAuthentication.SecretSelector = nil
	updated.Spec.Traffic.APIKeyAuthentication.ConfigMapSelector = &agentgateway.ConfigMapSelector{MatchLabels: labels}

	if dryRun {
		fmt.Fprintf(cmd.OutOrStdout(), "would update AgentgatewayPolicy %s/%s:\n", updated.Namespace, updated.Name)
		if err := printYAML(cmd, updated); err != nil {
			return false, nil, err
		}
	} else if _, err := kubeClient.Agentgateway().AgentgatewayAgentgateway().AgentgatewayPolicies(policy.Namespace).Update(ctx, updated, metav1.UpdateOptions{}); err != nil {
		return false, nil, fmt.Errorf("failed to update AgentgatewayPolicy %s/%s: %w", updated.Namespace, updated.Name, err)
	} else {
		fmt.Fprintf(cmd.OutOrStdout(), "updated AgentgatewayPolicy %s/%s to use configMapSelector\n", updated.Namespace, updated.Name)
	}

	return true, secretsToRemove, nil
}

// apiKeyEntry mirrors the JSON shape accepted for API key data, matching
// pkg/agentgateway/plugins.APIKeyEntry.
type apiKeyEntry struct {
	Key      string          `json:"key,omitempty"`
	KeyHash  string          `json:"keyHash,omitempty"`
	Metadata json.RawMessage `json:"metadata,omitempty"`
}

func buildConfigMap(secret *corev1.Secret, labels map[string]string) (*corev1.ConfigMap, error) {
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
		entry, err := toKeyHashEntry(merged[k])
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
		ObjectMeta: metav1.ObjectMeta{
			Name:      secret.Name + "-configmap",
			Namespace: secret.Namespace,
			Labels:    labels,
		},
		Data: data,
	}, nil
}

// toKeyHashEntry parses a Secret data value in any of the supported formats
// (raw key, or JSON with key/keyHash) and returns an entry using only
// keyHash, hashing any raw key with sha256.
func toKeyHashEntry(v []byte) (apiKeyEntry, error) {
	var entry apiKeyEntry
	trimmed := bytes.TrimSpace(v)
	if len(trimmed) == 0 {
		return apiKeyEntry{}, fmt.Errorf("empty value")
	}
	if trimmed[0] != '{' {
		entry = apiKeyEntry{Key: string(trimmed)}
	} else if err := json.Unmarshal(trimmed, &entry); err != nil {
		return apiKeyEntry{}, fmt.Errorf("invalid JSON: %w", err)
	}

	if entry.KeyHash != "" {
		return apiKeyEntry{KeyHash: entry.KeyHash, Metadata: entry.Metadata}, nil
	}
	if entry.Key == "" {
		return apiKeyEntry{}, fmt.Errorf("exactly one of key or keyHash must be set")
	}

	sum := sha256.Sum256([]byte(entry.Key))
	return apiKeyEntry{KeyHash: "sha256:" + hex.EncodeToString(sum[:]), Metadata: entry.Metadata}, nil
}

func printYAML(cmd *cobra.Command, v any) error {
	p, err := printer.New("yaml")
	if err != nil {
		return err
	}
	return p.Print(cmd.OutOrStdout(), v)
}
