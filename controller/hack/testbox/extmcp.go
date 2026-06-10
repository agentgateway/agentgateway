package main

import (
	"context"
	"log"
	"net"
	"strings"

	"google.golang.org/grpc"
	"google.golang.org/protobuf/types/known/structpb"

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

func stringField(s *structpb.Struct, key string) (string, bool) {
	if s == nil {
		return "", false
	}
	v, ok := s.GetFields()[key]
	if !ok {
		return "", false
	}
	return v.GetStringValue(), v.GetStringValue() != ""
}

// mutateToolsListResult appends " [extmcp]" to every tool description in a
// tools/list response. Returns the mutated struct and true if a tools array
// was found.
func mutateToolsListResult(in *structpb.Struct) (*structpb.Struct, bool) {
	if in == nil {
		return nil, false
	}
	tools, ok := in.GetFields()["tools"]
	if !ok {
		return nil, false
	}
	list := tools.GetListValue()
	if list == nil {
		return nil, false
	}
	for _, item := range list.GetValues() {
		obj := item.GetStructValue()
		if obj == nil {
			continue
		}
		desc, _ := obj.GetFields()["description"]
		base := ""
		if desc != nil {
			base = desc.GetStringValue()
		}
		obj.Fields["description"] = structpb.NewStringValue(base + " [extmcp]")
	}
	return in, true
}
