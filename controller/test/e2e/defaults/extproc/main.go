// based on https://github.com/salrashid123/envoy_ext_proc/blob/eca3b3a89929bf8cb80879ba553798ecea1c5622/grpc_server.go

package main

import (
	"context"
	"encoding/json"
	"flag"
	"io"
	"log/slog"
	"net"
	"os"
	"os/signal"
	"strconv"
	"syscall"
	"time"

	core_v3 "github.com/envoyproxy/go-control-plane/envoy/config/core/v3"
	service_ext_proc_v3 "github.com/envoyproxy/go-control-plane/envoy/service/ext_proc/v3"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/health/grpc_health_v1"
	"google.golang.org/grpc/status"
)

var (
	grpcport = flag.String("grpcport", ":18080", "grpcport")
)

type Instructions struct {
	// Header key/value pairs to add to the request or response.
	AddHeaders map[string]string `json:"addHeaders"`
	// Header keys to remove from the request or response.
	RemoveHeaders []string `json:"removeHeaders"`
	// Set the body of the request or response to the specified string. If empty, will be ignored.
	SetBody string `json:"setBody"`
	// Set the request or response trailers.
	SetTrailers map[string]string `json:"setTrailers"`
}

type server struct{}

type healthServer struct{}

func (s *healthServer) Check(ctx context.Context, in *grpc_health_v1.HealthCheckRequest) (*grpc_health_v1.HealthCheckResponse, error) {
	slog.Info("handling grpc Check request", "request", in.String())
	return &grpc_health_v1.HealthCheckResponse{Status: grpc_health_v1.HealthCheckResponse_SERVING}, nil
}

func (s *healthServer) Watch(in *grpc_health_v1.HealthCheckRequest, srv grpc_health_v1.Health_WatchServer) error {
	return status.Error(codes.Unimplemented, "Watch is not implemented")
}

func (s *healthServer) List(ctx context.Context, in *grpc_health_v1.HealthListRequest) (*grpc_health_v1.HealthListResponse, error) {
	return &grpc_health_v1.HealthListResponse{
		Statuses: map[string]*grpc_health_v1.HealthCheckResponse{
			"": {Status: grpc_health_v1.HealthCheckResponse_SERVING},
		},
	}, nil
}

func (s *server) Process(srv service_ext_proc_v3.ExternalProcessor_ProcessServer) error {
	slog.Info("process")
	ctx := srv.Context()
	for {
		select {
		case <-ctx.Done():
			slog.Info("context done")
			return ctx.Err()
		default:
		}

		req, err := srv.Recv()
		if err == io.EOF {
			// envoy has closed the stream. Don't return anything and close this stream entirely
			return nil
		}
		if err != nil {
			return status.Errorf(codes.Unknown, "cannot receive stream request: %v", err)
		}

		// build response based on request type
		resp := &service_ext_proc_v3.ProcessingResponse{}
		switch v := req.Request.(type) {
		case *service_ext_proc_v3.ProcessingRequest_RequestHeaders:
			slog.Info("got RequestHeaders")

			h := req.Request.(*service_ext_proc_v3.ProcessingRequest_RequestHeaders)
			headersResp, err := getHeadersResponseFromInstructions(h.RequestHeaders)
			if err != nil {
				return err
			}
			resp = &service_ext_proc_v3.ProcessingResponse{
				Response: &service_ext_proc_v3.ProcessingResponse_RequestHeaders{
					RequestHeaders: headersResp,
				},
			}

		case *service_ext_proc_v3.ProcessingRequest_RequestBody:
			slog.Info("got RequestBody - forwarding")

			h := req.Request.(*service_ext_proc_v3.ProcessingRequest_RequestBody)

			resp = &service_ext_proc_v3.ProcessingResponse{
				Response: &service_ext_proc_v3.ProcessingResponse_RequestBody{
					RequestBody: &service_ext_proc_v3.BodyResponse{
						Response: &service_ext_proc_v3.CommonResponse{
							BodyMutation: &service_ext_proc_v3.BodyMutation{
								Mutation: &service_ext_proc_v3.BodyMutation_StreamedResponse{
									StreamedResponse: &service_ext_proc_v3.StreamedBodyResponse{
										Body:        h.RequestBody.Body,
										EndOfStream: h.RequestBody.EndOfStream,
									},
								},
							},
						},
					},
				},
			}
		case *service_ext_proc_v3.ProcessingRequest_RequestTrailers:
			slog.Info("got RequestTrailers (not currently handled)")
			resp.Response = &service_ext_proc_v3.ProcessingResponse_RequestTrailers{}

		case *service_ext_proc_v3.ProcessingRequest_ResponseHeaders:
			slog.Info("got ResponseHeaders")

			h := req.Request.(*service_ext_proc_v3.ProcessingRequest_ResponseHeaders)
			headersResp, err := getHeadersResponseFromInstructions(h.ResponseHeaders)
			if err != nil {
				return err
			}
			resp = &service_ext_proc_v3.ProcessingResponse{
				Response: &service_ext_proc_v3.ProcessingResponse_ResponseHeaders{
					ResponseHeaders: headersResp,
				},
			}

		case *service_ext_proc_v3.ProcessingRequest_ResponseBody:
			slog.Info("got ResponseBody - forwarding")

			h := req.Request.(*service_ext_proc_v3.ProcessingRequest_ResponseBody)

			resp = &service_ext_proc_v3.ProcessingResponse{
				Response: &service_ext_proc_v3.ProcessingResponse_ResponseBody{
					ResponseBody: &service_ext_proc_v3.BodyResponse{
						Response: &service_ext_proc_v3.CommonResponse{
							BodyMutation: &service_ext_proc_v3.BodyMutation{
								Mutation: &service_ext_proc_v3.BodyMutation_StreamedResponse{
									StreamedResponse: &service_ext_proc_v3.StreamedBodyResponse{
										Body:        h.ResponseBody.Body,
										EndOfStream: h.ResponseBody.EndOfStream,
									},
								},
							},
						},
					},
				},
			}

		case *service_ext_proc_v3.ProcessingRequest_ResponseTrailers:
			slog.Info("got ResponseTrailers (not currently handled)")
			resp.Response = &service_ext_proc_v3.ProcessingResponse_ResponseTrailers{}

		default:
			slog.Info("unknown request type", "request type", v)
		}

		// At this point we believe we have created a valid response...
		// note that this is sometimes not the case
		// anyways for now just send it
		slog.Info("sending ProcessingResponse")
		if err := srv.Send(resp); err != nil {
			slog.Info("send error", "error", err)
			return err
		}

	}
}

func main() {

	flag.Parse()

	lis, err := net.Listen("tcp", *grpcport)
	if err != nil {
		slog.Error("failed to listen", "error", err)
		os.Exit(1)
	}

	sopts := []grpc.ServerOption{grpc.MaxConcurrentStreams(1000)}
	s := grpc.NewServer(sopts...)

	service_ext_proc_v3.RegisterExternalProcessorServer(s, &server{})

	grpc_health_v1.RegisterHealthServer(s, &healthServer{})

	slog.Info("starting gRPC server", "port", *grpcport)

	var gracefulStop = make(chan os.Signal, 1)
	signal.Notify(gracefulStop, syscall.SIGTERM, syscall.SIGINT)
	go func() {
		sig := <-gracefulStop
		slog.Info("caught sig", "sig", sig)
		time.Sleep(time.Second)
		slog.Info("graceful stop completed")
		os.Exit(0)
	}()
	err = s.Serve(lis)
	if err != nil {
		slog.Error("killing server", "error", err)
		os.Exit(1)
	}
}

func getInstructionsFromHeaders(in *service_ext_proc_v3.HttpHeaders) string {
	for _, n := range in.Headers.Headers {
		if n.Key == "instructions" {
			return string(n.RawValue)
		}
	}
	return ""
}

func getHeadersResponseFromInstructions(in *service_ext_proc_v3.HttpHeaders) (*service_ext_proc_v3.HeadersResponse, error) {
	instructionString := getInstructionsFromHeaders(in)

	// no instructions were sent, so don't modify anything
	if instructionString == "" {
		return &service_ext_proc_v3.HeadersResponse{}, nil
	}

	var instructions *Instructions
	err := json.Unmarshal([]byte(instructionString), &instructions)
	if err != nil {
		slog.Info("error unmarshalling instructions", "error", err)
		return nil, err
	}

	// build the response
	resp := &service_ext_proc_v3.HeadersResponse{
		Response: &service_ext_proc_v3.CommonResponse{},
	}

	// headers
	if len(instructions.AddHeaders) > 0 || len(instructions.RemoveHeaders) > 0 {
		var addHeaders []*core_v3.HeaderValueOption
		for k, v := range instructions.AddHeaders {
			addHeaders = append(addHeaders, &core_v3.HeaderValueOption{
				Header: &core_v3.HeaderValue{Key: k, RawValue: []byte(v)},
			})
		}
		resp.Response.HeaderMutation = &service_ext_proc_v3.HeaderMutation{
			SetHeaders:    addHeaders,
			RemoveHeaders: instructions.RemoveHeaders,
		}
	}

	// body
	if instructions.SetBody != "" {
		body := []byte(instructions.SetBody)

		if resp.Response.HeaderMutation == nil {
			resp.Response.HeaderMutation = &service_ext_proc_v3.HeaderMutation{}
		}
		resp.Response.HeaderMutation.SetHeaders = append(resp.Response.HeaderMutation.SetHeaders,
			[]*core_v3.HeaderValueOption{
				{
					Header: &core_v3.HeaderValue{
						Key:   "content-type",
						Value: "text/plain",
					},
				},
				{
					Header: &core_v3.HeaderValue{
						Key:   "Content-Length",
						Value: strconv.Itoa(len(body)),
					},
				},
			}...)
		resp.Response.BodyMutation = &service_ext_proc_v3.BodyMutation{
			Mutation: &service_ext_proc_v3.BodyMutation_Body{
				Body: body,
			},
		}
	}

	// trailers
	if len(instructions.SetTrailers) > 0 {
		var setTrailers []*core_v3.HeaderValue
		for k, v := range instructions.SetTrailers {
			setTrailers = append(setTrailers, &core_v3.HeaderValue{Key: k, Value: v})
		}
		resp.Response.Trailers = &core_v3.HeaderMap{
			Headers: setTrailers,
		}
	}

	return resp, nil
}
