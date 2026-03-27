package deployer

import (
	"github.com/agentgateway/agentgateway/controller/pkg/wellknown"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/test/util/assert"
	"sigs.k8s.io/controller-runtime/pkg/client"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

func NewAgwCols(t test.Failer, initObjs ...client.Object) *plugins.AgwCollections {
	ctx := test.NewContext(t)
	krtopts := krtutil.NewKrtOptions(ctx.Done(), nil)
	clt := fake.NewClient(t, initObjs...)
	c, err := plugins.NewAgwCollections(
		krtopts,
		clt,
		wellknown.DefaultAgwControllerName,
		apisettings.Settings{},
		"agentgateway-system",
		"test-cluster",
	)
	assert.NoError(t, err)
	clt.RunAndWait(test.NewStop(t))
	kube.WaitForCacheSync("test", test.NewStop(t), c.HasSynced)
	return c
}
