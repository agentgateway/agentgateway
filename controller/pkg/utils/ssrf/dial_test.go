package ssrf

import (
	"net"
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestBlockCloudMetadata(t *testing.T) {
	tests := []struct {
		name    string
		ip      string
		blocked bool
	}{
		{name: "IPv4 link-local metadata", ip: "169.254.169.254", blocked: true},
		{name: "other IPv4 link-local", ip: "169.254.1.1", blocked: true},
		{name: "IPv6 link-local", ip: "fe80::1", blocked: true},
		{name: "Alibaba metadata", ip: "100.100.100.200", blocked: true},
		{name: "AWS IPv6 metadata", ip: "fd00:ec2::254", blocked: true},
		{name: "private IPv4", ip: "10.0.0.1"},
		{name: "loopback", ip: "127.0.0.1"},
		{name: "unrelated ULA", ip: "fd12:3456::1"},
		{name: "public IPv4", ip: "8.8.8.8"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := blockCloudMetadata(net.ParseIP(tt.ip))
			if tt.blocked {
				assert.ErrorContains(t, err, "connection blocked")
			} else {
				assert.NoError(t, err)
			}
		})
	}
}
