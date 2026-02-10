package deployer

import (
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/test/util/assert"
	"sigs.k8s.io/controller-runtime/pkg/client"

	_ "github.com/envoyproxy/go-control-plane/envoy/extensions/upstreams/http/v3"

	apisettings "github.com/agentgateway/agentgateway/controller/api/settings"
	"github.com/agentgateway/agentgateway/controller/pkg/apiclient/fake"
	"github.com/agentgateway/agentgateway/controller/pkg/kgateway/wellknown"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/collections"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

func NewCommonCols(t test.Failer, initObjs ...client.Object) *collections.CommonCollections {
	ctx := test.NewContext(t)
	krtopts := krtutil.NewKrtOptions(ctx.Done(), nil)
	clt := fake.NewClient(t, initObjs...)
	c, err := collections.NewCommonCollections(krtopts, clt, wellknown.DefaultAgwControllerName, apisettings.Settings{})
	assert.NoError(t, err)
	clt.RunAndWait(test.NewStop(t))
	kube.WaitForCacheSync("test", test.NewStop(t), c.HasSynced)
	return c
}
