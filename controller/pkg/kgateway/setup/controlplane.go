package setup

import (
	"context"
	"crypto/tls"
	"fmt"
	"log/slog"
	"math"
	"net"

	envoy_service_discovery_v3 "github.com/envoyproxy/go-control-plane/envoy/service/discovery/v3"
	grpc_middleware "github.com/grpc-ecosystem/go-grpc-middleware"
	grpc_zap "github.com/grpc-ecosystem/go-grpc-middleware/logging/zap"
	"go.uber.org/zap"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/reflection"
	"istio.io/istio/pkg/security"
	"sigs.k8s.io/controller-runtime/pkg/certwatcher"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/krtxds"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/agentgatewaysyncer/nack"
	"github.com/kgateway-dev/kgateway/v2/pkg/metrics"
)

const (
	xdsSubsystem = "xds"
)

var (
	xdsAuthRequestTotal = metrics.NewCounter(
		metrics.CounterOpts{
			Subsystem: xdsSubsystem,
			Name:      "auth_rq_total",
			Help:      "Total number of xDS auth requests",
		}, nil)

	xdsAuthSuccessTotal = metrics.NewCounter(
		metrics.CounterOpts{
			Subsystem: xdsSubsystem,
			Name:      "auth_rq_success_total",
			Help:      "Total number of successful xDS auth requests",
		}, nil)

	xdsAuthFailureTotal = metrics.NewCounter(
		metrics.CounterOpts{
			Subsystem: xdsSubsystem,
			Name:      "auth_rq_failure_total",
			Help:      "Total number of failed xDS auth requests",
		}, nil)
)

func NewAgwControlPlane(
	ctx context.Context,
	lis net.Listener,
	authenticators []security.Authenticator,
	xdsAuth bool,
	certWatcher *certwatcher.CertWatcher,
	nackPublisher *nack.Publisher,
	reg ...krtxds.Registration,
) {
	baseLogger := slog.Default().With("component", "agentgateway-controlplane")

	serverOpts := getGRPCServerOpts(authenticators, xdsAuth, certWatcher, baseLogger)
	grpcServer := grpc.NewServer(serverOpts...)

	ds := krtxds.NewDiscoveryServer(nil, nackPublisher, reg...)
	stop := make(chan struct{})
	context.AfterFunc(ctx, func() {
		close(stop)
	})
	ds.Start(stop)

	reflection.Register(grpcServer)
	envoy_service_discovery_v3.RegisterAggregatedDiscoveryServiceServer(grpcServer, ds)

	baseLogger.Info("starting server", "address", lis.Addr().String())
	go grpcServer.Serve(lis)

	go func() {
		<-ctx.Done()
		grpcServer.GracefulStop()
	}()
}

func getGRPCServerOpts(
	authenticators []security.Authenticator,
	xdsAuth bool,
	certWatcher *certwatcher.CertWatcher,
	logger *slog.Logger,
) []grpc.ServerOption {
	opts := []grpc.ServerOption{
		grpc.MaxRecvMsgSize(math.MaxInt32),
		grpc.StreamInterceptor(
			grpc_middleware.ChainStreamServer(
				grpc_zap.StreamServerInterceptor(zap.NewNop()),
				func(srv any, ss grpc.ServerStream, info *grpc.StreamServerInfo, handler grpc.StreamHandler) error {
					slog.Debug("gRPC call", "method", info.FullMethod)
					if xdsAuth {
						xdsAuthRequestTotal.Inc()
						am := authenticationManager{
							Authenticators: authenticators,
						}
						if u := am.authenticate(ss.Context()); u != nil {
							xdsAuthSuccessTotal.Inc()
							return handler(srv, &grpc_middleware.WrappedServerStream{
								ServerStream:   ss,
								WrappedContext: context.WithValue(ss.Context(), krtxds.PeerCtxKey, u),
							})
						}
						xdsAuthFailureTotal.Inc()
						slog.Error("authentication failed", "reasons", am.authFailMsgs)
						return fmt.Errorf("authentication failed: %v", am.authFailMsgs)
					} else {
						slog.Warn("xDS authentication is disabled")
						return handler(srv, ss)
					}
				},
			)),
	}

	// Add TLS credentials if the certificate watcher was provided. Needed to react to
	// certificate rotations to ensure we're always serving the latest CA certificate.
	if certWatcher != nil {
		creds := credentials.NewTLS(&tls.Config{
			MinVersion:     tls.VersionTLS12,
			GetCertificate: certWatcher.GetCertificate,
		})
		opts = append(opts, grpc.Creds(creds))
		logger.Info("TLS enabled for xDS servers with certificate watcher")
	} else {
		logger.Warn("TLS disabled for xDS servers: connections will be unencrypted")
	}

	return opts
}
