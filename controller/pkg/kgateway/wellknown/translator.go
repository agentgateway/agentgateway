package wellknown

const (
	// GatewayApiProxyValue is the label value for ProxyTypeKey applied to Proxy CRs
	// that have been generated from Kubernetes Gateway API resources
	GatewayApiProxyValue = "kgateway-kube-gateway-api"

	CELExtensionFilter = "envoy.access_loggers.extension_filters.cel"
)
