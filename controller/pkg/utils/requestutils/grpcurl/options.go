package grpcurl

import (
	"fmt"
)

// Option is a functional option for configuring a grpcurl command.
// It allows for a flexible way to set various command-line arguments for grpcurl.
type Option func(*Command)

// Command represents a grpcurl command with its various flags and arguments.
type Command struct {
	Address        string   // Target server address (e.g., "host:port")
	Port           int      // Target server port (e.g., 8080)
	Authority      string   // Authority header value (e.g., "example.com")
	Symbol         string   // Service name, method name, or symbol to describe/list (e.g., "package.Service/Method" or "package.Service")
	Data           string   // JSON string for the request body.
	Plaintext      bool     // Use plaintext connection (disables TLS).
	Headers        []string // List of headers in "Name: Value" format.
	Verbose        bool     // Enable verbose output (-v).
	ConnectTimeout int      // Connection timeout in seconds.
}

// NewCommand creates a new Command with default settings and applies the given options.
func NewCommand(opts ...Option) *Command {
	cmd := &Command{}
	for _, opt := range opts {
		opt(cmd)
	}
	return cmd
}

// ToArgs converts the Command struct into a slice of string arguments suitable for `exec`.
func (c *Command) ToArgs() []string {
	args := []string{}

	if c.Plaintext {
		args = append(args, "-plaintext")
	}

	if c.Data != "" {
		args = append(args, "-d", c.Data)
	}
	for _, h := range c.Headers {
		args = append(args, "-H", h)
	}
	if c.Verbose {
		args = append(args, "-v")
	}
	if c.ConnectTimeout > 0 {
		args = append(args, "-connect-timeout", fmt.Sprintf("%d", c.ConnectTimeout))
	}

	if c.Authority != "" {
		args = append(args, "-authority", c.Authority)
	}

	if c.Address != "" {
		if c.Port != 0 {
			args = append(args, fmt.Sprintf("%s:%d", c.Address, c.Port))
		} else {
			args = append(args, c.Address)
		}
	}

	if c.Symbol != "" {
		args = append(args, c.Symbol)
	}

	return args
}

// Option functions to modify the Command struct.

func WithAddress(address string) Option     { return func(c *Command) { c.Address = address } }
func WithPort(port int) Option              { return func(c *Command) { c.Port = port } }
func WithAuthority(authority string) Option { return func(c *Command) { c.Authority = authority } }
func WithSymbol(symbol string) Option       { return func(c *Command) { c.Symbol = symbol } }
func WithPlaintext() Option                 { return func(c *Command) { c.Plaintext = true } }

func WithConnectTimeout(sec int) Option { return func(c *Command) { c.ConnectTimeout = sec } }
func WithVerbose() Option               { return func(c *Command) { c.Verbose = true } }
