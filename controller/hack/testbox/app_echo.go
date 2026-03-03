package main

import (
	"context"
	"log"
	"os"

	"istio.io/istio/pkg/config/protocol"
	"istio.io/istio/pkg/test/echo/common"
	echoserver "istio.io/istio/pkg/test/echo/server"
)

func startEchoAppServer() (shutdownFunc, error) {
	ports := common.PortList{
		&common.Port{Name: "http-0", Protocol: protocol.HTTP, Port: 80},
		&common.Port{Name: "tcp-0", Protocol: protocol.TCP, Port: 9090},
		&common.Port{Name: "grpc-0", Protocol: protocol.GRPC, Port: 7070},
	}
	cfg := echoserver.Config{
		Ports:     ports,
		Namespace: os.Getenv("NAMESPACE"),
		ReportRequest: func() {
		},
	}

	if fileExists("/tls/tls.crt") && fileExists("/tls/tls.key") {
		cfg.TLSCert = "/tls/tls.crt"
		cfg.TLSKey = "/tls/tls.key"
		cfg.Ports = append(cfg.Ports, &common.Port{Name: "https", Protocol: protocol.HTTP, Port: 443, TLS: true})
	} else {
		log.Println("app TLS certs are not mounted; skipping echo TLS")
	}

	s := echoserver.New(cfg)

	if err := s.Start(); err != nil {
		return nil, err
	}

	return func(_ context.Context) error {
		return s.Close()
	}, nil
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
