package deployer

import (
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/test/util/assert"
	"sigs.k8s.io/controller-runtime/pkg/client"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
)

func NewAgwCols(t test.Failer, initObjs ...client.Object) *plugins.AgwCollections {
	return NewAgwColsWithSettings(t, apisettings.Settings{}, initObjs...)
}

// NewAgwColsWithSettings builds collections with explicit control-plane settings. Use this when a
// test needs install-time settings baked in at construction (e.g. IstioRevision/IstioNamespace,
// which the MeshConfig singleton captures when it's built and can't be changed afterward).
func NewAgwColsWithSettings(t test.Failer, settings apisettings.Settings, initObjs ...client.Object) *plugins.AgwCollections {
	ctx := test.NewContext(t)
	krtopts := krtutil.NewKrtOptions(ctx.Done(), nil)
	clt := fake.NewClient(t, initObjs...)
	c, err := plugins.NewAgwCollections(
		krtopts,
		clt,
		wellknown.DefaultAgwControllerName,
		settings,
		"agentgateway-system",
	)
	assert.NoError(t, err)
	clt.RunAndWait(test.NewStop(t))
	kube.WaitForCacheSync("test", test.NewStop(t), c.HasSynced)
	return c
}
