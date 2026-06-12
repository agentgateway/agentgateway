package main

import (
	"bytes"
	"context"
	"encoding/json"
	"log"
	"net"
	"strings"

	"google.golang.org/grpc"

	"github.com/agentgateway/agentgateway/api"
)

// Policy: any request whose tool name (params.name for tools/call) contains
// "forbidden" is rejected. tools/list responses are mutated to mark each tool
// with a description suffix so tests can observe response-phase mutation.
type extMcpServer struct {
	api.UnimplementedExtMcpServer
}

const extMcpListenAddr = ":9001"

func startExtMcpServer() (shutdownFunc, error) {
	// nolint: gosec // Test code only
	listener, err := net.Listen("tcp", extMcpListenAddr)
	if err != nil {
		return nil, err
	}

	grpcServer := grpc.NewServer()
	api.RegisterExtMcpServer(grpcServer, &extMcpServer{})

	return serveGRPC("ext-mcp", listener, grpcServer), nil
}

func (s *extMcpServer) CheckRequest(_ context.Context, req *api.McpRequest) (*api.McpRequestResult, error) {
	log.Printf("[ext-mcp][request] method=%q services=%q", req.GetMethod(), req.GetServiceNames())

	if req.GetMethod() == "tools/call" {
		if name, ok := stringField(req.GetMcpRequest(), "name"); ok && strings.Contains(name, "forbidden") {
			return &api.McpRequestResult{
				Result: &api.McpRequestResult_Error{
					Error: &api.AuthorizationError{
						Code:   api.AuthorizationError_PERMISSION_DENIED,
						Reason: "tool " + name + " is not allowed",
					},
				},
			}, nil
		}
	}

	return &api.McpRequestResult{Result: &api.McpRequestResult_Pass{Pass: &api.Pass{}}}, nil
}

func (s *extMcpServer) CheckResponse(_ context.Context, resp *api.McpResponse) (*api.McpResponseResult, error) {
	log.Printf("[ext-mcp][response] method=%q services=%q", resp.GetMethod(), resp.GetServiceNames())

	if resp.GetMethod() != "tools/list" {
		return &api.McpResponseResult{Result: &api.McpResponseResult_Pass{Pass: &api.Pass{}}}, nil
	}

	mutated, ok := mutateToolsListResult(resp.GetMcpResponse())
	if !ok {
		return &api.McpResponseResult{Result: &api.McpResponseResult_Pass{Pass: &api.Pass{}}}, nil
	}
	return &api.McpResponseResult{Result: &api.McpResponseResult_Mutated{Mutated: mutated}}, nil
}

// unmarshalObject decodes raw JSON into a map, keeping numbers as json.Number
// so re-marshaling doesn't turn integers into floats.
func unmarshalObject(raw []byte) (map[string]any, bool) {
	if len(raw) == 0 {
		return nil, false
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.UseNumber()
	var m map[string]any
	if err := dec.Decode(&m); err != nil {
		return nil, false
	}
	return m, true
}

func stringField(raw []byte, key string) (string, bool) {
	m, ok := unmarshalObject(raw)
	if !ok {
		return "", false
	}
	s, ok := m[key].(string)
	return s, ok && s != ""
}

// mutateToolsListResult appends " [extmcp]" to every tool description in a
// tools/list response. Returns the mutated JSON and true if a tools array
// was found.
func mutateToolsListResult(raw []byte) ([]byte, bool) {
	m, ok := unmarshalObject(raw)
	if !ok {
		return nil, false
	}
	tools, ok := m["tools"].([]any)
	if !ok {
		return nil, false
	}
	for _, item := range tools {
		obj, ok := item.(map[string]any)
		if !ok {
			continue
		}
		base, _ := obj["description"].(string)
		obj["description"] = base + " [extmcp]"
	}
	out, err := json.Marshal(m)
	if err != nil {
		return nil, false
	}
	return out, true
}
