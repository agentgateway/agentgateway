package main

import (
	"context"
	"errors"
	"log"
	"net"
	"net/http"

	"google.golang.org/grpc"
)

func serveHTTP(name string, srv *http.Server, listen func() error) shutdownFunc {
	go func() {
		if err := listen(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			log.Fatalf("%s exited: %v", name, err)
		}
	}()

	return func(ctx context.Context) error {
		return srv.Shutdown(ctx)
	}
}

func serveGRPC(name string, listener net.Listener, server *grpc.Server) shutdownFunc {
	go func() {
		if err := server.Serve(listener); err != nil && !errors.Is(err, grpc.ErrServerStopped) {
			log.Fatalf("%s exited: %v", name, err)
		}
	}()

	return func(ctx context.Context) error {
		done := make(chan struct{})
		go func() {
			server.GracefulStop()
			close(done)
		}()

		select {
		case <-done:
			return nil
		case <-ctx.Done():
			server.Stop()
			return ctx.Err()
		}
	}
}
