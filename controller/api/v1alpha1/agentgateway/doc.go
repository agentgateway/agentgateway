// +k8s:openapi-gen=true
// +kubebuilder:object:generate=true
// +groupName=agentgateway.dev
// +versionName=v1alpha1
package agentgateway

// Gateway API resources with status management
// +kubebuilder:rbac:groups=gateway.networking.k8s.io,resources=gatewayclasses;gateways;httproutes;grpcroutes;tcproutes;tlsroutes;referencegrants;backendtlspolicies,verbs=get;list;watch
// +kubebuilder:rbac:groups=gateway.networking.x-k8s.io,resources=xlistenersets,verbs=get;list;watch
// +kubebuilder:rbac:groups=gateway.networking.k8s.io,resources=gatewayclasses/status;gateways/status;httproutes/status;grpcroutes/status;tcproutes/status;tlsroutes/status;backendtlspolicies/status,verbs=patch;update
// +kubebuilder:rbac:groups=gateway.networking.x-k8s.io,resources=xlistenersets/status,verbs=patch;update
// +kubebuilder:rbac:groups=gateway.networking.k8s.io,resources=gatewayclasses,verbs=create;patch;update

// Controller resources
// +kubebuilder:rbac:groups="",resources=pods,verbs=get;list;watch
// +kubebuilder:rbac:groups="",resources=nodes,verbs=get;list;watch
// +kubebuilder:rbac:groups="",resources=secrets,verbs=get;list;watch
// +kubebuilder:rbac:groups="",resources=namespaces,verbs=get;list;watch
// +kubebuilder:rbac:groups="",resources=endpoints,verbs=get;list;watch

// jwks store controller that require extra permissions
// +kubebuilder:rbac:groups="",resources=configmaps,verbs=get;list;watch;create;patch;update;delete

// Proxy deployer resources that require extra permissions
// +kubebuilder:rbac:groups=apps,resources=deployments,verbs=get;list;watch;create;patch;update;delete
// +kubebuilder:rbac:groups="",resources=services,verbs=get;list;watch;create;patch;update;delete
// +kubebuilder:rbac:groups="",resources=secrets;serviceaccounts,verbs=get;list;watch;create;patch;delete
// +kubebuilder:rbac:groups="",resources=serviceaccounts,verbs=get;list;watch;create;patch;delete

// EDS discovery resources
// +kubebuilder:rbac:groups=discovery.k8s.io,resources=endpointslices,verbs=get;list;watch

// CRD access for scheme registration
// +kubebuilder:rbac:groups=apiextensions.k8s.io,resources=customresourcedefinitions,verbs=get;list;watch

// Istio resources for traffic management
// +kubebuilder:rbac:groups=networking.istio.io,resources=destinationrules,verbs=get;list;watch
// +kubebuilder:rbac:groups=networking.istio.io,resources=serviceentries,verbs=get;list;watch
// +kubebuilder:rbac:groups=networking.istio.io,resources=workloadentries,verbs=get;list;watch
// +kubebuilder:rbac:groups=security.istio.io,resources=authorizationpolicies,verbs=get;list;watch

// Leases for leader election
// +kubebuilder:rbac:groups=coordination.k8s.io,resources=leases,verbs=get;create;update
// +kubebuilder:rbac:groups="",resources=events,verbs=create;patch
