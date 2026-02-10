// This file is derived from https://github.com/istio/istio/blob/master/pilot/pkg/xds/delta.go (Apache 2.0)
// The primary changes are stripping out the majority of the not-relevant code that is specific to Istio, and building
// on the KRT generator layer.

package krtxds

import (
	"context"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"sync"
	stdatomic "sync/atomic"
	"time"

	envoycorev3 "github.com/envoyproxy/go-control-plane/envoy/config/core/v3"
	discovery "github.com/envoyproxy/go-control-plane/envoy/service/discovery/v3"
	"github.com/google/uuid"
	"go.uber.org/atomic"
	"golang.org/x/time/rate"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/peer"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
	"istio.io/istio/pilot/pkg/features"
	istiogrpc "istio.io/istio/pilot/pkg/grpc"
	"istio.io/istio/pilot/pkg/model"
	"istio.io/istio/pilot/pkg/util/protoconv"
	"istio.io/istio/pkg/env"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/maps"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/security"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	"istio.io/istio/pkg/xds"
	"k8s.io/apimachinery/pkg/types"

	_ "istio.io/istio/pkg/util/protomarshal" // Ensure we get the more efficient vtproto gRPC encoder

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/nack"
	"github.com/kgateway-dev/kgateway/v2/pkg/logging"
	"github.com/kgateway-dev/kgateway/v2/pkg/metrics"
	"github.com/kgateway-dev/kgateway/v2/pkg/pluginsdk/krtutil"
)

var (
	log                 = logging.New("krtxds")
	agentGwXdsSubsystem = "agentgateway_xds"
	xdsRejectsTotal     = metrics.NewCounter(
		metrics.CounterOpts{
			Subsystem: agentGwXdsSubsystem,
			Name:      "rejects_total",
			Help:      "Total number of xDS responses rejected by agentgateway proxy",
		}, nil)
)

type CollectionRegistration struct {
	Start     func(stop <-chan struct{})
	HasSynced func() bool
}

type Registration func(*DiscoveryServer) CollectionRegistration

func TypeName[T proto.Message]() string {
	ft := new(T)
	return "type.googleapis.com/" + string((*ft).ProtoReflect().Descriptor().FullName())
}

type IntoProto[T proto.Message] interface {
	IntoProto() T
}

type IntoResourceName interface {
	XDSResourceName() string
}

type DiscoveryResource struct {
	*discovery.Resource
	ForGateway *types.NamespacedName
}

func (d DiscoveryResource) Equals(other DiscoveryResource) bool {
	return protoconv.Equals(d.Resource, other.Resource) && ptr.Equal(d.ForGateway, other.ForGateway)
}

func (d DiscoveryResource) IsForGateway(other types.NamespacedName) bool {
	// Not scoped || collection is scoped but this resource isn't || it is scoped to this one
	return d.ForGateway == nil || *d.ForGateway == (types.NamespacedName{}) || *d.ForGateway == other
}

func (d DiscoveryResource) ResourceName() string {
	if d.ForGateway != nil {
		return d.ForGateway.String() + "/" + d.Name
	}
	return d.Name
}

func getKey[T any](t T) string {
	if xx, ok := any(t).(IntoResourceName); ok {
		return xx.XDSResourceName()
	}
	return krt.GetKey(t)
}

func PerGatewayCollection[T IntoProto[TT], TT proto.Message](collection krt.Collection[T], extract func(o T) types.NamespacedName, krtopts krtutil.KrtOptions) Registration {
	return func(s *DiscoveryServer) CollectionRegistration {
		nc := krt.NewCollection(collection, func(ctx krt.HandlerContext, i T) *DiscoveryResource {
			var forGateway *types.NamespacedName
			if extract != nil {
				forGateway = ptr.Of(extract(i))
			}
			return &DiscoveryResource{
				Resource: &discovery.Resource{
					Name:         getKey(i),
					Version:      "",
					Resource:     protoconv.MessageToAny(i.IntoProto()),
					Ttl:          nil,
					CacheControl: nil,
					Metadata:     nil,
				},
				ForGateway: forGateway,
			}
		}, krtopts.ToOptions(fmt.Sprintf("XDS/%s", TypeName[TT]()))...)

		t := TypeName[TT]()
		s.Collections[t] = CollectionGenerator{
			PerGateway: extract != nil,
			Col:        nc,
		}
		synced := atomic.NewBool(false)
		start := func(stop <-chan struct{}) {
			handler := nc.RegisterBatch(func(o []krt.Event[DiscoveryResource]) {
				un := make(sets.String, len(o))
				for _, oo := range o {
					un.Insert(oo.Latest().Name)
				}
				pr := PushRequest{
					ConfigsUpdated: map[TypeUrl]sets.String{
						TypeUrl(t): un,
					},
				}
				s.InboundUpdates.Inc()
				s.pushChannel <- &pr
			}, true)
			go func() {
				handler.WaitUntilSynced(stop)
				synced.Store(true)
			}()
		}
		return CollectionRegistration{
			Start:     start,
			HasSynced: synced.Load,
		}
	}
}

func Collection[T IntoProto[TT], TT proto.Message](collection krt.Collection[T], krtopts krtutil.KrtOptions) Registration {
	return PerGatewayCollection(collection, nil, krtopts)
}

var (
	DebounceAfter = env.Register(
		"KGW_DEBOUNCE_AFTER",
		10*time.Millisecond,
		"The delay added to config/registry events for debouncing. This will delay the push by "+
			"at least this interval. If no change is detected within this period, the push will happen, "+
			" otherwise we'll keep delaying until things settle, up to a max of KGW_DEBOUNCE_MAX.",
	).Get()

	DebounceMax = env.Register(
		"KGW_DEBOUNCE_MAX",
		1*time.Second,
		"The maximum amount of time to wait for events while debouncing. If events keep showing up with no breaks "+
			"for this time, we'll trigger a push.",
	).Get()
)

// NewDiscoveryServer creates a DiscoveryServer for agentgateway that sources data from KRT collections via registered generators
func NewDiscoveryServer(debugger *krt.DebugHandler, nackPublisher *nack.Publisher, reg ...Registration) *DiscoveryServer {
	out := &DiscoveryServer{
		concurrentPushLimit: make(chan struct{}, features.PushThrottle),
		RequestRateLimit:    rate.NewLimiter(rate.Limit(features.RequestLimit), 1),
		InboundUpdates:      atomic.NewInt64(0),
		CommittedUpdates:    atomic.NewInt64(0),
		pushChannel:         make(chan *PushRequest, 10),
		pushQueue:           NewPushQueue(),
		debugHandlers:       map[string]string{},
		adsClients:          map[string]*Connection{},
		krtDebugger:         debugger,
		nackPublisher:       nackPublisher,
		DebounceOptions: DebounceOptions{
			DebounceAfter: DebounceAfter,
			DebounceMax:   DebounceMax,
		},
		Collections: make(map[string]CollectionGenerator),
	}

	for _, r := range reg {
		out.registrations = append(out.registrations, r(out))
	}

	return out
}

// DiscoveryServer is Pilot's gRPC implementation for Envoy's xds APIs
type DiscoveryServer struct {
	// Generators allow customizing the generated config, based on the client metadata.
	// Key is the generator type - will match the Generator metadata to set the per-connection
	// default generator, or the combination of Generator metadata and TypeUrl to select a
	// different generator for a type.
	// Normal istio clients use the default generator - will not be impacted by this.
	Collections map[string]CollectionGenerator

	// concurrentPushLimit is a semaphore that limits the amount of concurrent XDS pushes.
	concurrentPushLimit chan struct{}
	// RequestRateLimit limits the number of new XDS requests allowed. This helps prevent thundering hurd of incoming requests.
	RequestRateLimit *rate.Limiter

	// InboundUpdates describes the number of configuration updates the discovery server has received
	InboundUpdates *atomic.Int64
	// CommittedUpdates describes the number of configuration updates the discovery server has
	// received, process, and stored in the push context. If this number is less than InboundUpdates,
	// there are updates we have not yet processed.
	// Note: This does not mean that all proxies have received these configurations; it is strictly
	// the push context, which means that the next push to a proxy will receive this configuration.
	CommittedUpdates *atomic.Int64

	// pushChannel is the buffer used for debouncing.
	// after debouncing the pushRequest will be sent to pushQueue
	pushChannel chan *PushRequest

	// pushQueue is the buffer that used after debounce and before the real xds push.
	pushQueue *PushQueue

	// debugHandlers is the list of all the supported debug handlers.
	debugHandlers map[string]string

	// adsClients reflect active gRPC channels, for both ADS and EDS.
	adsClients      map[string]*Connection
	adsClientsMutex sync.RWMutex

	DebounceOptions DebounceOptions

	// pushVersion stores the numeric push version. This should be accessed via NextVersion()
	pushVersion atomic.Uint64

	krtDebugger   *krt.DebugHandler
	pushOrder     []string
	registrations []CollectionRegistration

	nackPublisher *nack.Publisher
}

// Proxy contains information about an specific instance of a proxy.
// The Proxy is initialized when a client connects to XDS, and populated from
// 'node' info in the protocol as well as data extracted from registries.
//
// In current implementation nodes use a 4-parts '~' delimited ID.
// Type~IPAddress~ID~Domain
type Proxy struct {
	sync.RWMutex

	// ID is the unique identifier for a client.
	ID string

	// WatchedResources contains the list of watched resources for the proxy, keyed by the DiscoveryRequest TypeUrl.
	WatchedResources map[string]*model.WatchedResource
}

type Connection struct {
	xds.Connection

	// Original node metadata, to avoid unmarshal/marshal.
	// This is included in internal events.
	node *envoycorev3.Node

	// proxy is the client to which this connection is established.
	proxy *Proxy

	// deltaStream is used for Delta XDS. Only one of deltaStream or stream will be set
	deltaStream DeltaDiscoveryStream

	deltaReqChan chan *discovery.DeltaDiscoveryRequest
}
type DeltaDiscoveryStream = discovery.AggregatedDiscoveryService_DeltaAggregatedResourcesServer

// StreamAggregatedResources implements the ADS interface.
func (s *DiscoveryServer) StreamAggregatedResources(stream discovery.AggregatedDiscoveryService_StreamAggregatedResourcesServer) error {
	return fmt.Errorf("not supported")
}

func (s *DiscoveryServer) DeltaAggregatedResources(stream discovery.AggregatedDiscoveryService_DeltaAggregatedResourcesServer) error {
	return s.StreamDeltas(stream)
}

func (s *DiscoveryServer) StreamDeltas(stream DeltaDiscoveryStream) error {
	// Check if server is ready to accept clients and process new requests.
	// Currently ready means caches have been synced and hence can build
	// clusters correctly. Without this check, InitContext() call below would
	// initialize with empty config, leading to reconnected Envoys loosing
	// configuration. This is an additional safety check inaddition to adding
	// cachesSynced logic to readiness probe to handle cases where kube-proxy
	// ip tables update latencies.
	// See https://github.com/istio/istio/issues/25495.
	if !s.IsServerReady() {
		return errors.New("server is not ready to serve discovery information")
	}

	ctx := stream.Context()
	peerAddr := "0.0.0.0"
	if peerInfo, ok := peer.FromContext(ctx); ok {
		peerAddr = peerInfo.Addr.String()
	}

	if err := s.WaitForRequestLimit(stream.Context()); err != nil {
		log.Warn("ADS: exceeded rate limit", "peer", peerAddr, "error", err)
		return status.Errorf(codes.ResourceExhausted, "request rate limit exceeded: %v", err)
	}

	id := s.authenticate(ctx)
	if id != nil {
		log.Debug("authenticated XDS", "peer", peerAddr, "identity", id)
	} else {
		log.Debug("unauthenticated XDS", "peer", peerAddr)
	}

	con := newDeltaConnection(peerAddr, stream)

	// Do not call: defer close(con.pushChannel). The push channel will be garbage collected
	// when the connection is no longer used. Closing the channel can cause subtle race conditions
	// with push. According to the spec: "It's only necessary to close a channel when it is important
	// to tell the receiving goroutines that all data have been sent."

	// Block until either a request is received or a push is triggered.
	// We need 2 go routines because 'read' blocks in Recv().
	go s.receiveDelta(con, id)

	// Wait for the proxy to be fully initialized before we start serving traffic. Because
	// initialization doesn't have dependencies that will block, there is no need to add any timeout
	// here. Prior to this explicit wait, we were implicitly waiting by receive() not sending to
	// reqChannel and the connection not being enqueued for pushes to pushChannel until the
	// initialization is complete.
	<-con.InitializedCh()

	for {
		// Go select{} statements are not ordered; the same channel can be chosen many times.
		// For requests, these are higher priority (client may be blocked on startup until these are done)
		// and often very cheap to handle (simple ACK), so we check it first.
		select {
		case req, ok := <-con.deltaReqChan:
			if ok {
				if err := s.processDeltaRequest(req, con); err != nil {
					return err
				}
			} else {
				// Remote side closed connection or error processing the request.
				return <-con.ErrorCh()
			}
		case <-con.StopCh():
			return nil
		default:
		}
		// If there wasn't already a request, poll for requests and pushes. Note: if we have a huge
		// amount of incoming requests, we may still send some pushes, as we do not `continue` above;
		// however, requests will be handled ~2x as much as pushes. This ensures a wave of requests
		// cannot completely starve pushes. However, this scenario is unlikely.
		select {
		case req, ok := <-con.deltaReqChan:
			if ok {
				if err := s.processDeltaRequest(req, con); err != nil {
					return err
				}
			} else {
				// Remote side closed connection or error processing the request.
				return <-con.ErrorCh()
			}
		case ev := <-con.PushCh():
			pushEv := ev.(*Event)
			err := s.pushConnectionDelta(con, pushEv)
			pushEv.Done()
			if err != nil {
				return err
			}
		case <-con.StopCh():
			return nil
		}
	}
}

// Compute and send the new configuration for a connection.
func (s *DiscoveryServer) pushConnectionDelta(con *Connection, pushEv *Event) error {
	pushRequest := pushEv.PushRequest

	needsPush := s.ProxyNeedsPush(con.proxy, pushRequest)
	if !needsPush {
		log.Debug("skipping push, no updates required", "connection", con.ID())
		return nil
	}

	// Send pushes to all generators
	// Each Generator is responsible for determining if the push event requires a push
	wrl := con.watchedResourcesByOrder(s.pushOrder)
	for _, w := range wrl {
		if err := s.pushDeltaXds(con, w, pushRequest); err != nil {
			return err
		}
	}

	//proxiesConvergeDelay.Record(time.Since(pushRequest.Start).Seconds())
	return nil
}

func (s *DiscoveryServer) receiveDelta(con *Connection, id *types.NamespacedName) {
	defer func() {
		close(con.deltaReqChan)
		close(con.ErrorCh())
		// Close the initialized channel, if its not already closed, to prevent blocking the stream
		select {
		case <-con.InitializedCh():
		default:
			close(con.InitializedCh())
		}
	}()
	firstRequest := true
	for {
		req, err := con.deltaStream.Recv()
		if err != nil {
			if istiogrpc.GRPCErrorType(err) != istiogrpc.UnexpectedError {
				log.Info("ADS: terminated", "peer", con.Peer(), "connection", con.ID())
				return
			}
			con.ErrorCh() <- err
			log.Error("ADS: terminated with error", "peer", con.Peer(), "connection", con.ID(), "error", err)
			xds.TotalXDSInternalErrors.Increment()
			return
		}
		// This should be only set for the first request. The node id may not be set - for example malicious clients.
		if firstRequest {
			firstRequest = false
			if req.Node == nil || req.Node.Id == "" {
				con.ErrorCh() <- status.New(codes.InvalidArgument, "missing node information").Err()
				return
			}
			if err := s.initConnection(req.Node, con, id); err != nil {
				con.ErrorCh() <- err
				return
			}
			defer s.closeConnection(con)
			log.Info("ADS: new delta connection", "node", con.ID())
		}

		select {
		case con.deltaReqChan <- req:
		case <-con.deltaStream.Context().Done():
			log.Info("ADS: terminated with stream closed", "peer", con.Peer(), "connection", con.ID())
			return
		}
	}
}

func (conn *Connection) sendDelta(res *discovery.DeltaDiscoveryResponse) error {
	sendResonse := func() error {
		start := time.Now()
		defer func() { xds.RecordSendTime(time.Since(start)) }()
		return conn.deltaStream.Send(res)
	}
	err := sendResonse()
	if err == nil {
		conn.proxy.UpdateWatchedResource(res.TypeUrl, func(wr *model.WatchedResource) *model.WatchedResource {
			if wr == nil {
				wr = &model.WatchedResource{TypeUrl: res.TypeUrl}
			}
			wr.NonceSent = res.Nonce
			wr.LastSendTime = time.Now()
			return wr
		})
	} else if status.Convert(err).Code() == codes.DeadlineExceeded {
		log.Info("timeout writing", "connection", conn.ID(), "type", GetShortType(res.TypeUrl))
		xds.ResponseWriteTimeouts.Increment()
	}
	return err
}

// processDeltaRequest is handling one request. This is currently called from the 'main' thread, which also
// handles 'push' requests and close - the code will eventually call the 'push' code, and it needs more mutex
// protection. Original code avoided the mutexes by doing both 'push' and 'process requests' in same thread.
func (s *DiscoveryServer) processDeltaRequest(req *discovery.DeltaDiscoveryRequest, con *Connection) error {
	stype := GetShortType(req.TypeUrl)
	log.Debug("ADS: REQ resources", "type", stype, "connection", con.ID(), "subscribe", len(req.ResourceNamesSubscribe), "unsubscribe", len(req.ResourceNamesUnsubscribe), "nonce", req.ResponseNonce)

	shouldRespond := shouldRespondDelta(con, req, s.nackPublisher)
	if !shouldRespond {
		log.Debug("no response needed")
		return nil
	}

	subs, _, _ := deltaWatchedResources(nil, req)
	request := &PushRequest{
		IsFromRequest: true,
		Delta: model.ResourceDelta{
			// Record sub/unsub, but drop synthetic wildcard info
			Subscribed:   subs,
			Unsubscribed: sets.New(req.ResourceNamesUnsubscribe...).Delete("*"),
		},
	}

	err := s.pushDeltaXds(con, con.proxy.GetWatchedResource(req.TypeUrl), request)
	if err != nil {
		return err
	}
	return nil
}

// shouldRespondDelta determines whether this request needs to be responded back. It applies the ack/nack rules as per xds protocol
// using WatchedResource for previous state and discovery request for the current state.
func shouldRespondDelta(con *Connection, request *discovery.DeltaDiscoveryRequest, nackPublisher *nack.Publisher) bool {
	stype := GetShortType(request.TypeUrl)

	// If there is an error in request that means previous response is erroneous.
	// We do not have to respond in that case. In this case request's version info
	// will be different from the version sent. But it is fragile to rely on that.
	if request.ErrorDetail != nil {
		// nolint: gosec // error side is bounded
		errCode := codes.Code(request.ErrorDetail.Code)
		log.Warn("ADS: ACK ERROR", "type", stype, "connection", con.ID(), "code", errCode.String(), "message", request.ErrorDetail.GetMessage())
		xdsRejectsTotal.Inc()
		con.proxy.UpdateWatchedResource(request.TypeUrl, func(wr *model.WatchedResource) *model.WatchedResource {
			wr.LastError = request.ErrorDetail.GetMessage()
			return wr
		})

		if nackPublisher != nil {
			gateway := AgentgatewayID(con.node)
			nackEvent := nack.NackEvent{
				Gateway:   gateway,
				TypeUrl:   request.TypeUrl,
				ErrorMsg:  request.ErrorDetail.GetMessage(),
				Timestamp: time.Now(),
			}
			nackPublisher.PublishNack(&nackEvent)
		}
		return false
	}

	log.Debug("ADS: REQUEST", "type", stype, "connection", con.ID(), "subscribe", request.ResourceNamesSubscribe, "unsubscribe", request.ResourceNamesUnsubscribe, "initial", request.InitialResourceVersions)
	previousInfo := con.proxy.GetWatchedResource(request.TypeUrl)

	// This can happen in two cases:
	// 1. Envoy initially send request to Istiod
	// 2. Envoy reconnect to Istiod i.e. Istiod does not have
	// information about this typeUrl, but Envoy sends response nonce - either
	// because Istiod is restarted or Envoy disconnects and reconnects.
	// We should always respond with the current resource names.
	if previousInfo == nil {
		con.proxy.Lock()
		defer con.proxy.Unlock()

		if len(request.InitialResourceVersions) > 0 {
			log.Debug("ADS: RECONNECT", "type", stype, "connection", con.ID(), "nonce", request.ResponseNonce, "resources", len(request.InitialResourceVersions))
		} else {
			log.Debug("ADS: INIT", "type", stype, "connection", con.ID(), "nonce", request.ResponseNonce)
		}

		res, wildcard, _ := deltaWatchedResources(nil, request)
		skip := request.TypeUrl == TargetTypeAddressUrl && wildcard
		if skip {
			// Due to the high resource count in WDS at scale, we do not store ResourceName.
			// See the workload generator for more information on why we don't use this.
			res = nil
		}
		con.proxy.WatchedResources[request.TypeUrl] = &model.WatchedResource{
			TypeUrl:       request.TypeUrl,
			ResourceNames: res,
			Wildcard:      wildcard,
		}
		return true
	}

	// If there is mismatch in the nonce, that is a case of expired/stale nonce.
	// A nonce becomes stale following a newer nonce being sent to Envoy.
	if request.ResponseNonce != "" && request.ResponseNonce != previousInfo.NonceSent {
		log.Debug("ADS: REQ Expired nonce received", "type", stype, "connection", con.ID(), "received", request.ResponseNonce, "sent", previousInfo.NonceSent)
		//xds.ExpiredNonce.With(typeTag.Value(v3.GetMetricType(request.TypeUrl))).Increment()
		return false
	}

	// Spontaneous DeltaDiscoveryRequests from the client.
	// This can be done to dynamically add or remove elements from the tracked resource_names set.
	// In this case response_nonce is empty.
	spontaneousReq := request.ResponseNonce == ""

	var alwaysRespond bool
	var subChanged bool

	// Update resource names, and record ACK if required.
	con.proxy.UpdateWatchedResource(request.TypeUrl, func(wr *model.WatchedResource) *model.WatchedResource {
		wr.ResourceNames, _, subChanged = deltaWatchedResources(wr.ResourceNames, request)
		if !spontaneousReq {
			// Clear last error, we got an ACK.
			// Otherwise, this is just a change in resource subscription, so leave the last ACK info in place.
			wr.LastError = ""
			wr.NonceAcked = request.ResponseNonce
		}
		alwaysRespond = wr.AlwaysRespond
		wr.AlwaysRespond = false
		return wr
	})

	// It is invalid in the below two cases:
	// 1. no subscribed resources change from spontaneous delta request.
	// 2. subscribed resources changes from ACK.
	if spontaneousReq && !subChanged || !spontaneousReq && subChanged {
		log.Error("ADS: Subscribed resources check mismatch", "type", stype, "spontaneous", spontaneousReq, "subChanged", subChanged)
		if features.EnableUnsafeAssertions {
			panic(fmt.Sprintf("ADS:%s: Subscribed resources check mismatch: %v vs %v", stype, spontaneousReq, subChanged))
		}
	}

	// Envoy can send two DiscoveryRequests with same version and nonce
	// when it detects a new resource. We should respond if they change.
	if !subChanged {
		// We should always respond "alwaysRespond" marked requests to let Envoy finish warming
		// even though Nonce match and it looks like an ACK.
		if alwaysRespond {
			log.Info("ADS: FORCE RESPONSE for warming", "type", stype, "connection", con.ID())
			return true
		}

		log.Debug("ADS: ACK", "type", stype, "connection", con.ID(), "nonce", request.ResponseNonce)
		return false
	}
	log.Debug("ADS: RESOURCE CHANGE", "type", stype, "connection", con.ID(), "nonce", request.ResponseNonce)

	return true
}

// Push a Delta XDS resource for the given connection.
func (s *DiscoveryServer) pushDeltaXds(con *Connection, w *model.WatchedResource, req *PushRequest) error {
	if w == nil {
		log.Warn("no watched resource found")
		return nil
	}
	gen, f := s.findGenerator(w.TypeUrl)
	if !f {
		log.Warn("no generator found", "type", w.TypeUrl)
		return nil
	}
	pushVersion := req.PushVersion
	gw := AgentgatewayID(con.node)
	res, deletedRes, err := gen.GenerateDeltas(req, w, gw)
	if err != nil || (res == nil && deletedRes == nil) {
		return err
	}
	//defer func() { recordPushTime(w.TypeUrl, time.Since(t0)) }()
	resp := &discovery.DeltaDiscoveryResponse{
		//ControlPlane: ControlPlane(w.TypeUrl),
		TypeUrl:           w.TypeUrl,
		SystemVersionInfo: pushVersion,
		Nonce:             nonce(pushVersion),
		Resources:         res,
		RemovedResources:  deletedRes,
	}
	if len(resp.RemovedResources) > 0 {
		log.Debug("ADS: REMOVE", "type", GetShortType(w.TypeUrl), "node", con.ID(), "removed", resp.RemovedResources)
	}

	configSize := ResourceSize(res)
	//configSizeBytes.With(typeTag.Value(w.TypeUrl)).Record(float64(configSize))

	if err := con.sendDelta(resp); err != nil {
		log.Debug("send failure", "type", GetShortType(w.TypeUrl), "node", con.proxy.ID, "resources", len(res), "size", ByteCount(configSize), "error", err)
		return err
	}

	log.Info("push response",
		"type", GetShortType(w.TypeUrl),
		"reason", req.PushReason(),
		"node", con.proxy.ID,
		"resources", len(res),
		"removed", len(resp.RemovedResources),
		"size", ByteCount(configSize))

	return nil
}

func (s *DiscoveryServer) IsServerReady() bool {
	for _, r := range s.registrations {
		if !r.HasSynced() {
			return false
		}
	}
	return true
}

func newDeltaConnection(peerAddr string, stream DeltaDiscoveryStream) *Connection {
	return &Connection{
		Connection:   xds.NewConnection(peerAddr, nil),
		deltaStream:  stream,
		deltaReqChan: make(chan *discovery.DeltaDiscoveryRequest, 1),
	}
}

// deltaWatchedResources returns current watched resources of delta xds
func deltaWatchedResources(existing sets.String, request *discovery.DeltaDiscoveryRequest) (sets.String, bool, bool) {
	res := existing
	if res == nil {
		res = sets.New[string]()
	}
	changed := false
	for _, r := range request.ResourceNamesSubscribe {
		if !res.InsertContains(r) {
			changed = true
		}
	}
	// This is set by Envoy on first request on reconnection so that we are aware of what Envoy knows
	// and can continue the xDS session properly.
	for r := range request.InitialResourceVersions {
		if !res.InsertContains(r) {
			changed = true
		}
	}
	for _, r := range request.ResourceNamesUnsubscribe {
		if res.DeleteContains(r) {
			changed = true
		}
	}
	wildcard := false
	// A request is wildcard if they explicitly subscribe to "*" or subscribe to nothing
	if res.Contains("*") {
		wildcard = true
		res.Delete("*")
	}
	// "if the client sends a request but has never explicitly subscribed to any resource names, the
	// server should treat that identically to how it would treat the client having explicitly
	// subscribed to *"
	// NOTE: this means you cannot subscribe to nothing, which is useful for on-demand loading; to workaround this
	// Istio clients will send and initial request both subscribing+unsubscribing to `*`.
	if len(request.ResourceNamesSubscribe) == 0 {
		wildcard = true
	}
	return res, wildcard, changed
}

// Clients returns all currently connected clients. This method can be safely called concurrently,
// but care should be taken with the underlying objects (ie model.Proxy) to ensure proper locking.
// This method returns only fully initialized connections; for all connections, use AllClients
func (s *DiscoveryServer) Clients() []*Connection {
	s.adsClientsMutex.RLock()
	defer s.adsClientsMutex.RUnlock()
	clients := make([]*Connection, 0, len(s.adsClients))
	for _, con := range s.adsClients {
		select {
		case <-con.InitializedCh():
		default:
			// Initialization not complete, skip
			continue
		}
		clients = append(clients, con)
	}
	return clients
}

// AllClients returns all connected clients, per Clients, but additionally includes uninitialized connections
// Warning: callers must take care not to rely on the con.proxy field being set
func (s *DiscoveryServer) AllClients() []*Connection {
	s.adsClientsMutex.RLock()
	defer s.adsClientsMutex.RUnlock()
	return maps.Values(s.adsClients)
}

func (s *DiscoveryServer) WaitForRequestLimit(ctx context.Context) error {
	if s.RequestRateLimit.Limit() == 0 {
		// Allow opt out when rate limiting is set to 0qps
		return nil
	}
	// Give a bit of time for queue to clear out, but if not fail fast. Client will connect to another
	// instance in best case, or retry with backoff.
	wait, cancel := context.WithTimeout(ctx, time.Second)
	defer cancel()
	return s.RequestRateLimit.Wait(wait)
}

func (s *DiscoveryServer) NextVersion() string {
	return time.Now().Format(time.RFC3339) + "/" + strconv.FormatUint(s.pushVersion.Inc(), 10)
}

func (s *DiscoveryServer) authenticate(ctx context.Context) *types.NamespacedName {
	peer, ok := ctx.Value(PeerCtxKey).(*security.Caller)
	if !ok {
		// Not authenticated. If XDS auth was enabled, this will be rejected by the middleware, so no need to fail here
		return nil
	}
	return &types.NamespacedName{
		Namespace: peer.KubernetesInfo.PodNamespace,
		// We assume the SA and gateway name are the same
		Name: peer.KubernetesInfo.PodServiceAccount,
	}
}

func (s *DiscoveryServer) ProxyNeedsPush(proxy *Proxy, request *PushRequest) bool {
	return true
	// TODO(krt) make pushrequest a {type,name} and then filter if we dont watch any... maybe? Does it help?
}

// watchedResourcesByOrder returns the ordered list of
// watched resources for the proxy, ordered in accordance with known push order.
func (conn *Connection) watchedResourcesByOrder(pushOrder []string) []*model.WatchedResource {
	allWatched := conn.proxy.ShallowCloneWatchedResources()
	ordered := make([]*model.WatchedResource, 0, len(allWatched))
	// first add all known types, in order
	pushOrderSet := sets.New(pushOrder...)
	for _, tp := range pushOrder {
		if allWatched[tp] != nil {
			ordered = append(ordered, allWatched[tp])
		}
	}
	// Then add any undeclared types
	for tp, res := range allWatched {
		if !pushOrderSet.Contains(tp) {
			ordered = append(ordered, res)
		}
	}
	return ordered
}

// update the node associated with the connection, after receiving a packet from envoy, also adds the connection
// to the tracking map.
func (s *DiscoveryServer) initConnection(node *envoycorev3.Node, con *Connection, id *types.NamespacedName) error {
	// Setup the initial proxy metadata
	proxy := s.initProxyMetadata(node)
	// First request so initialize connection id and start tracking it.
	con.SetID(connectionID(proxy.ID))
	con.node = node
	con.proxy = proxy

	// Authorize xds clients
	if id != nil {
		reqId := AgentgatewayID(con.node)
		if reqId != *id {
			return fmt.Errorf("requested gateway %v but authenticated as %v", reqId, *id)
		}
	}

	// Register the connection. this allows pushes to be triggered for the proxy. Note: the timing of
	// this and initializeProxy important. While registering for pushes *after* initialization is complete seems like
	// a better choice, it introduces a race condition; If we complete initialization of a new push
	// context between initializeProxy and addCon, we would not get any pushes triggered for the new
	// push context, leading the proxy to have a stale state until the next full push.
	s.addCon(con.ID(), con)
	// Register that initialization is complete. This triggers to calls that it is safe to access the
	// proxy
	defer con.MarkInitialized()
	proxy.WatchedResources = map[string]*model.WatchedResource{}
	return nil
}

func (s *DiscoveryServer) closeConnection(con *Connection) {
	if con.ID() == "" {
		return
	}
	s.removeCon(con.ID())
}

func (s *DiscoveryServer) addCon(conID string, con *Connection) {
	s.adsClientsMutex.Lock()
	defer s.adsClientsMutex.Unlock()
	s.adsClients[conID] = con
	//recordXDSClients(con.proxy.Metadata.IstioVersion, 1)
}

func (s *DiscoveryServer) removeCon(conID string) {
	s.adsClientsMutex.Lock()
	defer s.adsClientsMutex.Unlock()

	if _, exist := s.adsClients[conID]; !exist {
		log.Error("ADS: Removing connection for non-existing node", "node", conID)
		//xds.TotalXDSInternalErrors.Increment()
	} else {
		delete(s.adsClients, conID)
		//recordXDSClients(con.proxy.Metadata.IstioVersion, -1)
	}
}

// Debouncing and push request happens in a separate thread, it uses locks
// and we want to avoid complications, ConfigUpdate may already hold other locks.
// handleUpdates processes events from pushChannel
// It ensures that at minimum minQuiet time has elapsed since the last event before processing it.
// It also ensures that at most maxDelay is elapsed between receiving an event and processing it.
func (s *DiscoveryServer) handleUpdates(stopCh <-chan struct{}) {
	debounce(s.pushChannel, stopCh, s.DebounceOptions, s.Push, s.CommittedUpdates)
}

func (s *DiscoveryServer) adsClientCount() int {
	s.adsClientsMutex.RLock()
	defer s.adsClientsMutex.RUnlock()
	return len(s.adsClients)
}

// Shutdown shuts down DiscoveryServer components.
func (s *DiscoveryServer) Shutdown() {
	s.pushQueue.ShutDown()
}

// EnsureSynced waits until all pending debounce events have been processed.
// This is useful in tests to ensure that no spurious pushes will occur after
// connecting a client.
func (s *DiscoveryServer) EnsureSynced() {
	target := s.InboundUpdates.Load()
	for s.CommittedUpdates.Load() < target {
		time.Sleep(time.Millisecond)
	}
}

func (s *DiscoveryServer) Start(stopCh <-chan struct{}) {
	go s.handleUpdates(stopCh)
	go s.sendPushes(stopCh)
	for _, reg := range s.registrations {
		reg.Start(stopCh)
	}
}

func (s *DiscoveryServer) sendPushes(stopCh <-chan struct{}) {
	semaphore := s.concurrentPushLimit
	queue := s.pushQueue
	for {
		select {
		case <-stopCh:
			return
		default:
			// We can send to it until it is full, then it will block until a pushes finishes and reads from it.
			// This limits the number of pushes that can happen concurrently
			semaphore <- struct{}{}

			// Get the next proxy to push. This will block if there are no updates required.
			client, push, shuttingdown := queue.Dequeue()
			if shuttingdown {
				return
			}
			//recordPushTriggers(push.Reason)
			// Signals that a push is done by reading from the semaphore, allowing another send on it.
			doneFunc := func() {
				queue.MarkDone(client)
				<-semaphore
			}

			//proxiesQueueTime.Record(time.Since(push.Start).Seconds())
			var closed <-chan struct{}
			if client.deltaStream != nil {
				closed = client.deltaStream.Context().Done()
			} else {
				closed = client.StreamDone()
			}
			go func() {
				pushEv := &Event{
					PushRequest: push,
					Done:        doneFunc,
				}

				select {
				case client.PushCh() <- pushEv:
					return
				case <-closed: // grpc stream was closed
					doneFunc()
					log.Info("client closed connection", "id", client.ID())
				}
			}()
		}
	}
}

// Push is called to push changes on config updates using ADS.
func (s *DiscoveryServer) Push(req *PushRequest) {
	version := s.NextVersion()
	log.Info("XDS: Pushing", "clients", s.adsClientCount(), "version", version)

	req.PushVersion = version
	for _, p := range s.AllClients() {
		s.pushQueue.Enqueue(p, req)
	}
}

type DebounceOptions struct {
	// DebounceAfter is the delay added to events to wait
	// after a registry/config event for debouncing.
	// This will delay the push by at least this interval, plus
	// the time getting subsequent events. If no change is
	// detected the push will happen, otherwise we'll keep
	// delaying until things settle.
	DebounceAfter time.Duration

	// debounceMax is the maximum time to wait for events
	// while debouncing. Defaults to 10 seconds. If events keep
	// showing up with no break for this time, we'll trigger a push.
	DebounceMax time.Duration
}

// The debounce helper function is implemented to enable mocking
func debounce(ch chan *PushRequest, stopCh <-chan struct{}, opts DebounceOptions, pushFn func(req *PushRequest), updateSent *atomic.Int64) {
	var timeChan <-chan time.Time
	var startDebounce time.Time
	var lastConfigUpdateTime time.Time

	pushCounter := 0
	debouncedEvents := 0

	// Keeps track of the push requests. If updates are debounce they will be merged.
	var req *PushRequest

	free := true
	freeCh := make(chan struct{}, 1)

	push := func(req *PushRequest, debouncedEvents int) {
		pushFn(req)
		updateSent.Add(int64(debouncedEvents))
		//debounceTime.Record(time.Since(startDebounce).Seconds())
		freeCh <- struct{}{}
	}

	pushWorker := func() {
		eventDelay := time.Since(startDebounce)
		quietTime := time.Since(lastConfigUpdateTime)
		// it has been too long or quiet enough
		if eventDelay >= opts.DebounceMax || quietTime >= opts.DebounceAfter {
			if req != nil {
				pushCounter++
				if req.ConfigsUpdated == nil {
					log.Info("push debounce stable",
						"id", pushCounter,
						"debouncedEvents", debouncedEvents,
						"lastChange", quietTime.String(),
						"lastPush", eventDelay.String())
				} else {
					log.Info("push debounce stable",
						"id", pushCounter,
						"debouncedEvents", debouncedEvents,
						"lastChange", quietTime.String(),
						"lastPush", eventDelay.String(),
						"cause", configsUpdated(req),
					)
				}
				free = false
				go push(req, debouncedEvents)
				req = nil
				debouncedEvents = 0
			}
		} else {
			timeChan = time.After(opts.DebounceAfter - quietTime)
		}
	}

	for {
		select {
		case <-freeCh:
			free = true
			pushWorker()
		case r := <-ch:
			lastConfigUpdateTime = time.Now()
			if debouncedEvents == 0 {
				timeChan = time.After(opts.DebounceAfter)
				startDebounce = lastConfigUpdateTime
			}
			debouncedEvents++

			req = req.Merge(r)
		case <-timeChan:
			if free {
				pushWorker()
			}
		case <-stopCh:
			return
		}
	}
}

func configsUpdated(req *PushRequest) string {
	var configs strings.Builder
	count := 0
	for _, keys := range req.ConfigsUpdated {
		count += len(keys)
		for key := range keys {
			configs.WriteString(key)
			break
		}
	}
	if count > 1 {
		more := " and " + strconv.Itoa(count-1) + " more configs"
		configs.WriteString(more)
	}
	return configs.String()
}

func nonce(noncePrefix string) string {
	return noncePrefix + uuid.New().String()
}

// initProxyMetadata initializes just the basic metadata of a proxy. This is decoupled from
// initProxyState such that we can perform authorization before attempting expensive computations to
// fully initialize the proxy.
func (s *DiscoveryServer) initProxyMetadata(node *envoycorev3.Node) *Proxy {
	return &Proxy{
		RWMutex:          sync.RWMutex{},
		ID:               node.Id,
		WatchedResources: nil,
	}
}

func (s *DiscoveryServer) findGenerator(url string) (CollectionGenerator, bool) {
	c, f := s.Collections[url]
	if f {
		return c, f
	}
	return CollectionGenerator{}, false
}

var connectionNumber = int64(0)

func connectionID(node string) string {
	id := stdatomic.AddInt64(&connectionNumber, 1)
	return node + "-" + strconv.FormatInt(id, 10)
}

type CollectionGenerator struct {
	PerGateway bool
	Col        krt.Collection[DiscoveryResource]
}

// GenerateDeltas computes Workload resources. This is design to be highly optimized to delta updates,
// and supports *on-demand* client usage. A client can subscribe with a wildcard subscription and get all
// resources (with delta updates), or on-demand and only get responses for specifically subscribed resources.
//
// Incoming requests may be for VIP or Pod IP addresses. However, all responses are Workload resources, which are pod based.
// This means subscribing to a VIP may end up pushing many resources of different name than the request.
// On-demand clients are expected to handle this (for wildcard, this is not applicable, as they don't specify any resources at all).
func (e CollectionGenerator) GenerateDeltas(req *PushRequest, w *model.WatchedResource, gw types.NamespacedName) (model.Resources, model.DeletedResources, error) {
	if req.IsRequest() {
		// Full update, expect everything
		res := slices.MapFilter(e.Col.List(), func(e DiscoveryResource) **discovery.Resource {
			if !e.IsForGateway(gw) {
				return nil
			}
			return &e.Resource
		})
		toDeleted := req.Delta.Subscribed
		for _, r := range res {
			toDeleted.Delete(r.Name)
		}
		deletes := sets.SortedList(toDeleted)
		return res, deletes, nil
	}
	k := req.ConfigsUpdated[TypeUrl(w.TypeUrl)]
	res := make([]*discovery.Resource, 0, len(k))
	var deletes []string

	for k := range k {
		originalKey := k
		var keys []string
		if e.PerGateway {
			// Lookup both unscoped and for our gateway
			keys = []string{types.NamespacedName{}.String() + "/" + k, gw.String() + "/" + k}
		} else {
			// Just lookup the key, no need to worry about gateways
			keys = []string{k}
		}
		found := false
		for _, key := range keys {
			v := e.Col.GetKey(key)
			if v != nil && !v.IsForGateway(gw) {
				v = nil
			}
			if v != nil {
				found = true
				res = append(res, v.Resource)
				break
			}
		}
		if !found {
			deletes = append(deletes, originalKey)
		}
	}

	if len(res) == 0 && len(deletes) == 0 {
		// No changes
		return nil, nil, nil
	}

	return res, deletes, nil
}

type TypeUrl string

// PushRequest defines a request to push to proxies
// It is used to send updates to the config update debouncer and pass to the PushQueue.
type PushRequest struct {
	// ConfigsUpdated keeps track of configs that have changed.
	ConfigsUpdated map[TypeUrl]sets.String

	IsFromRequest bool

	// PushVersion represent the version of the push
	PushVersion string

	// Delta defines the resources that were added or removed as part of this push request.
	// This is set only on requests from the client which change the set of resources they (un)subscribe from.
	Delta xds.ResourceDelta
}

func (r PushRequest) IsRequest() bool {
	// TODO(krt)
	return r.IsFromRequest
}
func (pr *PushRequest) PushReason() string {
	if pr.IsRequest() {
		return " request"
	}
	return ""
}

// Merge two update requests together
// Merge behaves similarly to a list append; usage should in the form `a = a.merge(b)`.
// Importantly, Merge may decide to allocate a new PushRequest object or reuse the existing one - both
// inputs should not be used after completion.
func (pr *PushRequest) Merge(other *PushRequest) *PushRequest {
	if pr == nil {
		return other
	}
	if other == nil {
		return pr
	}

	if pr.ConfigsUpdated == nil {
		pr.ConfigsUpdated = other.ConfigsUpdated
	} else {
		for k, v := range other.ConfigsUpdated {
			if e, f := pr.ConfigsUpdated[k]; f {
				e.Merge(v)
			} else {
				pr.ConfigsUpdated[k] = v
			}
		}
	}

	return pr
}

// Event represents a config or registry event that results in a push.
type Event struct {
	// PushRequest PushRequest to use for the push.
	PushRequest *PushRequest

	// function to call once a push is finished. This must be called or future changes may be blocked.
	Done func()
}

func (node *Proxy) UpdateWatchedResource(typeURL string, updateFn func(*model.WatchedResource) *model.WatchedResource) {
	node.Lock()
	defer node.Unlock()
	r := node.WatchedResources[typeURL]
	r = updateFn(r)
	if r != nil {
		node.WatchedResources[typeURL] = r
	} else {
		delete(node.WatchedResources, typeURL)
	}
}
func (node *Proxy) DeleteWatchedResource(typeURL string) {
	node.Lock()
	defer node.Unlock()

	delete(node.WatchedResources, typeURL)
}

func (node *Proxy) AddOrUpdateWatchedResource(r *model.WatchedResource) {
	if r == nil {
		return
	}
	node.Lock()
	defer node.Unlock()
	node.WatchedResources[r.TypeUrl] = r
}

func (node *Proxy) GetWatchedResourceTypes() sets.String {
	node.RLock()
	defer node.RUnlock()

	ret := sets.NewWithLength[string](len(node.WatchedResources))
	for typeURL := range node.WatchedResources {
		ret.Insert(typeURL)
	}
	return ret
}

func (node *Proxy) GetWatchedResource(typeURL string) *model.WatchedResource {
	node.RLock()
	defer node.RUnlock()

	return node.WatchedResources[typeURL]
}

// ShallowCloneWatchedResources clones the watched resources, both the keys and values are shallow copy.
func (node *Proxy) ShallowCloneWatchedResources() map[string]*model.WatchedResource {
	node.RLock()
	defer node.RUnlock()
	return maps.Clone(node.WatchedResources)
}

// DeepCloneWatchedResources clones the watched resources
func (node *Proxy) DeepCloneWatchedResources() map[string]model.WatchedResource {
	node.RLock()
	defer node.RUnlock()
	m := make(map[string]model.WatchedResource, len(node.WatchedResources))
	for k, v := range node.WatchedResources {
		m[k] = *v
	}
	return m
}

func ResourceSize(r model.Resources) int {
	// Approximate size by looking at the Any marshaled size. This avoids high cost
	// proto.Size, at the expense of slightly under counting.
	size := 0
	for _, r := range r {
		size += len(r.Resource.Value)
	}
	return size
}


const (
	TargetTypeResourceUrl = "type.googleapis.com/agentgateway.dev.resource.Resource"
	TargetTypeAddressUrl  = "type.googleapis.com/istio.workload.Address"
)

// GetShortType returns an abbreviated form of a type, useful for logging or human friendly messages
func GetShortType(typeURL string) string {
	switch typeURL {
	case TargetTypeAddressUrl:
		return "WDS"
	case TargetTypeResourceUrl:
		return "RDS"
	default:
		return typeURL
	}
}

func AgentgatewayID(node *envoycorev3.Node) types.NamespacedName {
	if node.GetMetadata() != nil {
		roleValue := node.GetMetadata().GetFields()[RoleKey]
		if roleValue != nil {
			s := roleValue.GetStringValue()
			ns, name, ok := strings.Cut(s, KeyDelimiter)
			if ok {
				return types.NamespacedName{
					Namespace: ns,
					Name:      name,
				}
			}
		}
	}

	return types.NamespacedName{}
}

const (
	// KeyDelimiter is the character used to join segments of a cache key
	KeyDelimiter = "~"

	// RoleKey is the name of the ket in the node.metadata used to store the role
	RoleKey = "role"

	// PeerCtxKey is the key used to store the peer information in the context
	PeerCtxKey = "peer"
)

// ByteCount returns a human readable byte format
// Inspired by https://yourbasic.org/golang/formatting-byte-size-to-human-readable-format/
func ByteCount(b int) string {
	const unit = 1000
	if b < unit {
		return fmt.Sprintf("%dB", b)
	}
	div, exp := int64(unit), 0
	for n := b / unit; n >= unit; n /= unit {
		div *= unit
		exp++
	}
	return fmt.Sprintf("%.1f%cB",
		float64(b)/float64(div), "kMGTPE"[exp])
}
