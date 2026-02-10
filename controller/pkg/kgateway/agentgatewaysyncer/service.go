package agentgatewaysyncer

import (
	"fmt"
	"strings"
	"sync"
	"time"

	"github.com/google/go-cmp/cmp"
	"github.com/google/go-cmp/cmp/cmpopts"
	"istio.io/istio/pilot/pkg/features"
	"istio.io/istio/pilot/pkg/model"
	"istio.io/istio/pilot/pkg/serviceregistry/provider"
	"istio.io/istio/pilot/pkg/util/protoconv"
	"istio.io/istio/pkg/cluster"
	"istio.io/istio/pkg/config/constants"
	"istio.io/istio/pkg/config/host"
	"istio.io/istio/pkg/config/protocol"
	"istio.io/istio/pkg/config/schema/kind"
	"istio.io/istio/pkg/config/visibility"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/maps"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	"istio.io/istio/pkg/workloadapi"
	"k8s.io/apimachinery/pkg/types"
	inf "sigs.k8s.io/gateway-api-inference-extension/api/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils"
)

// InferenceHostname produces FQDN for a k8s service
func InferenceHostname(name, namespace, domainSuffix string) host.Name {
	return host.Name(name + "." + namespace + "." + "inference" + "." + domainSuffix) // Format: "%s.%s.svc.%s"
}

func inferencePoolBuilder() krt.TransformationSingle[*inf.InferencePool, model.ServiceInfo] {
	domainSuffix := kubeutils.GetClusterDomainName()
	return func(ctx krt.HandlerContext, s *inf.InferencePool) *model.ServiceInfo {
		portNames := map[int32]model.ServicePortName{}
		ports := []*workloadapi.Port{{
			ServicePort: uint32(s.Spec.TargetPorts[0].Number), //nolint:gosec // G115: InferencePool TargetPort is int32 with validation 1-65535, always safe
			TargetPort:  uint32(s.Spec.TargetPorts[0].Number), //nolint:gosec // G115: InferencePool TargetPort is int32 with validation 1-65535, always safe
			AppProtocol: workloadapi.AppProtocol_HTTP11,
		}}

		// TODO this is only checking one controller - we may be missing service vips for instances in another cluster
		svc := &workloadapi.Service{
			Name:      s.Name,
			Namespace: s.Namespace,
			Hostname:  string(InferenceHostname(s.Name, s.Namespace, domainSuffix)),
			Ports:     ports,
		}

		selector := make(map[string]string, len(s.Spec.Selector.MatchLabels))
		for k, v := range s.Spec.Selector.MatchLabels {
			selector[string(k)] = string(v)
		}
		return precomputeServicePtr(&model.ServiceInfo{
			Service:       svc,
			PortNames:     portNames,
			LabelSelector: model.NewSelector(selector),
			Source: model.TypedObject{
				NamespacedName: types.NamespacedName{
					Namespace: s.Namespace,
					Name:      s.Name,
				},
				Kind: kind.InferencePool,
			},
		})
	}
}

// Service describes an Istio service (e.g., catalog.mystore.com:8080)
// Each service has a fully qualified domain name (FQDN) and one or more
// ports where the service is listening for connections. *Optionally*, a
// service can have a single load balancer/virtual IP address associated
// with it, such that the DNS queries for the FQDN resolves to the virtual
// IP address (a load balancer IP).
//
// E.g., in kubernetes, a service foo is associated with
// foo.default.svc.cluster.local hostname, has a virtual IP of 10.0.1.1 and
// listens on ports 80, 8080
type Service struct {
	// Attributes contains additional attributes associated with the service
	// used mostly by RBAC for policy enforcement purposes.
	Attributes ServiceAttributes

	// Ports is the set of network ports where the service is listening for
	// connections
	Ports PortList `json:"ports,omitempty"`

	// ServiceAccounts specifies the service accounts that run the service.
	ServiceAccounts []string `json:"serviceAccounts,omitempty"`

	// CreationTime records the time this service was created, if available.
	// +krtEqualsTodo decide if CreationTime impacts KRT outputs
	CreationTime time.Time `json:"creationTime,omitempty"`

	// Name of the service, e.g. "catalog.mystore.com"
	Hostname host.Name `json:"hostname"`

	// ClusterVIPs specifies the service address of the load balancer
	// in each of the clusters where the service resides
	ClusterVIPs AddressMap `json:"clusterVIPs,omitempty"`

	// DefaultAddress specifies the default service IP of the load balancer.
	// Do not access directly. Use GetAddressForProxy
	DefaultAddress string `json:"defaultAddress,omitempty"`

	// AutoAllocatedIPv4Address and AutoAllocatedIPv6Address specifies
	// the automatically allocated IPv4/IPv6 address out of the reserved
	// Class E subnet (240.240.0.0/16) or reserved Benchmarking IP range
	// (2001:2::/48) in RFC5180.for service entries with non-wildcard
	// hostnames. The IPs assigned to services are not
	// synchronized across istiod replicas as the DNS resolution
	// for these service entries happens completely inside a pod
	// whose proxy is managed by one istiod. That said, the algorithm
	// to allocate IPs is pretty deterministic that at stable state, two
	// istiods will allocate the exact same set of IPs for a given set of
	// service entries.
	AutoAllocatedIPv4Address string `json:"autoAllocatedIPv4Address,omitempty"`
	AutoAllocatedIPv6Address string `json:"autoAllocatedIPv6Address,omitempty"`

	// Resolution indicates how the service instances need to be resolved before routing
	// traffic. Most services in the service registry will use static load balancing wherein
	// the proxy will decide the service instance that will receive the traffic. Service entries
	// could either use DNS load balancing (i.e. proxy will query DNS server for the IP of the service)
	// or use the passthrough model (i.e. proxy will forward the traffic to the network endpoint requested
	// by the caller)
	Resolution Resolution

	// ResourceVersion represents the internal version of this object.
	// +krtEqualsTodo propagate ResourceVersion into equality if needed
	ResourceVersion string
}

func (s *Service) NamespacedName() types.NamespacedName {
	return types.NamespacedName{Name: s.Attributes.Name, Namespace: s.Attributes.Namespace}
}

func (s *Service) Key() string {
	if s == nil {
		return ""
	}

	return s.Attributes.Namespace + "/" + string(s.Hostname)
}

var serviceCmpOpts = []cmp.Option{cmpopts.IgnoreFields(AddressMap{}, "mutex")}

func (s *Service) CmpOpts() []cmp.Option {
	return serviceCmpOpts
}

func (s *Service) SupportsDrainingEndpoints() bool {
	return (features.PersistentSessionLabel != "" && s.Attributes.Labels[features.PersistentSessionLabel] != "") ||
		(features.PersistentSessionHeaderLabel != "" && s.Attributes.Labels[features.PersistentSessionHeaderLabel] != "")
}

// SupportsUnhealthyEndpoints marks if this service should send unhealthy endpoints
func (s *Service) SupportsUnhealthyEndpoints() bool {
	if features.GlobalSendUnhealthyEndpoints.Load() {
		// Enable process-wide
		return true
	}
	if s != nil && s.Attributes.TrafficDistribution != TrafficDistributionAny {
		// When we are doing location aware routing, we need some way to indicate if endpoints are healthy, otherwise we don't
		// know when to spill over to other zones.
		// For the older DestinationRule localityLB, we do this by requiring outlier detection.
		// If they use the newer Kubernetes-native TrafficDistribution we don't want to require an Istio-specific outlier rule,
		// and instead will use endpoint health which requires sending unhealthy endpoints.
		return true
	}
	return false
}

// Resolution indicates how the service instances need to be resolved before routing traffic.
type Resolution int

const (
	// ClientSideLB implies that the proxy will decide the endpoint from its local lb pool
	ClientSideLB Resolution = iota
	// DNSLB implies that the proxy will resolve a DNS address and forward to the resolved address
	DNSLB
	// Passthrough implies that the proxy should forward traffic to the destination IP requested by the caller
	Passthrough
	// DNSRoundRobinLB implies that the proxy will resolve a DNS address and forward to the resolved address
	DNSRoundRobinLB
	// Alias defines a Service that is an alias for another.
	Alias
)

// String converts Resolution in to String.
func (resolution Resolution) String() string {
	switch resolution {
	case ClientSideLB:
		return "ClientSide"
	case DNSLB:
		return "DNS"
	case DNSRoundRobinLB:
		return "DNSRoundRobin"
	case Passthrough:
		return "Passthrough"
	default:
		return fmt.Sprintf("%d", int(resolution))
	}
}

// Port represents a network port where a service is listening for
// connections. The port should be annotated with the type of protocol
// used by the port.
type Port struct {
	// Name ascribes a human-readable name for the port object. When a
	// service has multiple ports, the name field is mandatory
	Name string `json:"name,omitempty"`

	// Port number where the service can be reached. Does not necessarily
	// map to the corresponding port numbers for the instances behind the
	// service.
	Port int `json:"port"`

	// Protocol to be used for the port.
	Protocol protocol.Instance `json:"protocol,omitempty"`
}

func (p Port) String() string {
	return fmt.Sprintf("Name:%s Port:%d Protocol:%v", p.Name, p.Port, p.Protocol)
}

// PortList is a set of ports
type PortList []*Port

// ServiceTarget includes a Service object, along with a specific service port
// and target port. This is basically a smaller version of ServiceInstance,
// intended to avoid the need to have the full object when only port information
// is needed.
type ServiceTarget struct {
	Service *Service
	Port    ServiceInstancePort
}

type (
	ServicePort = *Port
	// ServiceInstancePort defines a port that has both a port and targetPort (which distinguishes it from Port)
	// Note: ServiceInstancePort only makes sense in the context of a specific ServiceInstance, because TargetPort depends on a specific instance.
	ServiceInstancePort struct {
		ServicePort
		TargetPort uint32
	}
)

type workloadKind int

const (
	// PodKind indicates the workload is from pod
	PodKind workloadKind = iota
	// WorkloadEntryKind indicates the workload is from workloadentry
	WorkloadEntryKind
)

func (k workloadKind) String() string {
	if k == PodKind {
		return "Pod"
	}

	if k == WorkloadEntryKind {
		return "WorkloadEntry"
	}
	return ""
}

// ServiceAttributes represents a group of custom attributes of the service.
type ServiceAttributes struct {
	// ServiceRegistry indicates the backing service registry system where this service
	// was sourced from.
	// TODO: move the ServiceRegistry type from platform.go to model
	ServiceRegistry provider.ID
	// Name is "destination.service.name" attribute
	Name string
	// Namespace is "destination.service.namespace" attribute
	Namespace string
	// Labels applied to the service
	Labels map[string]string
	// ExportTo defines the visibility of Service in
	// a namespace when the namespace is imported.
	ExportTo sets.Set[visibility.Instance]

	// LabelSelectors are the labels used by the service to select workloads.
	// Applicable to both Kubernetes and ServiceEntries.
	LabelSelectors map[string]string

	// Aliases is the resolved set of aliases for this service. This is computed based on a global view of all Service's `AliasFor`
	// fields.
	// For example, if I had two Services with `externalName: foo`, "a" and "b", then the "foo" service would have Aliases=[a,b].
	Aliases []NamespacedHostname

	// For Kubernetes platform

	// ClusterExternalAddresses is a mapping between a cluster name and the external
	// address(es) to access the service from outside the cluster.
	// Used by the aggregator to aggregate the Attributes.ClusterExternalAddresses
	// for clusters where the service resides
	ClusterExternalAddresses *AddressMap

	// ClusterExternalPorts is a mapping between a cluster name and the service port
	// to node port mappings for a given service. When accessing the service via
	// node port IPs, we need to use the kubernetes assigned node ports of the service
	ClusterExternalPorts map[cluster.ID]map[uint32]uint32

	// +krtEqualsTodo evaluate passthrough target port equality semantics
	PassthroughTargetPorts map[uint32]uint32

	K8sAttributes
}

type NamespacedHostname struct {
	Hostname  host.Name
	Namespace string
}

type K8sAttributes struct {
	// Type holds the value of the corev1.Type of the Kubernetes service
	// spec.Type
	Type string

	// spec.ExternalName
	ExternalName string

	// NodeLocal means the proxy will only forward traffic to node local endpoints
	// spec.InternalTrafficPolicy == Local
	NodeLocal bool

	// TrafficDistribution determines the service-level traffic distribution.
	// This may be overridden by locality load balancing settings.
	TrafficDistribution TrafficDistribution

	// ObjectName is the object name of the underlying object. This may differ from the Service.Attributes.Name for legacy semantics.
	ObjectName string

	// spec.PublishNotReadyAddresses
	PublishNotReadyAddresses bool
}

type TrafficDistribution int

const (
	// TrafficDistributionAny allows any destination
	TrafficDistributionAny TrafficDistribution = iota
	// TrafficDistributionPreferClose prefers traffic in same region/zone/network if possible, with failover allowed.
	TrafficDistributionPreferClose TrafficDistribution = iota
)

// DeepCopy creates a deep copy of ServiceAttributes, but skips internal mutexes.
func (s *ServiceAttributes) DeepCopy() ServiceAttributes {
	// AddressMap contains a mutex, which is safe to copy in this case.
	// nolint: govet
	out := *s

	out.Labels = maps.Clone(s.Labels)
	if s.ExportTo != nil {
		out.ExportTo = s.ExportTo.Copy()
	}

	out.LabelSelectors = maps.Clone(s.LabelSelectors)
	out.ClusterExternalAddresses = s.ClusterExternalAddresses.DeepCopy()

	if s.ClusterExternalPorts != nil {
		out.ClusterExternalPorts = make(map[cluster.ID]map[uint32]uint32, len(s.ClusterExternalPorts))
		for k, m := range s.ClusterExternalPorts {
			out.ClusterExternalPorts[k] = maps.Clone(m)
		}
	}

	out.Aliases = slices.Clone(s.Aliases)
	out.PassthroughTargetPorts = maps.Clone(out.PassthroughTargetPorts)

	// AddressMap contains a mutex, which is safe to return a copy in this case.
	// nolint: govet
	return out
}

// Equals checks whether the attributes are equal from the passed in service.
func (s *ServiceAttributes) Equals(other *ServiceAttributes) bool {
	if s == nil {
		return other == nil
	}
	if other == nil {
		return s == nil
	}

	if !maps.Equal(s.Labels, other.Labels) {
		return false
	}

	if !maps.Equal(s.LabelSelectors, other.LabelSelectors) {
		return false
	}

	if !maps.Equal(s.ExportTo, other.ExportTo) {
		return false
	}

	if !slices.Equal(s.Aliases, other.Aliases) {
		return false
	}

	if s.ClusterExternalAddresses.Len() != other.ClusterExternalAddresses.Len() {
		return false
	}

	for k, v1 := range s.ClusterExternalAddresses.GetAddresses() {
		if v2, ok := other.ClusterExternalAddresses.Addresses[k]; !ok || !slices.Equal(v1, v2) {
			return false
		}
	}

	if len(s.ClusterExternalPorts) != len(other.ClusterExternalPorts) {
		return false
	}

	for k, v1 := range s.ClusterExternalPorts {
		if v2, ok := s.ClusterExternalPorts[k]; !ok || !maps.Equal(v1, v2) {
			return false
		}
	}
	return s.Name == other.Name && s.Namespace == other.Namespace &&
		s.ServiceRegistry == other.ServiceRegistry && s.K8sAttributes == other.K8sAttributes
}

type TypedObject struct {
	types.NamespacedName
	Kind string
}

type StatusMessage struct {
	Reason  string
	Message string
}

type WaypointBindingStatus struct {
	// ResourceName that clients should use when addressing traffic to this Service.
	ResourceName string
	// IngressUseWaypoint specifies whether ingress gateways should use the waypoint for this service.
	IngressUseWaypoint bool
	// IngressLabelPresent specifies whether the istio.io/ingress-use-waypoint label is set on the service.
	IngressLabelPresent bool
	// Error represents some error
	Error *StatusMessage
}

func (i WaypointBindingStatus) Equals(other WaypointBindingStatus) bool {
	return i.ResourceName == other.ResourceName &&
		i.IngressUseWaypoint == other.IngressUseWaypoint &&
		i.IngressLabelPresent == other.IngressLabelPresent &&
		ptr.Equal(i.Error, other.Error)
}

type Address struct {
	Workload *model.WorkloadInfo
	Service  *model.ServiceInfo
}

func (i Address) ResourceName() string {
	if i.Workload != nil {
		return i.Workload.ResourceName()
	}
	return i.Service.ResourceName()
}
func (i Address) Equals(other Address) bool {
	if (i.Workload != nil) != (other.Workload != nil) {
		return false
	}
	if i.Workload != nil {
		return i.Workload.Equals(*other.Workload)
	}
	return i.Service.Equals(*other.Service)
}

func (i Address) IntoProto() *workloadapi.Address {
	if i.Workload != nil {
		return i.Workload.AsAddress.Address
	}
	return i.Service.AsAddress.Address
}

// MCSServiceInfo combines the name of a service with a particular Kubernetes cluster. This
// is used for debug information regarding the state of Kubernetes Multi-Cluster Services (MCS).
type MCSServiceInfo struct {
	Cluster         cluster.ID
	Name            string
	Namespace       string
	Exported        bool
	Imported        bool
	ClusterSetVIP   string
	Discoverability map[host.Name]string
}

// GetNames returns port names
func (ports PortList) GetNames() []string {
	names := make([]string, 0, len(ports))
	for _, port := range ports {
		names = append(names, port.Name)
	}
	return names
}

// Get retrieves a port declaration by name
func (ports PortList) Get(name string) (*Port, bool) {
	for _, port := range ports {
		if port.Name == name {
			return port, true
		}
	}
	return nil, false
}

// GetByPort retrieves a port declaration by port value
func (ports PortList) GetByPort(num int) (*Port, bool) {
	for _, port := range ports {
		if port.Port == num && port.Protocol != protocol.UDP {
			return port, true
		}
	}
	return nil, false
}

func (p *Port) Equals(other *Port) bool {
	if p == nil {
		return other == nil
	}
	if other == nil {
		return p == nil
	}
	return p.Name == other.Name && p.Port == other.Port && p.Protocol == other.Protocol
}

func (ports PortList) Equals(other PortList) bool {
	return slices.EqualFunc(ports, other, func(a, b *Port) bool {
		return a.Equals(b)
	})
}

func (ports PortList) String() string {
	sp := make([]string, 0, len(ports))
	for _, p := range ports {
		sp = append(sp, p.String())
	}
	return strings.Join(sp, ", ")
}

// HasAddressOrAssigned returns whether the service has an IP address.
// This includes auto-allocated IP addresses. Note that not all proxies support auto-allocated IP addresses;
// typically GetAllAddressesForProxy should be used which automatically filters addresses to account for that.
func (s *Service) HasAddressOrAssigned(id cluster.ID) bool {
	if id != "" {
		if len(s.ClusterVIPs.GetAddressesFor(id)) > 0 {
			return true
		}
	}
	if s.DefaultAddress != constants.UnspecifiedIP {
		return true
	}
	if s.AutoAllocatedIPv4Address != "" {
		return true
	}
	if s.AutoAllocatedIPv6Address != "" {
		return true
	}
	return false
}

// DeepCopy creates a clone of Service.
func (s *Service) DeepCopy() *Service {
	// nolint: govet
	out := *s
	out.Attributes = s.Attributes.DeepCopy()
	if s.Ports != nil {
		out.Ports = make(PortList, len(s.Ports))
		for i, port := range s.Ports {
			if port != nil {
				out.Ports[i] = &Port{
					Name:     port.Name,
					Port:     port.Port,
					Protocol: port.Protocol,
				}
			} else {
				out.Ports[i] = nil
			}
		}
	}

	out.ServiceAccounts = slices.Clone(s.ServiceAccounts)
	out.ClusterVIPs = *s.ClusterVIPs.DeepCopy()
	return &out
}

// Equals compares two service objects.
func (s *Service) Equals(other *Service) bool {
	if s == nil {
		return other == nil
	}
	if other == nil {
		return s == nil
	}

	if !s.Attributes.Equals(&other.Attributes) {
		return false
	}

	if !s.Ports.Equals(other.Ports) {
		return false
	}
	if !slices.Equal(s.ServiceAccounts, other.ServiceAccounts) {
		return false
	}

	if len(s.ClusterVIPs.Addresses) != len(other.ClusterVIPs.Addresses) {
		return false
	}
	for k, v1 := range s.ClusterVIPs.Addresses {
		if v2, ok := other.ClusterVIPs.Addresses[k]; !ok || !slices.Equal(v1, v2) {
			return false
		}
	}

	return s.DefaultAddress == other.DefaultAddress && s.AutoAllocatedIPv4Address == other.AutoAllocatedIPv4Address &&
		s.AutoAllocatedIPv6Address == other.AutoAllocatedIPv6Address && s.Hostname == other.Hostname &&
		s.Resolution == other.Resolution
}

// AddressMap provides a thread-safe mapping of addresses for each Kubernetes cluster.
type AddressMap struct {
	// Addresses hold the underlying map. Most code should only access this through the available methods.
	// Should only be used by tests and construction/initialization logic, where there is no concern
	// for race conditions.
	Addresses map[cluster.ID][]string

	// NOTE: The copystructure library is not able to copy unexported fields, so the mutex will not be copied.
	mutex sync.RWMutex
}

func (m *AddressMap) Len() int {
	if m == nil {
		return 0
	}
	m.mutex.RLock()
	defer m.mutex.RUnlock()

	return len(m.Addresses)
}

func (m *AddressMap) DeepCopy() *AddressMap {
	if m == nil {
		return nil
	}
	return &AddressMap{
		Addresses: m.GetAddresses(),
	}
}

// GetAddresses returns the mapping of clusters to addresses.
func (m *AddressMap) GetAddresses() map[cluster.ID][]string {
	if m == nil {
		return nil
	}

	m.mutex.RLock()
	defer m.mutex.RUnlock()

	if m.Addresses == nil {
		return nil
	}

	out := make(map[cluster.ID][]string)
	for k, v := range m.Addresses {
		out[k] = slices.Clone(v)
	}
	return out
}

// SetAddresses sets the addresses per cluster.
func (m *AddressMap) SetAddresses(addrs map[cluster.ID][]string) {
	if len(addrs) == 0 {
		addrs = nil
	}

	m.mutex.Lock()
	m.Addresses = addrs
	m.mutex.Unlock()
}

func (m *AddressMap) GetAddressesFor(c cluster.ID) []string {
	if m == nil {
		return nil
	}

	m.mutex.RLock()
	defer m.mutex.RUnlock()

	if m.Addresses == nil {
		return nil
	}

	// Copy the Addresses array.
	return append([]string{}, m.Addresses[c]...)
}

func (m *AddressMap) SetAddressesFor(c cluster.ID, addresses []string) *AddressMap {
	m.mutex.Lock()
	defer m.mutex.Unlock()

	if len(addresses) == 0 {
		// Setting an empty array for the cluster. Remove the entry for the cluster if it exists.
		if m.Addresses != nil {
			delete(m.Addresses, c)

			// Delete the map if there's nothing left.
			if len(m.Addresses) == 0 {
				m.Addresses = nil
			}
		}
	} else {
		// Create the map if it doesn't already exist.
		if m.Addresses == nil {
			m.Addresses = make(map[cluster.ID][]string)
		}
		m.Addresses[c] = addresses
	}
	return m
}

func (m *AddressMap) AddAddressesFor(c cluster.ID, addresses []string) *AddressMap {
	if len(addresses) == 0 {
		return m
	}

	m.mutex.Lock()
	defer m.mutex.Unlock()

	// Create the map if nil.
	if m.Addresses == nil {
		m.Addresses = make(map[cluster.ID][]string)
	}

	m.Addresses[c] = append(m.Addresses[c], addresses...)
	return m
}

func (m *AddressMap) ForEach(fn func(c cluster.ID, addresses []string)) {
	if m == nil {
		return
	}

	m.mutex.RLock()
	defer m.mutex.RUnlock()

	if m.Addresses == nil {
		return
	}

	for c, addresses := range m.Addresses {
		fn(c, addresses)
	}
}

func precomputeServicePtr(w *model.ServiceInfo) *model.ServiceInfo {
	return ptr.Of(precomputeService(*w))
}

func precomputeService(w model.ServiceInfo) model.ServiceInfo {
	addr := serviceToAddress(w.Service)
	setServiceAddress(&w, addr)
	return w
}

func serviceToAddress(s *workloadapi.Service) *workloadapi.Address {
	return &workloadapi.Address{
		Type: &workloadapi.Address_Service{
			Service: s,
		},
	}
}

// setServiceAddress sets the AsAddress and MarshaledAddress fields on a ServiceInfo
// using the provided Address. This is the shared logic for populating AsAddress.
func setServiceAddress(s *model.ServiceInfo, addr *workloadapi.Address) {
	s.MarshaledAddress = protoconv.MessageToAny(addr)
	s.AsAddress = model.AddressInfo{
		Address:   addr,
		Marshaled: s.MarshaledAddress,
	}
}
