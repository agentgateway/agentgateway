package virtualkey

import (
	"context"
	"fmt"
	"io"
	"os"
	"strings"

	"github.com/spf13/cobra"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/tools/clientcmd"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/kubeutil"
	"github.com/agentgateway/agentgateway/controller/pkg/cli/printer"
)

const (
	outputJSON = "json"
	outputText = "text"
	outputYAML = "yaml"
)

func Command() flag.Command {
	return flag.Command{
		Use:   "virtualkey",
		Short: "Manage virtual API keys",
		Long: strings.TrimSpace(`Manage virtual API keys.

Generated keys and import output contain secret key material. Store command
output securely.`),
		Children: []flag.CommandBuilder{
			importCommand,
			generateCommand,
		},
	}
}

type importFlags struct {
	file                   string
	secretName             string
	namespace              string
	output                 string
	mode                   string
	strict                 bool
	escapeSpecialChars     bool
	labels                 []string
	kubecontext            string
	collisionCheckSecret   string
	collisionCheckSelector string
}

func importCommand() flag.Command {
	flags := &importFlags{
		output: outputYAML,
		mode:   "replace",
	}
	return flag.Command{
		Use:   "import",
		Short: "Import virtual API keys from CSV",
		Long: strings.TrimSpace(`Import virtual API keys from CSV and print Kubernetes Secret manifests.

The CSV must include a header row. The key column is optional; empty key values
cause agctl to generate new keys. The optional entry column controls the Secret
data key; when omitted, agctl derives one from id or the row number. The id or
metadata.id column is stored as stable key identity metadata. Columns named
metadata.<name> are stored as metadata, and other non-reserved columns are also
preserved as metadata.

By default, id values containing reserved special characters are rejected. Use
--escape-special-characters to store escaped id values instead.

Stdout contains Secret manifests with key material. Store it securely.`),
		Example: strings.TrimSpace(`
agctl virtualkey import -f virtual-keys.csv --secret-name agw-virtual-keys -n default
agctl virtualkey import -f virtual-keys.csv --secret-name agw-virtual-keys -n default | kubectl apply -f -
agctl virtualkey import -f virtual-keys.csv --secret-name agw-virtual-keys --escape-special-characters
`),
		Args: cobra.NoArgs,
		AddFlags: func(cmd *cobra.Command) {
			cmd.Flags().StringVarP(&flags.file, "file", "f", "", "CSV input path, or - for stdin")
			cmd.Flags().StringVar(&flags.secretName, "secret-name", "", "Name of the Secret to emit")
			cmd.Flags().StringVarP(&flags.namespace, "namespace", "n", "", "Namespace for the emitted Secret")
			cmd.Flags().StringVarP(&flags.output, "output", "o", flags.output, "Output format: one of yaml|json")
			cmd.Flags().StringVar(&flags.mode, "mode", flags.mode, "Import mode; only replace is currently supported")
			cmd.Flags().BoolVar(&flags.strict, "strict", false, "Treat warnings, such as missing or duplicate id metadata, as errors")
			cmd.Flags().BoolVar(&flags.escapeSpecialChars, "escape-special-characters", false, "Escape reserved id values (<missing>, |, ^, or backtick) instead of rejecting them")
			cmd.Flags().StringArrayVar(&flags.labels, "label", nil, "Additional Secret label in key=value form (may be repeated)")
			cmd.Flags().StringVar(&flags.kubecontext, "kubecontext", "", "Kubeconfig context for collision checks")
			cmd.Flags().StringVar(&flags.collisionCheckSecret, "collision-check-secret", "", "Existing Secret name to check for imported API key collisions; may be namespace/name")
			cmd.Flags().StringVar(&flags.collisionCheckSelector, "collision-check-selector", "", "Label selector for existing Secrets to check for imported API key collisions")
			_ = cmd.MarkFlagRequired("file")
			_ = cmd.MarkFlagRequired("secret-name")
		},
		RunE: func(cmd *cobra.Command, args []string) error {
			return runImport(cmd, flags)
		},
	}
}

func runImport(cmd *cobra.Command, flags *importFlags) error {
	if flags.mode != "replace" {
		return fmt.Errorf("--mode %q is not supported; v1 import emits a replacement Secret manifest", flags.mode)
	}
	if flags.output != outputYAML && flags.output != outputJSON {
		return fmt.Errorf("output format %q not supported", flags.output)
	}
	if flags.collisionCheckSecret != "" && flags.collisionCheckSelector != "" {
		return fmt.Errorf("--collision-check-secret and --collision-check-selector are mutually exclusive")
	}
	labels, err := parseLabels(flags.labels)
	if err != nil {
		return err
	}
	namespace, err := kubeutil.LoadNamespace(flags.namespace)
	if err != nil {
		return err
	}
	input, closeInput, err := openInput(flags.file)
	if err != nil {
		return err
	}
	defer closeInput()

	result, err := importCSV(input, importOptions{
		SecretName: flags.secretName,
		Namespace:  namespace,
		Labels:     labels,
		Strict:     flags.strict,
		EscapeIDs:  flags.escapeSpecialChars,
	})
	if err != nil {
		return err
	}
	if flags.collisionCheckSecret != "" || flags.collisionCheckSelector != "" {
		collisionNamespace, collisionSecret := splitNamespacedSecret(namespace, flags.collisionCheckSecret)
		keys, err := loadCollisionKeys(cmd.Context(), collisionNamespace, flags.kubecontext, collisionSecret, flags.collisionCheckSelector)
		if err != nil {
			return err
		}
		if err := checkCollisions(result.ImportedKeys, keys); err != nil {
			return err
		}
	}

	if warning := warnIfPermissive(flags.file); warning != "" {
		fmt.Fprintln(cmd.ErrOrStderr(), "warning:", warning)
	}
	for _, warning := range result.Warnings {
		fmt.Fprintln(cmd.ErrOrStderr(), "warning:", warning)
	}
	if len(result.GeneratedRows) > 0 {
		fmt.Fprintf(cmd.ErrOrStderr(), "notice: generated API keys for rows %s; the emitted Secret manifest contains those key values\n", formatRows(result.GeneratedRows))
	}
	if len(result.Secrets) > 1 {
		fmt.Fprintf(cmd.ErrOrStderr(), "notice: split import into %d Secrets to keep each manifest below %d bytes\n", len(result.Secrets), defaultMaxSerializedSecretSize)
	}

	p, err := printer.New(flags.output)
	if err != nil {
		return err
	}
	return p.Print(cmd.OutOrStdout(), result.Manifest())
}

type generateFlags struct {
	count                  int
	label                  string
	namespace              string
	output                 string
	kubecontext            string
	collisionCheckSecret   string
	collisionCheckSelector string
}

func generateCommand() flag.Command {
	flags := &generateFlags{
		count:  1,
		label:  "key",
		output: outputText,
	}
	return flag.Command{
		Use:   "generate",
		Short: "Generate virtual API keys",
		Long: strings.TrimSpace(`Generate virtual API keys and print them to stdout.

Generated keys use the form sk-<label>-<random>. The label is lowercased,
sanitized, and truncated before it is embedded in the key. Stdout is the only
record of the generated key material; store it securely.`),
		Example: strings.TrimSpace(`
agctl virtualkey generate --label alice
agctl virtualkey generate --count 5 --label batch-import
agctl virtualkey generate --label alice --collision-check-secret default/agw-virtual-keys
`),
		Args: cobra.NoArgs,
		AddFlags: func(cmd *cobra.Command) {
			cmd.Flags().IntVarP(&flags.count, "count", "n", flags.count, "Number of keys to generate")
			cmd.Flags().StringVar(&flags.label, "label", flags.label, "Human-readable label to sanitize and embed in generated keys")
			cmd.Flags().StringVar(&flags.namespace, "namespace", "", "Namespace for collision checks")
			cmd.Flags().StringVarP(&flags.output, "output", "o", flags.output, "Output format: one of text|json|yaml")
			cmd.Flags().StringVar(&flags.kubecontext, "kubecontext", "", "Kubeconfig context for collision checks")
			cmd.Flags().StringVar(&flags.collisionCheckSecret, "collision-check-secret", "", "Existing Secret name to avoid generated API key collisions; may be namespace/name")
			cmd.Flags().StringVar(&flags.collisionCheckSelector, "collision-check-selector", "", "Label selector for existing Secrets to avoid generated API key collisions")
		},
		RunE: func(cmd *cobra.Command, args []string) error {
			return runGenerate(cmd, flags)
		},
	}
}

func runGenerate(cmd *cobra.Command, flags *generateFlags) error {
	if flags.count < 1 {
		return fmt.Errorf("--count must be greater than 0")
	}
	switch flags.output {
	case outputText, outputJSON, outputYAML:
	default:
		return fmt.Errorf("output format %q not supported", flags.output)
	}
	if flags.collisionCheckSecret != "" && flags.collisionCheckSelector != "" {
		return fmt.Errorf("--collision-check-secret and --collision-check-selector are mutually exclusive")
	}

	namespace, secretName := splitNamespacedSecret(flags.namespace, flags.collisionCheckSecret)
	if namespace == "" {
		var err error
		namespace, err = kubeutil.LoadNamespace("")
		if err != nil {
			return err
		}
	}

	var existing []existingKey
	var err error
	if secretName != "" || flags.collisionCheckSelector != "" {
		existing, err = loadCollisionKeys(cmd.Context(), namespace, flags.kubecontext, secretName, flags.collisionCheckSelector)
		if err != nil {
			return err
		}
	}

	keys := make([]string, 0, flags.count)
	for i := 0; i < flags.count; i++ {
		label := flags.label
		if flags.count > 1 {
			label = fmt.Sprintf("%s-%04d", flags.label, i+1)
		}
		key, err := generateNonCollidingKey(label, existing)
		if err != nil {
			return err
		}
		keys = append(keys, key)
		existing = append(existing, existingKey{Entry: fmt.Sprintf("generated-%d", i), Key: key})
	}

	if flags.output == outputText {
		for _, key := range keys {
			fmt.Fprintln(cmd.OutOrStdout(), key)
		}
		return nil
	}
	p, err := printer.New(flags.output)
	if err != nil {
		return err
	}
	return p.Print(cmd.OutOrStdout(), keys)
}

func generateNonCollidingKey(label string, existing []existingKey) (string, error) {
	for range 3 {
		key, err := generateAPIKey(label)
		if err != nil {
			return "", err
		}
		if err := checkCollisions(map[int]string{1: key}, existing); err == nil {
			return key, nil
		}
	}
	return "", fmt.Errorf("generated key collided with existing data after 3 attempts")
}

func openInput(path string) (io.Reader, func(), error) {
	if path == "-" {
		return os.Stdin, func() {}, nil
	}
	file, err := os.Open(path)
	if err != nil {
		return nil, func() {}, fmt.Errorf("open CSV file %q: %w", path, err)
	}
	return file, func() { _ = file.Close() }, nil
}

func loadCollisionKeys(ctx context.Context, namespace, kubecontext, secretName, selector string) ([]existingKey, error) {
	client, err := newCoreClient(kubecontext)
	if err != nil {
		return nil, err
	}
	return loadExistingKeys(ctx, client.CoreV1().Secrets(namespace), secretName, selector)
}

func newCoreClient(kubecontext string) (kubernetes.Interface, error) {
	loadingRules := clientcmd.NewDefaultClientConfigLoadingRules()
	if kubeconfig := flag.Kubeconfig(); kubeconfig != "" {
		loadingRules.ExplicitPath = kubeconfig
	}
	overrides := &clientcmd.ConfigOverrides{CurrentContext: kubecontext}
	config, err := clientcmd.NewNonInteractiveDeferredLoadingClientConfig(loadingRules, overrides).ClientConfig()
	if err != nil {
		return nil, fmt.Errorf("failed to build Kubernetes client config: %w", err)
	}
	client, err := kubernetes.NewForConfig(config)
	if err != nil {
		return nil, fmt.Errorf("failed to build Kubernetes client: %w", err)
	}
	return client, nil
}

func splitNamespacedSecret(namespace, secret string) (string, string) {
	ns, name, ok := strings.Cut(secret, "/")
	if !ok {
		return namespace, secret
	}
	return ns, name
}
