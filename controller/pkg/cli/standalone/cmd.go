package standalone

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"sort"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/spf13/cobra"
	utilyaml "k8s.io/apimachinery/pkg/util/yaml"
	"sigs.k8s.io/yaml"
)

const defaultAddress = "http://localhost:15000"

var kinds = []string{
	"modelCatalog",
	"llm.provider",
	"llm.model",
	"llm.virtualModel",
	"llm.apiKey",
	"llm.policy",
	"llm.settings",
	"mcp.target",
	"mcp.policy",
	"mcp.settings",
	"traffic.gateway",
	"traffic.route",
	"traffic.tcpRoute",
	"ui.policy",
	"frontend.policy",
}

type client struct {
	address string
	http    *http.Client
	// resolve sets address and auth from flags, environment, and the config file.
	resolve func() error
}

type resource struct {
	Kind      string          `json:"kind" yaml:"kind"`
	ID        string          `json:"id,omitempty" yaml:"id,omitempty"`
	Value     json.RawMessage `json:"value" yaml:"value"`
	Revision  *int64          `json:"revision,omitempty" yaml:"revision,omitempty"`
	CreatedAt string          `json:"createdAt,omitempty" yaml:"createdAt,omitempty"`
	UpdatedAt string          `json:"updatedAt,omitempty" yaml:"updatedAt,omitempty"`
	// Result is set on write responses: created, updated, or unchanged.
	Result string `json:"result,omitempty" yaml:"result,omitempty"`
}

type resourceList struct {
	Resources []resource `json:"resources" yaml:"resources"`
}

func Command() *cobra.Command {
	var address, gateway string
	c := &client{http: &http.Client{
		Timeout: 10 * time.Second,
		// The admin API never redirects on success; redirects are login flows, reported as errors.
		CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },
	}}
	cmd := &cobra.Command{
		Use:     "standalone",
		Aliases: []string{"s"},
		Short:   "Manage a standalone agentgateway",
		Long: `Manage configuration resources through a standalone agentgateway admin API.

The gateway address is taken from --gateway, --address, $AGCTL_STANDALONE_ADDRESS,
the current gateway in the config file, or ` + defaultAddress + `, in that order.

The config file is read from $AGCTL_CONFIG, or agctl/config.yaml in
the user config directory (typically ~/.config/agctl/config.yaml). Gateways may
authenticate with a kubectl credential plugin, using the kubeconfig exec schema:

  current: prod
  gateways:
    local:
      url: http://localhost:15000
    prod:
      url: https://agw.example.com
      exec:
        apiVersion: client.authentication.k8s.io/v1
        command: kubectl
        args: [oidc-login, get-token, --oidc-issuer-url=https://issuer.example.com, --oidc-client-id=agctl]`,
		Example: `  agctl standalone get llm.models
  agctl standalone get all -o yaml
  agctl standalone apply -f resources.yaml
  agctl standalone delete llm.models my-model
  agctl standalone --gateway prod get all`,
		PersistentPreRunE: func(*cobra.Command, []string) error {
			return c.resolve()
		},
	}
	c.resolve = func() error {
		c.http.Transport = nil
		addr := os.Getenv("AGCTL_STANDALONE_ADDRESS")
		if cmd.PersistentFlags().Changed("address") {
			addr = address
		}
		if gateway != "" || addr == "" {
			gw, err := loadGateway(gateway)
			if err != nil {
				return err
			}
			addr = defaultAddress
			if gw != nil {
				addr = gw.URL
				if c.http.Transport, err = gw.transport(); err != nil {
					return fmt.Errorf("configure gateway auth: %w", err)
				}
			}
		}
		c.address = strings.TrimRight(addr, "/")
		if _, err := url.ParseRequestURI(c.address); err != nil {
			return fmt.Errorf("invalid gateway address: %w", err)
		}
		return nil
	}
	cmd.PersistentFlags().StringVar(&address, "address", "", "Standalone agentgateway admin address")
	cmd.PersistentFlags().StringVar(&gateway, "gateway", "", "Gateway name from the config file")
	cmd.MarkFlagsMutuallyExclusive("address", "gateway")
	cmd.AddCommand(getCommand(c), applyCommand(c), deleteCommand(c), logsCommand(c), analyticsCommand(c), keysCommand(c), budgetsCommand(c))
	return cmd
}

func getCommand(c *client) *cobra.Command {
	var output string
	var noMetadata bool
	cmd := &cobra.Command{
		Use:          "get KIND [NAME]",
		SilenceUsage: true,
		Short:        "Display one or more resources",
		Args:         cobra.RangeArgs(1, 2),
		ValidArgsFunction: func(_ *cobra.Command, args []string, _ string) ([]string, cobra.ShellCompDirective) {
			if len(args) == 0 {
				return kindCompletions(true), cobra.ShellCompDirectiveNoFileComp
			}
			if len(args) == 1 && canonicalKind(args[0]) != "" && args[0] != "all" {
				return completeResourceNames(c, args[0]), cobra.ShellCompDirectiveNoFileComp
			}
			return nil, cobra.ShellCompDirectiveNoFileComp
		},
		RunE: func(cmd *cobra.Command, args []string) error {
			kind, err := requireKind(args[0], true)
			if err != nil {
				return err
			}
			var result resourceList
			path := "/api/config/resources"
			if kind != "all" {
				path += "/" + url.PathEscape(kind)
			}
			if err := c.do(cmd.Context(), http.MethodGet, path, nil, &result); err != nil {
				return err
			}
			if len(args) == 2 {
				result.Resources = filterResources(result.Resources, args[1])
				if len(result.Resources) == 0 {
					return fmt.Errorf("resource %s/%s not found", kind, args[1])
				}
			}
			if noMetadata {
				for i := range result.Resources {
					if !identifiedByID(result.Resources[i].Kind) {
						result.Resources[i].ID = ""
					}
					result.Resources[i].Revision = nil
					result.Resources[i].CreatedAt = ""
					result.Resources[i].UpdatedAt = ""
				}
			}
			return printResources(cmd.OutOrStdout(), result, output)
		},
	}
	cmd.Flags().StringVarP(&output, "output", "o", "table", "Output format: table, yaml, or json")
	cmd.Flags().BoolVarP(&noMetadata, "no-metadata", "n", false, "Omit server-managed fields for use with apply")
	_ = cmd.RegisterFlagCompletionFunc("output", cobra.FixedCompletions([]string{"table", "yaml", "json"}, cobra.ShellCompDirectiveNoFileComp))
	return cmd
}

func applyCommand(c *client) *cobra.Command {
	var filename string
	cmd := &cobra.Command{
		Use:          "apply -f FILENAME",
		SilenceUsage: true,
		Short:        "Apply resources from YAML or JSON",
		Long: `Apply resources from YAML or JSON.

Each resource has a kind and value, for example:

  kind: llm.model
  value:
    name: llama
    provider: ollama
    params:
      model: llama3`,
		Args: cobra.NoArgs,
		RunE: func(cmd *cobra.Command, _ []string) error {
			if filename == "" {
				return fmt.Errorf("-f is required")
			}
			resources, err := readResources(filename)
			if err != nil {
				return err
			}
			byKind := map[string][]resource{}
			for _, r := range resources {
				kind, err := requireKind(r.Kind, false)
				if err != nil {
					return err
				}
				if len(r.Value) == 0 || bytes.Equal(r.Value, []byte("null")) {
					return fmt.Errorf("resource %q has no value", r.Kind)
				}
				byKind[kind] = append(byKind[kind], r)
			}
			orderedKinds := make([]string, 0, len(byKind))
			for kind := range byKind {
				orderedKinds = append(orderedKinds, kind)
			}
			sort.Strings(orderedKinds)
			for _, kind := range orderedKinds {
				body := struct {
					Resources []struct {
						Value json.RawMessage `json:"value"`
					} `json:"resources"`
				}{}
				var results []resource
				for _, r := range byKind[kind] {
					if r.ID != "" && identifiedByID(kind) {
						var result resourceList
						item := struct {
							Value json.RawMessage `json:"value"`
						}{Value: r.Value}
						path := "/api/config/resources/" + url.PathEscape(kind) + "/" + url.PathEscape(r.ID)
						if err := c.do(cmd.Context(), http.MethodPut, path, item, &result); err != nil {
							return err
						}
						results = append(results, result.Resources...)
						continue
					}
					body.Resources = append(body.Resources, struct {
						Value json.RawMessage `json:"value"`
					}{Value: r.Value})
				}
				if len(body.Resources) > 0 {
					var result resourceList
					if err := c.do(cmd.Context(), http.MethodPut, "/api/config/resources/"+url.PathEscape(kind), body, &result); err != nil {
						return err
					}
					results = append(results, result.Resources...)
				}
				for _, r := range results {
					status := r.Result
					// File-backed gateways do not report a result.
					if status == "" {
						status = "applied"
					}
					fmt.Fprintf(cmd.OutOrStdout(), "%s/%s %s\n", r.Kind, r.ID, status)
				}
			}
			return nil
		},
	}
	cmd.Flags().StringVarP(&filename, "filename", "f", "", "YAML or JSON file (use - for stdin)")
	_ = cmd.MarkFlagFilename("filename", "yaml", "yml", "json")
	return cmd
}

func deleteCommand(c *client) *cobra.Command {
	cmd := &cobra.Command{
		Use:          "delete KIND NAME",
		SilenceUsage: true,
		Short:        "Delete a resource",
		Args:         cobra.ExactArgs(2),
		ValidArgsFunction: func(_ *cobra.Command, args []string, _ string) ([]string, cobra.ShellCompDirective) {
			if len(args) == 0 {
				return kindCompletions(false), cobra.ShellCompDirectiveNoFileComp
			}
			if len(args) == 1 && canonicalKind(args[0]) != "" {
				return completeResourceNames(c, args[0]), cobra.ShellCompDirectiveNoFileComp
			}
			return nil, cobra.ShellCompDirectiveNoFileComp
		},
		RunE: func(cmd *cobra.Command, args []string) error {
			kind, err := requireKind(args[0], false)
			if err != nil {
				return err
			}
			var result resourceList
			if err := c.do(cmd.Context(), http.MethodGet, "/api/config/resources/"+url.PathEscape(kind), nil, &result); err != nil {
				return err
			}
			matches := filterResources(result.Resources, args[1])
			if len(matches) == 0 {
				return fmt.Errorf("resource %s/%s not found", kind, args[1])
			}
			if len(matches) > 1 {
				return fmt.Errorf("resource name %s/%s is ambiguous; use its ID", kind, args[1])
			}
			id := matches[0].ID
			if err := c.do(cmd.Context(), http.MethodDelete, "/api/config/resources/"+url.PathEscape(kind)+"/"+url.PathEscape(id), nil, nil); err != nil {
				return err
			}
			fmt.Fprintf(cmd.OutOrStdout(), "%s/%s deleted\n", kind, resourceName(matches[0]))
			return nil
		},
	}
	return cmd
}

func (c *client) do(ctx context.Context, method, path string, input, output any) error {
	var body io.Reader
	if input != nil {
		data, err := json.Marshal(input)
		if err != nil {
			return err
		}
		body = bytes.NewReader(data)
	}
	req, err := http.NewRequestWithContext(ctx, method, c.address+path, body)
	if err != nil {
		return err
	}
	if input != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("request failed: %w", err)
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	if err := responseError(resp, data); err != nil {
		return err
	}
	if output != nil && len(data) > 0 {
		if err := json.Unmarshal(data, output); err != nil {
			if ct := resp.Header.Get("Content-Type"); !strings.Contains(ct, "json") {
				return fmt.Errorf("expected a JSON response from %s, got %q; is this an agentgateway admin address?", c.address, ct)
			}
			return fmt.Errorf("decode response: %w", err)
		}
	}
	return nil
}

func responseError(resp *http.Response, data []byte) error {
	switch {
	case resp.StatusCode >= 300 && resp.StatusCode < 400:
		return fmt.Errorf("server returned %s redirecting to %s; the gateway likely requires login. %s",
			resp.Status, resp.Header.Get("Location"), authHint())
	case resp.StatusCode == http.StatusUnauthorized || resp.StatusCode == http.StatusForbidden:
		return fmt.Errorf("server returned %s. %s", resp.Status, authHint())
	case resp.StatusCode < 200 || resp.StatusCode >= 300:
		if strings.Contains(resp.Header.Get("Content-Type"), "html") {
			return fmt.Errorf("server returned %s", resp.Status)
		}
		message := strings.TrimSpace(string(data))
		var apiMessage string
		if json.Unmarshal(data, &apiMessage) == nil {
			message = apiMessage
		}
		return fmt.Errorf("server returned %s: %s", resp.Status, message)
	}
	return nil
}

func authHint() string {
	path, err := configPath()
	if err != nil {
		path = "the agctl standalone config file"
	}
	return fmt.Sprintf("Configure an exec credential plugin for this gateway in %s (see 'agctl standalone --help').", path)
}

func readResources(filename string) ([]resource, error) {
	var input io.Reader
	if filename == "-" {
		input = os.Stdin
	} else {
		file, err := os.Open(filename)
		if err != nil {
			return nil, err
		}
		defer file.Close()
		input = file
	}
	decoder := utilyaml.NewYAMLOrJSONDecoder(input, 4096)
	var resources []resource
	for {
		var raw json.RawMessage
		if err := decoder.Decode(&raw); err != nil {
			if err == io.EOF {
				break
			}
			return nil, fmt.Errorf("decode %s: %w", filename, err)
		}
		if len(bytes.TrimSpace(raw)) == 0 || bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			continue
		}
		var items []resource
		if bytes.HasPrefix(bytes.TrimSpace(raw), []byte("[")) {
			if err := json.Unmarshal(raw, &items); err != nil {
				return nil, fmt.Errorf("decode %s: %w", filename, err)
			}
		} else {
			var item resource
			if err := json.Unmarshal(raw, &item); err != nil {
				return nil, fmt.Errorf("decode %s: %w", filename, err)
			}
			items = []resource{item}
		}
		for _, item := range items {
			if item.Kind == "" {
				return nil, fmt.Errorf("resource in %s has no kind", filename)
			}
		}
		resources = append(resources, items...)
	}
	if len(resources) == 0 {
		return nil, fmt.Errorf("no resources found in %s", filename)
	}
	return resources, nil
}

func printResources(w io.Writer, result resourceList, output string) error {
	switch output {
	case "table":
		tw := tabwriter.NewWriter(w, 0, 4, 2, ' ', 0)
		allModels := len(result.Resources) > 0
		for _, r := range result.Resources {
			allModels = allModels && r.Kind == "llm.model"
		}
		if allModels {
			fmt.Fprintln(tw, "NAME\tPROVIDER\tMODEL\tID")
			for _, r := range result.Resources {
				var value struct {
					Provider any `json:"provider"`
					Params   struct {
						Model string `json:"model"`
					} `json:"params"`
				}
				_ = json.Unmarshal(r.Value, &value)
				provider := fmt.Sprint(value.Provider)
				if encoded, err := json.Marshal(value.Provider); err == nil && value.Provider != nil {
					provider = strings.Trim(string(encoded), `"`)
				}
				model := value.Params.Model
				if model == "" {
					model = "<incoming>"
				}
				fmt.Fprintf(tw, "%s\t%s\t%s\t%s\n", resourceName(r), provider, model, r.ID)
			}
		} else {
			fmt.Fprintln(tw, "KIND\tNAME\tID")
			for _, r := range result.Resources {
				fmt.Fprintf(tw, "%s\t%s\t%s\n", r.Kind, resourceName(r), r.ID)
			}
		}
		return tw.Flush()
	case "json":
		if result.Resources == nil {
			result.Resources = []resource{}
		}
		data, err := json.MarshalIndent(result.Resources, "", "  ")
		if err != nil {
			return err
		}
		fmt.Fprintf(w, "%s\n", data)
		return nil
	case "yaml":
		for i, r := range result.Resources {
			data, err := yaml.Marshal(r)
			if err != nil {
				return err
			}
			if i > 0 {
				fmt.Fprintln(w, "---")
			}
			fmt.Fprint(w, string(data))
		}
		return nil
	default:
		return fmt.Errorf("output format %q not supported", output)
	}
}

// identifiedByID reports whether a kind's ID is not derived from its value. Updates to these
// must target the existing ID; the kind-level PUT would create a new API key.
func identifiedByID(kind string) bool {
	return kind == "llm.apiKey" || strings.HasSuffix(kind, ".policy")
}

func requireKind(input string, allowAll bool) (string, error) {
	if allowAll && input == "all" {
		return input, nil
	}
	if kind := canonicalKind(input); kind != "" {
		return kind, nil
	}
	return "", fmt.Errorf("unsupported resource kind %q", input)
}

func canonicalKind(input string) string {
	for _, kind := range kinds {
		// Plural forms are accepted as aliases, e.g. llm.policies for llm.policy.
		plural := kind + "s"
		if strings.HasSuffix(kind, "y") {
			plural = strings.TrimSuffix(kind, "y") + "ies"
		} else if strings.HasSuffix(kind, "s") {
			plural = kind
		}
		if input == kind || input == plural {
			return kind
		}
	}
	return ""
}

func kindCompletions(includeAll bool) []string {
	result := append([]string(nil), kinds...)
	if includeAll {
		result = append(result, "all")
	}
	sort.Strings(result)
	return result
}

func completeResourceNames(c *client, inputKind string) []string {
	kind := canonicalKind(inputKind)
	if kind == "" || c.resolve() != nil {
		return nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	var result resourceList
	if c.do(ctx, http.MethodGet, "/api/config/resources/"+url.PathEscape(kind), nil, &result) != nil {
		return nil
	}
	names := make([]string, 0, len(result.Resources))
	for _, r := range result.Resources {
		names = append(names, resourceName(r))
	}
	sort.Strings(names)
	return names
}

func resourceName(r resource) string {
	var value struct {
		Name     string `json:"name"`
		ID       string `json:"id"`
		Metadata struct {
			Name string `json:"name"`
		} `json:"metadata"`
	}
	if json.Unmarshal(r.Value, &value) == nil {
		if value.Name != "" {
			return value.Name
		}
		if value.ID != "" {
			return value.ID
		}
		// llm.apiKey resources keep their name in metadata.
		if value.Metadata.Name != "" {
			return value.Metadata.Name
		}
	}
	return r.ID
}

func filterResources(resources []resource, name string) []resource {
	var matches []resource
	for _, r := range resources {
		if r.ID == name {
			return []resource{r}
		}
		if resourceName(r) == name {
			matches = append(matches, r)
		}
	}
	return matches
}
