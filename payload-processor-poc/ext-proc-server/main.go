// POC: ExtProc server that reads the request body, extracts the "model" field,
// and sets it as the X-Gateway-Model-Name header on the request.
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"io"
	"log"
	"net"

	corev3 "github.com/envoyproxy/go-control-plane/envoy/config/core/v3"
	extprocv3 "github.com/envoyproxy/go-control-plane/envoy/service/ext_proc/v3"
	"google.golang.org/grpc"
)

var port = flag.Int("port", 18080, "gRPC server port")

type server struct {
	extprocv3.UnimplementedExternalProcessorServer
}

func (s *server) Process(stream extprocv3.ExternalProcessor_ProcessServer) error {
	log.Println("new ext_proc stream started")
	var bodyBuf []byte

	for {
		req, err := stream.Recv()
		if err == io.EOF {
			return nil
		}
		if err != nil {
			log.Printf("recv error: %v", err)
			return err
		}

		switch r := req.Request.(type) {
		case *extprocv3.ProcessingRequest_RequestHeaders:
			log.Printf("received request headers (eos=%v)", r.RequestHeaders.EndOfStream)
			// POC: Only respond to headers when there is no body (end_of_stream).
			// When a body is expected, we defer the response until we've read the
			// full body and can set headers based on its content.
			if r.RequestHeaders.EndOfStream {
				if err := stream.Send(&extprocv3.ProcessingResponse{
					Response: &extprocv3.ProcessingResponse_RequestHeaders{
						RequestHeaders: &extprocv3.HeadersResponse{},
					},
				}); err != nil {
					return err
				}
			}

		case *extprocv3.ProcessingRequest_RequestBody:
			bodyBuf = append(bodyBuf, r.RequestBody.Body...)
			log.Printf("received body chunk (eos=%v, total=%d)", r.RequestBody.EndOfStream, len(bodyBuf))

			// POC: Do not respond to intermediate body chunks. The data plane
			// streams chunks without waiting for per-chunk acknowledgements.
			// Only respond once we have the full body (end_of_stream).
			if r.RequestBody.EndOfStream {
				model := extractModel(bodyBuf)
				log.Printf("extracted model: %q", model)

				// POC: Send a RequestHeaders response with the header mutation.
				// This is the pattern used by GIE's body-based router: header
				// mutations are delivered via a RequestHeaders response even
				// during body processing, so the data plane applies them to
				// the request before route selection.
				var headerMutation *extprocv3.HeaderMutation
				if model != "" {
					headerMutation = &extprocv3.HeaderMutation{
						SetHeaders: []*corev3.HeaderValueOption{
							{
								Header: &corev3.HeaderValue{
									Key:      "X-Gateway-Model-Name",
									RawValue: []byte(model),
								},
							},
						},
					}
				}
				if err := stream.Send(&extprocv3.ProcessingResponse{
					Response: &extprocv3.ProcessingResponse_RequestHeaders{
						RequestHeaders: &extprocv3.HeadersResponse{
							Response: &extprocv3.CommonResponse{
								HeaderMutation: headerMutation,
							},
						},
					},
				}); err != nil {
					return err
				}

				// POC: Then send a RequestBody response echoing the buffered body
				// via StreamedResponse so the upstream backend receives it.
				if err := stream.Send(&extprocv3.ProcessingResponse{
					Response: &extprocv3.ProcessingResponse_RequestBody{
						RequestBody: &extprocv3.BodyResponse{
							Response: &extprocv3.CommonResponse{
								BodyMutation: &extprocv3.BodyMutation{
									Mutation: &extprocv3.BodyMutation_StreamedResponse{
										StreamedResponse: &extprocv3.StreamedBodyResponse{
											Body:        bodyBuf,
											EndOfStream: true,
										},
									},
								},
							},
						},
					},
				}); err != nil {
					return err
				}
				bodyBuf = nil
			}

		case *extprocv3.ProcessingRequest_ResponseHeaders:
			if err := stream.Send(&extprocv3.ProcessingResponse{
				Response: &extprocv3.ProcessingResponse_ResponseHeaders{
					ResponseHeaders: &extprocv3.HeadersResponse{},
				},
			}); err != nil {
				return err
			}

		case *extprocv3.ProcessingRequest_ResponseBody:
			// POC: Echo the response body back via StreamedResponse so the
			// client receives it.
			if err := stream.Send(&extprocv3.ProcessingResponse{
				Response: &extprocv3.ProcessingResponse_ResponseBody{
					ResponseBody: &extprocv3.BodyResponse{
						Response: &extprocv3.CommonResponse{
							BodyMutation: &extprocv3.BodyMutation{
								Mutation: &extprocv3.BodyMutation_StreamedResponse{
									StreamedResponse: &extprocv3.StreamedBodyResponse{
										Body:        r.ResponseBody.Body,
										EndOfStream: r.ResponseBody.EndOfStream,
									},
								},
							},
						},
					},
				},
			}); err != nil {
				return err
			}

		default:
			log.Printf("unknown request type: %T", r)
		}
	}
}

func extractModel(body []byte) string {
	var parsed map[string]interface{}
	if err := json.Unmarshal(body, &parsed); err != nil {
		log.Printf("failed to parse JSON: %v", err)
		return ""
	}
	if model, ok := parsed["model"].(string); ok {
		return model
	}
	return ""
}

func main() {
	flag.Parse()
	lis, err := net.Listen("tcp", fmt.Sprintf(":%d", *port))
	if err != nil {
		log.Fatalf("failed to listen: %v", err)
	}
	grpcServer := grpc.NewServer()
	extprocv3.RegisterExternalProcessorServer(grpcServer, &server{})
	log.Printf("ext-proc BBR server listening on :%d", *port)
	if err := grpcServer.Serve(lis); err != nil {
		log.Fatalf("failed to serve: %v", err)
	}
}
