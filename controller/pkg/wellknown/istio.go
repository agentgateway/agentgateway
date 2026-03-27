package wellknown

import (
	istionetworking "istio.io/client-go/pkg/apis/networking/v1"
)

var (
	ServiceEntryGVK = istionetworking.SchemeGroupVersion.WithKind("ServiceEntry")
	HostnameGVK     = istionetworking.SchemeGroupVersion.WithKind("Hostname")
)
