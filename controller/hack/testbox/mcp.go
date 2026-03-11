package main

import (
	"context"
	"fmt"
	"net/http"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func startMCPWebsiteServer() (shutdownFunc, error) {
	return startMCPServer(":8000", "mcp-website-fetcher", newMCPWebsiteServer())
}

func startMCPAdminServer() (shutdownFunc, error) {
	return startMCPServer(":3001", "mcp-admin-server", newMCPAdminServer())
}

func startMCPServer(addr, name string, server *mcp.Server) (shutdownFunc, error) {
	mux := http.NewServeMux()
	mux.Handle("/mcp", mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server {
		return server
	}, nil))
	mux.Handle("/", mcp.NewSSEHandler(func(*http.Request) *mcp.Server {
		return server
	}, nil))

	// nolint: gosec // Test code only
	httpServer := &http.Server{
		Addr:    addr,
		Handler: mux,
	}

	return serveHTTP(name, httpServer, httpServer.ListenAndServe), nil
}

func newMCPWebsiteServer() *mcp.Server {
	s := mcp.NewServer(&mcp.Implementation{
		Name:    "mcp-website-fetcher",
		Version: "1.0.0",
	}, &mcp.ServerOptions{})

	mcp.AddTool(s, &mcp.Tool{
		Name:        "fetch",
		Description: "Fetch a URL and return extracted content.",
	}, fetchTool)

	return s
}

func newMCPAdminServer() *mcp.Server {
	s := mcp.NewServer(&mcp.Implementation{
		Name:    "mcp-admin-server",
		Version: "1.0.0",
	}, &mcp.ServerOptions{})

	mcp.AddTool(s, &mcp.Tool{
		Name:        "fetch",
		Description: "Fetch a URL and return extracted content.",
	}, fetchTool)

	mcp.AddTool(s, &mcp.Tool{
		Name:        "admin_status",
		Description: "Return admin diagnostics.",
	}, adminStatusTool)

	return s
}

type fetchArgs struct {
	URL string `json:"url" jsonschema:"The URL to fetch"`
}

func fetchTool(_ context.Context, _ *mcp.CallToolRequest, args fetchArgs) (*mcp.CallToolResult, any, error) {
	if args.URL == "" {
		return nil, nil, fmt.Errorf("missing url")
	}
	return &mcp.CallToolResult{
		Content: []mcp.Content{
			&mcp.TextContent{Text: "fetched: " + args.URL},
		},
	}, nil, nil
}

func adminStatusTool(_ context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, any, error) {
	return &mcp.CallToolResult{
		Content: []mcp.Content{
			&mcp.TextContent{Text: "ok"},
		},
	}, nil, nil
}
