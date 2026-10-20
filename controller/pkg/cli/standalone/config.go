package standalone

import (
	"errors"
	"fmt"
	"io/fs"
	"net/http"
	"os"
	"path/filepath"

	"k8s.io/client-go/pkg/apis/clientauthentication"
	"k8s.io/client-go/plugin/pkg/client/auth/exec"
	clientcmdapi "k8s.io/client-go/tools/clientcmd/api"
	"k8s.io/client-go/transport"
	"sigs.k8s.io/yaml"
)

// config is the agctl standalone config file. Each gateway's exec block uses the kubeconfig
// credential plugin schema, so existing kubectl auth plugins work unchanged.
type config struct {
	Current  string                   `json:"current"`
	Gateways map[string]gatewayConfig `json:"gateways"`
}

type gatewayConfig struct {
	URL  string                   `json:"url"`
	Exec *clientcmdapi.ExecConfig `json:"exec,omitempty"`
}

func configPath() (string, error) {
	if path := os.Getenv("AGCTL_CONFIG"); path != "" {
		return path, nil
	}
	dir, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "agctl", "config.yaml"), nil
}

// loadGateway returns the named gateway from the config file, or the current one if name is empty.
// It returns nil if no config file exists and no gateway was explicitly requested.
func loadGateway(name string) (*gatewayConfig, error) {
	path, err := configPath()
	if err != nil {
		return nil, err
	}
	data, err := os.ReadFile(path)
	if errors.Is(err, fs.ErrNotExist) && name == "" {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var cfg config
	if err := yaml.UnmarshalStrict(data, &cfg); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	if name == "" {
		name = cfg.Current
	}
	if name == "" {
		return nil, nil
	}
	gw, found := cfg.Gateways[name]
	if !found {
		return nil, fmt.Errorf("gateway %q not found in %s", name, path)
	}
	if gw.URL == "" {
		return nil, fmt.Errorf("gateway %q in %s has no url", name, path)
	}
	return &gw, nil
}

func (g *gatewayConfig) transport() (http.RoundTripper, error) {
	if g.Exec == nil {
		return http.DefaultTransport, nil
	}
	if g.Exec.InteractiveMode == "" {
		g.Exec.InteractiveMode = clientcmdapi.IfAvailableExecInteractiveMode
	}
	auth, err := exec.GetAuthenticator(g.Exec, &clientauthentication.Cluster{Server: g.URL})
	if err != nil {
		return nil, err
	}
	tc := &transport.Config{}
	if err := auth.UpdateTransportConfig(tc); err != nil {
		return nil, err
	}
	return transport.New(tc)
}
