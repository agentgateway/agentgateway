package krtxds_test

import (
	"context"
	"net"
	"testing"

	"github.com/agentgateway/agentgateway/go/api"
	discovery "github.com/envoyproxy/go-control-plane/envoy/service/discovery/v3"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/test/bufconn"
	"istio.io/istio/pilot/pkg/model"
	istioxds "istio.io/istio/pilot/pkg/xds"
	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/workloadapi"
	"k8s.io/apimachinery/pkg/types"

	agwir "github.com/kgateway-dev/kgateway/v2/pkg/agentgateway/ir"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/krtxds"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
)

type Fake struct {
	Server      *krtxds.DiscoveryServer
	Addresses   krt.StaticCollection[agentgatewaysyncer.Address]
	Resources   krt.StaticCollection[agwir.AgwResource]
	BufListener *bufconn.Listener
	t           *testing.T
}

func NewFakeDiscoveryServer(t *testing.T, initialAddress ...agentgatewaysyncer.Address) Fake {
	return NewFakeDiscoveryServerWith(t, initialAddress, nil)
}
func NewFakeDiscoveryServerWith(t *testing.T, initialAddress []agentgatewaysyncer.Address, initialResource []agwir.AgwResource) Fake {
	stop := test.NewStop(t)
	opts := krtutil.NewKrtOptions(stop, new(krt.DebugHandler))
	xdsAddress := krt.NewStaticCollection[agentgatewaysyncer.Address](nil, initialAddress, opts.ToOptions("address")...)
	xdsResource := krt.NewStaticCollection[agwir.AgwResource](nil, initialResource, opts.ToOptions("resource")...)
	agwResourcesByGateway := func(resource agwir.AgwResource) types.NamespacedName {
		return resource.Gateway
	}
	reg := []krtxds.Registration{
		krtxds.Collection[agentgatewaysyncer.Address, *workloadapi.Address](xdsAddress, opts),
		krtxds.PerGatewayCollection[agwir.AgwResource, *api.Resource](xdsResource, agwResourcesByGateway, opts),
	}
	// we won't need a mock nack event publisher for this testing, so we pass nil
	s := krtxds.NewDiscoveryServer(opts.Debugger, nil, reg...)
	s.Start(stop)
	xdsAddress.WaitUntilSynced(stop)
	xdsResource.WaitUntilSynced(stop)
	kube.WaitForCacheSync("test", stop, s.IsServerReady)
	// Wait for the initial data's debounce to complete before connecting.
	// If we connect before it fires, the client will receive both the request
	// response AND the debounce push, causing spurious test failures.
	s.EnsureSynced()

	buffer := 1024 * 1024
	listener := bufconn.Listen(buffer)

	grpcServer := grpc.NewServer()
	discovery.RegisterAggregatedDiscoveryServiceServer(grpcServer, s)
	go func() {
		if err := grpcServer.Serve(listener); err != nil && !(err == grpc.ErrServerStopped || err.Error() == "closed") {
			t.Fatal(err)
		}
	}()
	t.Cleanup(func() {
		grpcServer.Stop()
		_ = listener.Close()
	})

	return Fake{
		Server:      s,
		t:           t,
		BufListener: listener,
		Addresses:   xdsAddress,
		Resources:   xdsResource,
	}
}

// EnsureSynced waits until all pending debounce events have been processed.
// This should be called before connecting clients to avoid spurious pushes from
// initial data that was added when creating the fake server.
func (f Fake) EnsureSynced() {
	f.Server.EnsureSynced()
}

// ConnectDeltaADS starts a Delta ADS connection to the server. It will automatically be cleaned up when the test ends
func (f Fake) ConnectDeltaADS() *istioxds.DeltaAdsTest {
	//nolint:staticcheck // for testing
	conn, err := grpc.Dial("buffcon",
		grpc.WithTransportCredentials(insecure.NewCredentials()),
		//nolint:staticcheck // for testing
		grpc.WithBlock(),
		grpc.WithContextDialer(func(context.Context, string) (net.Conn, error) {
			return f.BufListener.Dial()
		}))
	if err != nil {
		f.t.Fatalf("failed to connect: %v", err)
	}
	return istioxds.NewDeltaAdsTest(f.t, conn)
}

var (
	testWorkload1 = agentgatewaysyncer.Address{
		Workload: ptr.Of(agentgatewaysyncer.PrecomputeWorkload(model.WorkloadInfo{Workload: &workloadapi.Workload{Uid: "wl1"}})),
	}
)

func TestEmptyXDS(t *testing.T) {
	s := NewFakeDiscoveryServer(t)
	ads := s.ConnectDeltaADS().WithType(krtxds.TargetTypeAddressUrl)
	ads.Request(nil)
	ads.ExpectEmptyResponse()
}

func TestXDS(t *testing.T) {
	s := NewFakeDiscoveryServer(t, testWorkload1)
	ads := s.ConnectDeltaADS().WithType(krtxds.TargetTypeAddressUrl)
	ads.RequestResponseAck(nil)
}

func TestXDSUpdate(t *testing.T) {
	s := NewFakeDiscoveryServer(t, testWorkload1)
	ads := s.ConnectDeltaADS().WithType(krtxds.TargetTypeAddressUrl)
	ads.RequestResponseAck(nil)

	wl1Updated := agentgatewaysyncer.Address{
		Workload: ptr.Of(agentgatewaysyncer.PrecomputeWorkload(model.WorkloadInfo{Workload: &workloadapi.Workload{Uid: "wl1", ClusterId: "cluster1"}})),
	}
	s.Addresses.UpdateObject(wl1Updated)
	resp := ads.ExpectResponse()
	assert.Equal(t, len(resp.Resources), 1)
	assert.Equal(t, len(resp.RemovedResources), 0)
	ads.ExpectNoResponse()

	s.Addresses.DeleteObject("wl1")
	resp = ads.ExpectResponse()
	assert.Equal(t, len(resp.Resources), 0)
	assert.Equal(t, len(resp.RemovedResources), 1)
}

func TestXDSDisconnect(t *testing.T) {
	t.Run("addresses", func(t *testing.T) {
		s := NewFakeDiscoveryServer(t, testWorkload1)
		ads := s.ConnectDeltaADS().WithType(krtxds.TargetTypeAddressUrl)
		ads.RequestResponseAck(nil)
		ads.Cleanup()

		wl2 := agentgatewaysyncer.Address{
			Workload: ptr.Of(agentgatewaysyncer.PrecomputeWorkload(model.WorkloadInfo{Workload: &workloadapi.Workload{Uid: "wl2", ClusterId: "cluster1"}})),
		}
		s.Addresses.DeleteObject("wl1")
		s.Addresses.UpdateObject(wl2)

		assert.EventuallyEqual(t, func() bool {
			col := s.Server.Collections[krtxds.TargetTypeAddressUrl].Col
			return col.GetKey("wl2") != nil && col.GetKey("wl1") == nil
		}, true)
		ads = s.ConnectDeltaADS().WithType(krtxds.TargetTypeAddressUrl)
		ads.Request(&discovery.DeltaDiscoveryRequest{
			ResourceNamesSubscribe:   []string{"*"},
			ResourceNamesUnsubscribe: []string{"*"},
			InitialResourceVersions: map[string]string{
				"wl1": "",
			},
		})
		resp := ads.ExpectResponse()
		// We should see wl1 deleted, wl2 added
		assert.Equal(t, len(resp.Resources), 1)
		assert.Equal(t, len(resp.RemovedResources), 1)
	})
	t.Run("resource", func(t *testing.T) {
		bind1 := agwir.AgwResource{
			Resource: &api.Resource{
				Kind: &api.Resource_Bind{Bind: &api.Bind{Key: "bind1"}},
			},
		}
		s := NewFakeDiscoveryServerWith(t, nil, []agwir.AgwResource{bind1})
		ads := s.ConnectDeltaADS().WithType(krtxds.TargetTypeResourceUrl)
		ads.RequestResponseAck(nil)
		ads.Cleanup()

		bind2 := agwir.AgwResource{
			Resource: &api.Resource{
				Kind: &api.Resource_Bind{Bind: &api.Bind{Key: "bind2"}},
			},
		}
		s.Resources.DeleteObject("bind/bind1")
		s.Resources.UpdateObject(bind2)

		assert.EventuallyEqual(t, func() bool {
			col := s.Server.Collections[krtxds.TargetTypeResourceUrl].Col
			return col.GetKey("//bind/bind2") != nil && col.GetKey("//bind/bind1") == nil
		}, true)
		ads = s.ConnectDeltaADS().WithType(krtxds.TargetTypeResourceUrl)
		ads.Request(&discovery.DeltaDiscoveryRequest{
			ResourceNamesSubscribe:   []string{"*"},
			ResourceNamesUnsubscribe: []string{"*"},
			InitialResourceVersions: map[string]string{
				"bind1": "",
			},
		})
		resp := ads.ExpectResponse()
		// We should see wl1 deleted, wl2 added
		assert.Equal(t, len(resp.Resources), 1)
		assert.Equal(t, len(resp.RemovedResources), 1)
	})
}
