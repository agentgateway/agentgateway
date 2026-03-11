package main

import (
	"context"
	"log"
	"net"
	"strings"

	envoycorev3 "github.com/envoyproxy/go-control-plane/envoy/config/core/v3"
	authv3 "github.com/envoyproxy/go-control-plane/envoy/service/auth/v3"
	typev3 "github.com/envoyproxy/go-control-plane/envoy/type/v3"
	"google.golang.org/genproto/googleapis/rpc/status"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
)

const (
	extAuthzCheckHeader           = "x-ext-authz"
	extAuthzAllowedValue          = "allow"
	extAuthzResultHeader          = "x-ext-authz-check-result"
	extAuthzReceivedHeader        = "x-ext-authz-check-received"
	extAuthzOverrideHeader        = "x-ext-authz-additional-header-override"
	extAuthzOverrideGRPCValue     = "grpc-additional-header-override-value"
	extAuthzResultAllowed         = "allowed"
	extAuthzResultDenied          = "denied"
	extAuthzAllowedServiceAccount = "a"
)

var extAuthzDenyBody = "denied by ext_authz for not found header `x-ext-authz: allow` in the request"

type extAuthzServerV3 struct{}

func startExtAuthzServer() (shutdownFunc, error) {
	// nolint: gosec // Test code only
	listener, err := net.Listen("tcp", ":9000")
	if err != nil {
		return nil, err
	}

	grpcServer := grpc.NewServer()
	authv3.RegisterAuthorizationServer(grpcServer, &extAuthzServerV3{})

	return serveGRPC("ext-authz", listener, grpcServer), nil
}

func (s *extAuthzServerV3) Check(_ context.Context, request *authv3.CheckRequest) (*authv3.CheckResponse, error) {
	attrs := request.GetAttributes()
	allow := false

	checkHeaderValue, contains := attrs.GetRequest().GetHttp().GetHeaders()[extAuthzCheckHeader]
	if contains {
		allow = checkHeaderValue == extAuthzAllowedValue
	} else {
		allow = attrs.Source != nil && strings.HasSuffix(attrs.Source.Principal, "/sa/"+extAuthzAllowedServiceAccount)
	}

	if allow {
		return s.allow(request), nil
	}

	return s.deny(request), nil
}

func (s *extAuthzServerV3) allow(request *authv3.CheckRequest) *authv3.CheckResponse {
	s.logRequest("allowed", request)
	return &authv3.CheckResponse{
		HttpResponse: &authv3.CheckResponse_OkResponse{
			OkResponse: &authv3.OkHttpResponse{
				Headers: []*envoycorev3.HeaderValueOption{
					{
						Header: &envoycorev3.HeaderValue{Key: extAuthzResultHeader, Value: extAuthzResultAllowed},
					},
					{
						Header: &envoycorev3.HeaderValue{Key: extAuthzReceivedHeader, Value: returnIfNotTooLong(request.GetAttributes().String())},
					},
					{
						Header: &envoycorev3.HeaderValue{Key: extAuthzOverrideHeader, Value: extAuthzOverrideGRPCValue},
					},
				},
			},
		},
		Status: &status.Status{Code: int32(codes.OK)},
	}
}

func (s *extAuthzServerV3) deny(request *authv3.CheckRequest) *authv3.CheckResponse {
	s.logRequest("denied", request)
	return &authv3.CheckResponse{
		HttpResponse: &authv3.CheckResponse_DeniedResponse{
			DeniedResponse: &authv3.DeniedHttpResponse{
				Status: &typev3.HttpStatus{Code: typev3.StatusCode_Forbidden},
				Body:   extAuthzDenyBody,
				Headers: []*envoycorev3.HeaderValueOption{
					{
						Header: &envoycorev3.HeaderValue{Key: extAuthzResultHeader, Value: extAuthzResultDenied},
					},
					{
						Header: &envoycorev3.HeaderValue{Key: extAuthzReceivedHeader, Value: returnIfNotTooLong(request.GetAttributes().String())},
					},
					{
						Header: &envoycorev3.HeaderValue{Key: extAuthzOverrideHeader, Value: extAuthzOverrideGRPCValue},
					},
				},
			},
		},
		Status: &status.Status{Code: int32(codes.PermissionDenied)},
	}
}

func (s *extAuthzServerV3) logRequest(state string, request *authv3.CheckRequest) {
	httpAttrs := request.GetAttributes().GetRequest().GetHttp()
	log.Printf("[ext-authz][%s] %s%s", state, httpAttrs.GetHost(), httpAttrs.GetPath())
}

func returnIfNotTooLong(body string) string {
	if len(body) > 60000 {
		return "<too-long>"
	}
	return body
}
