package main

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"strings"
)

const rawHeadersPort = 18081

func startRawHeadersServer() (shutdownFunc, error) {
	listener, err := net.Listen("tcp", fmt.Sprintf(":%d", rawHeadersPort))
	if err != nil {
		return nil, err
	}

	done := make(chan struct{})
	go func() {
		defer close(done)
		for {
			conn, err := listener.Accept()
			if err != nil {
				if errors.Is(err, net.ErrClosed) {
					return
				}
				log.Printf("raw headers accept failed: %v", err)
				continue
			}
			go handleRawHeadersConn(conn)
		}
	}()

	return func(ctx context.Context) error {
		err := listener.Close()
		select {
		case <-done:
		case <-ctx.Done():
			return ctx.Err()
		}
		return err
	}, nil
}

func handleRawHeadersConn(conn net.Conn) {
	defer conn.Close()

	reader := bufio.NewReader(conn)
	var lines []string
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			if !errors.Is(err, io.EOF) {
				log.Printf("raw headers read failed: %v", err)
			}
			return
		}
		line = strings.TrimRight(line, "\r\n")
		if line == "" {
			break
		}
		lines = append(lines, line)
	}

	body := strings.Join(lines, "\n") + "\n"
	_, _ = fmt.Fprintf(conn, "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: %d\r\nConnection: close\r\n\r\n%s", len(body), body)
}
