//go:build e2e

package transformation

import (
	"path/filepath"
	"time"

	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/fsutils"
)

const (
	namespace = "agentgateway-base"
)

var (
	// manifests
	transformForHeadersManifest      = filepath.Join(fsutils.MustGetThisDir(), "testdata", "transform-for-headers.yaml")
	transformForBodyManifest         = filepath.Join(fsutils.MustGetThisDir(), "testdata", "transform-for-body.yaml")
	gatewayAttachedTransformManifest = filepath.Join(fsutils.MustGetThisDir(), "testdata", "gateway-attached-transform.yaml")
	grpcTransformationManifest       = filepath.Join(fsutils.MustGetThisDir(), "testdata", "grpc-transformation.yaml")

	// objects from gateway manifest
	gateway = &gwv1.Gateway{
		ObjectMeta: metav1.ObjectMeta{
			Name:      "gateway",
			Namespace: namespace,
		},
	}

	// timeouts
	timeout = 1 * time.Minute
)
