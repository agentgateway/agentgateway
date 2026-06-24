package kubeutil

import (
	"context"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"strconv"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
	"k8s.io/client-go/tools/clientcmd"
	"k8s.io/client-go/tools/portforward"
	"k8s.io/client-go/transport/spdy"
	"k8s.io/streaming/pkg/httpstream"
	gatewayapiclient "sigs.k8s.io/gateway-api/pkg/client/clientset/versioned"

	"github.com/agentgateway/agentgateway/controller/pkg/cli/flag"
)

const defaultLocalAddress = "localhost"

type CLIClient interface {
	Kube() kubernetes.Interface
	GatewayAPI() gatewayapiclient.Interface
	AgentgatewayRequest(ctx context.Context, podName, podNamespace, method, path string, port int) ([]byte, error)
	NewPortForwarder(podName, namespace, localAddress string, localPort, podPort int) (PortForwarder, error)
}

type PortForwarder interface {
	Start() error
	Address() string
	Close()
	ErrChan() <-chan error
	WaitForStop()
}

type client struct {
	restConfig *rest.Config
	kube       kubernetes.Interface
	gatewayAPI gatewayapiclient.Interface
	http       *http.Client
}

func NewCLIClient() (CLIClient, error) {
	loadingRules := clientcmd.NewDefaultClientConfigLoadingRules()
	if kubeconfig := flag.Kubeconfig(); kubeconfig != "" {
		loadingRules.ExplicitPath = kubeconfig
	}

	restConfig, err := clientcmd.NewNonInteractiveDeferredLoadingClientConfig(loadingRules, &clientcmd.ConfigOverrides{}).ClientConfig()
	if err != nil {
		return nil, fmt.Errorf("failed to build Kubernetes client config: %w", err)
	}
	restConfig.QPS = 50
	restConfig.Burst = 100

	kubeClient, err := kubernetes.NewForConfig(restConfig)
	if err != nil {
		return nil, fmt.Errorf("failed to build Kubernetes client: %w", err)
	}
	gatewayClient, err := gatewayapiclient.NewForConfig(restConfig)
	if err != nil {
		return nil, fmt.Errorf("failed to build Gateway API client: %w", err)
	}

	return &client{
		restConfig: restConfig,
		kube:       kubeClient,
		gatewayAPI: gatewayClient,
		http:       http.DefaultClient,
	}, nil
}

func (c *client) Kube() kubernetes.Interface {
	return c.kube
}

func (c *client) GatewayAPI() gatewayapiclient.Interface {
	return c.gatewayAPI
}

func (c *client) AgentgatewayRequest(ctx context.Context, podName, podNamespace, method, path string, port int) ([]byte, error) {
	formatError := func(err error) error {
		return fmt.Errorf("failure running port forward process: %w", err)
	}

	fw, err := c.NewPortForwarder(podName, podNamespace, "", 0, port)
	if err != nil {
		return nil, err
	}
	if err = fw.Start(); err != nil {
		return nil, formatError(err)
	}
	defer fw.Close()

	req, err := http.NewRequestWithContext(ctx, method, fmt.Sprintf("http://%s/%s", fw.Address(), path), nil)
	if err != nil {
		return nil, formatError(err)
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, formatError(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("unexpected status code: %d", resp.StatusCode)
	}
	out, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, formatError(err)
	}
	return out, nil
}

func (c *client) NewPortForwarder(podName, namespace, localAddress string, localPort, podPort int) (PortForwarder, error) {
	if localAddress == "" {
		localAddress = defaultLocalAddress
	}
	return &forwarder{
		stopCh:       make(chan struct{}),
		restConfig:   c.restConfig,
		kube:         c.kube,
		podName:      podName,
		namespace:    namespace,
		localAddress: localAddress,
		localPort:    localPort,
		podPort:      podPort,
	}, nil
}

type forwarder struct {
	stopCh       chan struct{}
	restConfig   *rest.Config
	kube         kubernetes.Interface
	podName      string
	namespace    string
	localAddress string
	localPort    int
	podPort      int
	errCh        chan error
}

func (f *forwarder) Start() error {
	f.errCh = make(chan error, 1)
	readyCh := make(chan struct{}, 1)

	var fw *portforward.PortForwarder
	go func() {
		fwd, err := f.buildK8sPortForwarder(readyCh)
		if err != nil {
			f.errCh <- fmt.Errorf("building port forwarder: %w", err)
			return
		}
		fw = fwd
		if err = fw.ForwardPorts(); err != nil {
			f.errCh <- fmt.Errorf("port forward: %w", err)
			return
		}
		f.errCh <- nil
	}()

	select {
	case err := <-f.errCh:
		return fmt.Errorf("failure running port forward process: %w", err)
	case <-readyCh:
		ports, err := fw.GetPorts()
		if err != nil {
			return fmt.Errorf("failed to get ports: %w", err)
		}
		if len(ports) == 0 {
			return fmt.Errorf("got no ports")
		}
		f.localPort = int(ports[0].Local)
		return nil
	}
}

func (f *forwarder) Address() string {
	return net.JoinHostPort(f.localAddress, strconv.Itoa(f.localPort))
}

func (f *forwarder) Close() {
	select {
	case <-f.stopCh:
	default:
		close(f.stopCh)
	}
}

func (f *forwarder) ErrChan() <-chan error {
	return f.errCh
}

func (f *forwarder) WaitForStop() {
	<-f.stopCh
}

func (f *forwarder) buildK8sPortForwarder(readyCh chan struct{}) (*portforward.PortForwarder, error) {
	pod, err := f.kube.CoreV1().Pods(f.namespace).Get(context.Background(), f.podName, metav1.GetOptions{})
	if err != nil {
		return nil, fmt.Errorf("failed retrieving pod/%s in namespace %q: %w", f.podName, f.namespace, err)
	}
	if pod.Status.Phase != corev1.PodRunning {
		return nil, fmt.Errorf("pod is not running. Status=%v", pod.Status.Phase)
	}

	req := f.kube.CoreV1().RESTClient().Post().Resource("pods").Namespace(f.namespace).Name(f.podName).SubResource("portforward")
	roundTripper, upgrader, err := spdy.RoundTripperFor(f.restConfig)
	if err != nil {
		return nil, fmt.Errorf("failure creating roundtripper: %w", err)
	}
	dialer := spdy.NewDialer(upgrader, &http.Client{Transport: roundTripper}, http.MethodPost, req.URL())
	tunnelingDialer, err := portforward.NewSPDYOverWebsocketDialer(req.URL(), f.restConfig)
	if err != nil {
		return nil, fmt.Errorf("failure creating websocket tunneling dialer: %w", err)
	}
	dialer = portforward.NewFallbackDialer(tunnelingDialer, dialer, func(err error) bool {
		return httpstream.IsUpgradeFailure(err) || httpstream.IsHTTPSProxyError(err)
	})

	fw, err := portforward.NewOnAddresses(
		dialer,
		[]string{f.localAddress},
		[]string{fmt.Sprintf("%d:%d", f.localPort, f.podPort)},
		f.stopCh,
		readyCh,
		io.Discard,
		os.Stderr,
	)
	if err != nil {
		return nil, fmt.Errorf("failed establishing port-forward: %w", err)
	}
	return fw, nil
}
