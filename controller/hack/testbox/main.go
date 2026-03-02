package main

import (
	"context"
	"log"
	"os/signal"
	"syscall"
	"time"
)

type shutdownFunc func(context.Context) error

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	shutdowns := make([]shutdownFunc, 0, 7)

	start := func(name string, fn func() (shutdownFunc, error)) {
		shutdown, err := fn()
		if err != nil {
			log.Fatalf("failed to start %s: %v", name, err)
		}
		if shutdown != nil {
			shutdowns = append(shutdowns, shutdown)
		}
		log.Printf("started %s", name)
	}

	start("dummy-idp", startDummyIDP)
	start("extproc", startExtProcServer)
	start("ext-authz", startExtAuthzServer)
	start("mcp-website-fetcher", startMCPWebsiteServer)
	start("mcp-admin-server", startMCPAdminServer)
	start("test-a2a-server", startA2AServer)
	start("app", startEchoAppServer)

	<-ctx.Done()
	log.Printf("received shutdown signal")

	shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	for i := len(shutdowns) - 1; i >= 0; i-- {
		if err := shutdowns[i](shutdownCtx); err != nil {
			log.Printf("shutdown error: %v", err)
		}
	}

	log.Printf("testbox stopped")
}
