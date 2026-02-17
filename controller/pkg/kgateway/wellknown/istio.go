package wellknown

import (
	istionetworking "istio.io/client-go/pkg/apis/networking/v1"
	"k8s.io/apimachinery/pkg/util/sets"
)

var (
	ServiceEntryGVK = istionetworking.SchemeGroupVersion.WithKind("ServiceEntry")
	HostnameGVK     = istionetworking.SchemeGroupVersion.WithKind("Hostname")
)

var (
	GlobalRefGKs = sets.New(
		HostnameGVK.GroupKind(),
	)
)
