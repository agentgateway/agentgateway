//go:build conformance

package conformance_test

import (
	"context"
	"fmt"
	"net/netip"
	"slices"
	"strings"
	"testing"

	"istio.io/istio/pkg/ptr"
	corev1 "k8s.io/api/core/v1"
	apiextensionsclient "k8s.io/apiextensions-apiserver/pkg/client/clientset/clientset"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
	"k8s.io/apimachinery/pkg/util/sets"
	"k8s.io/client-go/dynamic"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
	"sigs.k8s.io/controller-runtime/pkg/client"
	"sigs.k8s.io/controller-runtime/pkg/client/config"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
	"sigs.k8s.io/gateway-api/conformance"
	"sigs.k8s.io/gateway-api/conformance/utils/suite"
	"sigs.k8s.io/gateway-api/pkg/features"
	"sigs.k8s.io/yaml"

	_ "sigs.k8s.io/gateway-api-inference-extension/conformance"
)

func TestConformance(t *testing.T) {
	options := conformance.DefaultOptions(t)

	// Auto-detect the Gateway API channel by checking installed CRDs
	channel, err := detectGatewayAPIChannel()
	if err != nil {
		t.Logf("Failed to detect Gateway API channel, defaulting to experimental: %v", err)
		channel = features.FeatureChannelExperimental
	} else {
		t.Logf("Detected Gateway API channel: %s", channel)
	}

	// Configure profiles and exempt features based on detected channel
	profiles := sets.New(suite.GatewayGRPCConformanceProfileName, suite.GatewayHTTPConformanceProfileName)
	if channel == features.FeatureChannelExperimental {
		profiles.Insert(suite.GatewayTLSConformanceProfileName)
	}
	options.ConformanceProfiles = profiles.UnsortedList()
	sf, err := fetchGatewayClassSupportedFeatures(options.GatewayClassName)
	if err != nil {
		t.Fatalf("Failed to fetch GatewayClass supported features: %v", err)
	}
	// Gateway API has this detection, but if we exempt any features it turns it off. So copy it over so we can have more control.
	options.SupportedFeatures = sf.UnsortedList()

	if channel == features.FeatureChannelStandard {
		exemptExperimentalFeatures(&options)
	}

	ip, err := guessMetallbAddress()
	if err == nil {
		options.UsableNetworkAddresses = []gwv1.GatewaySpecAddress{
			{
				Type:  ptr.Of(gwv1.IPAddressType),
				Value: ip,
			},
		}
		options.UnusableNetworkAddresses = []gwv1.GatewaySpecAddress{
			{
				Type:  ptr.Of(gwv1.HostnameAddressType),
				Value: "bogus.example.com",
			},
		}
	} else {
		t.Logf("Failed to guess MetalLB address: %v, skipping test", err)
		options.SkipTests = append(options.SkipTests, string(features.GatewayStaticAddressesFeature.Name))
	}
	options.Debug = true

	t.Logf("Running conformance tests with\nprofiles: %+v %v\n", profiles, options.RunTest)
	conformance.RunConformanceWithOptions(t, options)
}

func exemptExperimentalFeatures(options *suite.ConformanceOptions) {
	for _, feature := range features.AllFeatures.UnsortedList() {
		if feature.Channel == features.FeatureChannelExperimental {
			options.ExemptFeatures = append(options.ExemptFeatures, feature.Name)
		}
	}
}

func fetchGatewayClassSupportedFeatures(gatewayClassName string) (suite.FeaturesSet, error) {
	cfg, err := config.GetConfig()
	if err != nil {
		return nil, err
	}
	client, err := client.New(cfg, client.Options{})
	if err != nil {
		return nil, err
	}

	if gatewayClassName == "" {
		return nil, fmt.Errorf("GatewayClass name must be provided to fetch supported features")
	}
	gwc := &gwv1.GatewayClass{}
	if err := client.Get(context.TODO(), types.NamespacedName{Name: gatewayClassName}, gwc); err != nil {
		return nil, fmt.Errorf("fetchGatewayClassSupportedFeatures(): %w", err)
	}

	fs := suite.FeaturesSet{}
	for _, feature := range gwc.Status.SupportedFeatures {
		fs.Insert(features.FeatureName(feature.Name))
	}

	// If Mesh features are populated in the GatewayClass we remove them from the supported features set.
	meshFeatureNames := features.SetsToNamesSet(features.MeshCoreFeatures, features.MeshExtendedFeatures)
	for _, f := range fs.UnsortedList() {
		if meshFeatureNames.Has(f) {
			fs.Delete(f)
			fmt.Printf("WARNING: Mesh feature %q should not be populated in GatewayClass, skipping...", f)
		}
	}
	fmt.Printf("Supported features for GatewayClass %s: %v\n", gatewayClassName, fs.UnsortedList())
	return fs, nil
}

// detectGatewayAPIChannel checks which Gateway API CRDs are installed to determine the channel
func detectGatewayAPIChannel() (string, error) {
	cfg, err := config.GetConfig()
	if err != nil {
		return "", err
	}
	clientset, err := apiextensionsclient.NewForConfig(cfg)
	if err != nil {
		return "", err
	}

	// Check the gateway.networking.k8s.io/channel annotation on HTTPRoute CRD
	crd, err := clientset.ApiextensionsV1().CustomResourceDefinitions().Get(
		context.Background(),
		"httproutes.gateway.networking.k8s.io",
		metav1.GetOptions{},
	)
	if err != nil {
		return "", err
	}

	channel := crd.Annotations["gateway.networking.k8s.io/channel"]
	if channel == "" {
		return "", fmt.Errorf("gateway.networking.k8s.io/channel annotation not found on HTTPRoute CRD")
	}

	return channel, nil
}

func featureSetToCommaSeparatedString(featureSet sets.Set[features.Feature]) string {
	features := []string{}
	for _, feature := range featureSet.UnsortedList() {
		features = append(features, string(feature.Name))
	}
	return strings.Join(features, ",")
}

// guessMetallbAddress looks at MetalLB configuration to guess an IPv4 address it can use.
// It supports two formats:
// 1. IPAddressPool CRD (metallb.io/v1beta1) - newer format
// 2. ConfigMap in metallb-system namespace - older format
// Returns an IPv4 address from the end of the range/list, or an error if not found.
func guessMetallbAddress() (string, error) {
	cfg, err := config.GetConfig()
	if err != nil {
		return "", fmt.Errorf("failed to get kubeconfig: %w", err)
	}

	// Try IPAddressPool CRD format first (newer format)
	address, err := guessFromIPAddressPool(cfg)
	if err == nil {
		return address, nil
	}

	// Fall back to ConfigMap format (older format)
	address, err = guessFromConfigMap(cfg)
	if err != nil {
		return "", fmt.Errorf("failed to guess address from both IPAddressPool and ConfigMap: %w", err)
	}

	return address, nil
}

// guessFromIPAddressPool tries to get an address from IPAddressPool CRD resources
func guessFromIPAddressPool(cfg *rest.Config) (string, error) {
	dynamicClient, err := dynamic.NewForConfig(cfg)
	if err != nil {
		return "", fmt.Errorf("failed to create dynamic client: %w", err)
	}

	gvr := schema.GroupVersionResource{
		Group:    "metallb.io",
		Version:  "v1beta1",
		Resource: "ipaddresspools",
	}

	poolList, err := dynamicClient.Resource(gvr).List(context.Background(), metav1.ListOptions{})
	if err != nil {
		return "", fmt.Errorf("failed to list IPAddressPools: %w", err)
	}

	usedAddresses, err := usedLoadBalancerAddresses(cfg)
	if err != nil {
		return "", fmt.Errorf("failed to list used LoadBalancer addresses: %w", err)
	}

	var pools []metalLBAddressPool
	for _, pool := range poolList.Items {
		addresses, found, err := unstructured.NestedStringSlice(pool.Object, "spec", "addresses")
		if err != nil || !found || len(addresses) == 0 {
			continue
		}
		autoAssign, found, err := unstructured.NestedBool(pool.Object, "spec", "autoAssign")
		if err != nil {
			return "", fmt.Errorf("failed to read autoAssign for IPAddressPool %s: %w", pool.GetName(), err)
		}
		pools = append(pools, metalLBAddressPool{
			name:       pool.GetName(),
			addresses:  addresses,
			autoAssign: !found || autoAssign,
		})
	}

	if len(pools) == 0 {
		return "", fmt.Errorf("no addresses found in IPAddressPool resources")
	}

	return chooseMetallbAddress(pools, usedAddresses)
}

// guessFromConfigMap tries to get an address from the ConfigMap format
func guessFromConfigMap(cfg *rest.Config) (string, error) {
	clientset, err := kubernetes.NewForConfig(cfg)
	if err != nil {
		return "", fmt.Errorf("failed to create clientset: %w", err)
	}

	cm, err := clientset.CoreV1().ConfigMaps("metallb-system").Get(context.Background(), "config", metav1.GetOptions{})
	if err != nil {
		return "", fmt.Errorf("failed to get ConfigMap: %w", err)
	}

	configData, ok := cm.Data["config"]
	if !ok {
		return "", fmt.Errorf("config key not found in ConfigMap")
	}

	var config struct {
		AddressPools []struct {
			Addresses []string `json:"addresses"`
		} `json:"address-pools"`
	}

	if err := yaml.Unmarshal([]byte(configData), &config); err != nil {
		return "", fmt.Errorf("failed to parse config YAML: %w", err)
	}

	var pools []metalLBAddressPool
	for _, pool := range config.AddressPools {
		if len(pool.Addresses) > 0 {
			pools = append(pools, metalLBAddressPool{addresses: pool.Addresses, autoAssign: true})
		}
	}

	if len(pools) == 0 {
		return "", fmt.Errorf("no addresses found in ConfigMap")
	}

	return chooseMetallbAddress(pools, nil)
}

type metalLBAddressPool struct {
	name       string
	addresses  []string
	autoAssign bool
}

func chooseMetallbAddress(pools []metalLBAddressPool, usedAddresses map[string]struct{}) (string, error) {
	autoAssignedCandidates := map[string]struct{}{}
	hasStaticPool := false
	for _, pool := range pools {
		if !pool.autoAssign {
			hasStaticPool = true
			continue
		}
		for _, address := range pool.addresses {
			for _, candidate := range candidateIPv4Addresses(address) {
				autoAssignedCandidates[candidate] = struct{}{}
			}
		}
	}

	slices.SortStableFunc(pools, func(a, b metalLBAddressPool) int {
		if a.autoAssign != b.autoAssign {
			if !a.autoAssign {
				return -1
			}
			return 1
		}
		return strings.Compare(a.name, b.name)
	})

	var firstAutoAssigned string
	for _, pool := range pools {
		for _, address := range pool.addresses {
			for _, candidate := range candidateIPv4Addresses(address) {
				if _, used := usedAddresses[candidate]; used {
					continue
				}
				if !pool.autoAssign {
					if _, overlaps := autoAssignedCandidates[candidate]; overlaps {
						continue
					}
					return candidate, nil
				}
				if firstAutoAssigned == "" {
					firstAutoAssigned = candidate
				}
			}
		}
	}
	if hasStaticPool {
		return "", fmt.Errorf("no unused IPv4 addresses found in non-auto-assigned MetalLB address pools")
	}
	if firstAutoAssigned != "" {
		return firstAutoAssigned, nil
	}

	return "", fmt.Errorf("no unused IPv4 addresses found in MetalLB address pools")
}

func candidateIPv4Addresses(address string) []string {
	address = strings.TrimSuffix(strings.TrimSpace(address), ",")
	if address == "" {
		return nil
	}
	if strings.Contains(address, "-") {
		parts := strings.Split(address, "-")
		if len(parts) != 2 {
			return nil
		}
		return validIPv4Candidates(strings.TrimSpace(parts[1]), strings.TrimSpace(parts[0]))
	}
	if strings.Contains(address, "/") {
		prefix, err := netip.ParsePrefix(address)
		if err != nil {
			return nil
		}
		if last, ok := lastIPv4InPrefix(prefix); ok {
			return []string{last.String()}
		}
		return nil
	}
	return validIPv4Candidates(address)
}

func validIPv4Candidates(addresses ...string) []string {
	var candidates []string
	for _, address := range addresses {
		addr, err := netip.ParseAddr(address)
		if err == nil && addr.Is4() {
			candidates = append(candidates, addr.String())
		}
	}
	return candidates
}

func lastIPv4InPrefix(prefix netip.Prefix) (netip.Addr, bool) {
	prefix = prefix.Masked()
	addr := prefix.Addr()
	if !addr.Is4() {
		return netip.Addr{}, false
	}
	bits := prefix.Bits()
	if bits < 0 || bits > 32 {
		return netip.Addr{}, false
	}
	raw := addr.As4()
	value := uint32(raw[0])<<24 | uint32(raw[1])<<16 | uint32(raw[2])<<8 | uint32(raw[3])
	hostBits := 32 - bits
	if hostBits > 0 {
		value |= (uint32(1) << hostBits) - 1
	}
	return netip.AddrFrom4([4]byte{
		byte(value >> 24),
		byte(value >> 16),
		byte(value >> 8),
		byte(value),
	}), true
}

func usedLoadBalancerAddresses(cfg *rest.Config) (map[string]struct{}, error) {
	clientset, err := kubernetes.NewForConfig(cfg)
	if err != nil {
		return nil, fmt.Errorf("failed to create clientset: %w", err)
	}
	services, err := clientset.CoreV1().Services("").List(context.Background(), metav1.ListOptions{})
	if err != nil {
		return nil, err
	}

	used := map[string]struct{}{}
	for _, svc := range services.Items {
		for _, ip := range serviceAddresses(svc) {
			used[ip] = struct{}{}
		}
	}
	return used, nil
}

func serviceAddresses(svc corev1.Service) []string {
	var addresses []string
	if svc.Spec.LoadBalancerIP != "" {
		addresses = append(addresses, svc.Spec.LoadBalancerIP)
	}
	addresses = append(addresses, svc.Spec.ExternalIPs...)
	for _, ingress := range svc.Status.LoadBalancer.Ingress {
		if ingress.IP != "" {
			addresses = append(addresses, ingress.IP)
		}
	}
	return addresses
}
