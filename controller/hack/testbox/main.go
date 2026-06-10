package main

import (
	"context"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"
)

type shutdownFunc func(context.Context) error

func main() {
	if len(os.Args) > 1 && os.Args[1] == "fetch" {
		os.Exit(runFetchCommand(os.Args[2:]))
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	shutdowns := make([]shutdownFunc, 0, 8)

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
	start("llm", startLLMServer)
	start("app", startEchoAppServer)
	start("raw-headers", startRawHeadersServer)

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

func runFetchCommand(args []string) int {
	if len(args) != 1 {
		fmt.Fprintln(os.Stderr, "usage: testbox fetch <url>")
		return 2
	}

	client := &http.Client{Timeout: 5 * time.Second}
	resp, err := client.Get(args[0])
	if err != nil {
		fmt.Fprintf(os.Stderr, "fetch failed: %v\n", err)
		return 1
	}
	defer resp.Body.Close()

	fmt.Fprintf(os.Stdout, "%s\n", resp.Status)
	for key, values := range resp.Header {
		for _, value := range values {
			fmt.Fprintf(os.Stdout, "%s: %s\n", key, value)
		}
	}
	fmt.Fprintln(os.Stdout)

	if _, err := io.Copy(os.Stdout, resp.Body); err != nil {
		fmt.Fprintf(os.Stderr, "failed to read response body: %v\n", err)
		return 1
	}
	return 0
}
