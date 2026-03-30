package config

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"

	"github.com/spf13/cobra"
	"istio.io/istio/pkg/kube"
	"k8s.io/client-go/tools/clientcmd"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
)

const (
	defaultProxyAdminPort = 15000
	shortOutput           = "short"
	jsonOutput            = "json"
	yamlOutput            = "yaml"
)

type commonFlags struct {
	namespace      string
	configDumpFile string
	proxyAdminPort int
	outputFormat   string
}

type configDumpSource struct {
	ResourceName string
	Namespace    string
	PodName      string
	ConfigDump   json.RawMessage
	File         string
}

func Command() flag.Command {
	common := &commonFlags{
		proxyAdminPort: defaultProxyAdminPort,
		outputFormat:   shortOutput,
	}

	return flag.Command{
		Use:     "config",
		Aliases: []string{"c", "cfg"},
		Short:   "Retrieve Agentgateway configuration for a resource",
		Long:    "Retrieve Agentgateway configuration for a resource.",
		AddPersistentFlags: func(cmd *cobra.Command) {
			common.attach(cmd)
		},
		Children: []flag.CommandBuilder{
			func() flag.Command { return allCommand(common) },
		},
	}
}

func (c *commonFlags) attach(cmd *cobra.Command) {
	cmd.PersistentFlags().StringVarP(&c.namespace, "namespace", "n", "", "Namespace to use when resolving resources")
	cmd.PersistentFlags().StringVarP(&c.configDumpFile, "file", "f", "", "Agentgateway config dump JSON file")
	cmd.PersistentFlags().IntVar(&c.proxyAdminPort, "proxy-admin-port", c.proxyAdminPort, "Envoy admin port")
	cmd.PersistentFlags().StringVarP(&c.outputFormat, "output", "o", c.outputFormat, "Output format: one of short|json|yaml")
}

func (c *commonFlags) validateArgs(cmd *cobra.Command, args []string) error {
	if len(args) > 1 {
		return fmt.Errorf("accepts at most 1 arg(s), received %d", len(args))
	}
	if c.configDumpFile != "" && len(args) == 1 {
		cmd.Println(cmd.UsageString())
		return fmt.Errorf("at most one of --file or resource name may be passed")
	}
	if c.proxyAdminPort < 1 || c.proxyAdminPort > 65535 {
		return fmt.Errorf("invalid --proxy-admin-port %d", c.proxyAdminPort)
	}
	switch c.outputFormat {
	case shortOutput, jsonOutput, yamlOutput:
	default:
		return fmt.Errorf("output format %q not supported", c.outputFormat)
	}
	return nil
}

func loadConfigDumpSource(ctx context.Context, common *commonFlags, args []string) (*configDumpSource, error) {
	if common.configDumpFile != "" {
		data, err := readFile(common.configDumpFile)
		if err != nil {
			return nil, fmt.Errorf("failed to read config dump file %s: %w", common.configDumpFile, err)
		}
		return &configDumpSource{
			ConfigDump: data,
			File:       common.configDumpFile,
		}, nil
	}

	namespace, err := loadNamespace(common.namespace)
	if err != nil {
		return nil, err
	}

	kubeClient, err := newCLIClient()
	if err != nil {
		return nil, err
	}

	resourceName, err := resolveResourceName(ctx, kubeClient, namespace, args)
	if err != nil {
		return nil, err
	}

	podName, podNamespace, err := resolvePodForResource(kubeClient, resourceName, namespace)
	if err != nil {
		return nil, err
	}

	configDump, err := extractConfigDump(kubeClient, podName, podNamespace, common.proxyAdminPort)
	if err != nil {
		return nil, err
	}

	return &configDumpSource{
		ResourceName: resourceName,
		Namespace:    podNamespace,
		PodName:      podName,
		ConfigDump:   configDump,
	}, nil
}

func loadNamespace(namespaceOverride string) (string, error) {
	loadingRules := clientcmd.NewDefaultClientConfigLoadingRules()
	if kubeconfig := flag.Kubeconfig(); kubeconfig != "" {
		loadingRules.ExplicitPath = kubeconfig
	}

	configLoader := clientcmd.NewNonInteractiveDeferredLoadingClientConfig(loadingRules, &clientcmd.ConfigOverrides{})
	namespace, _, err := configLoader.Namespace()
	if err != nil {
		return "", fmt.Errorf("failed to resolve namespace from kubeconfig: %w", err)
	}
	if namespaceOverride != "" {
		namespace = namespaceOverride
	}

	return namespace, nil
}

func newCLIClient() (kube.CLIClient, error) {
	restConfig, err := kube.DefaultRestConfig(flag.Kubeconfig(), "")
	if err != nil {
		return nil, fmt.Errorf("failed to build Kubernetes client config: %w", err)
	}

	restConfig.QPS = 50
	restConfig.Burst = 100

	return kube.NewCLIClient(kube.NewClientConfigForRestConfig(restConfig))
}

func readFile(filename string) ([]byte, error) {
	file := os.Stdin
	if filename != "-" {
		var err error
		file, err = os.Open(filename)
		if err != nil {
			return nil, err
		}
	}
	defer file.Close()

	return io.ReadAll(file)
}
