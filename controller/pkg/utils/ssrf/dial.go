package ssrf

import (
	"context"
	"fmt"
	"net"
	"syscall"
)

var knownCloudMetadataRanges = mustParseCIDRs(
	"100.100.100.200/32", // Alibaba Cloud
	"fd00:ec2::/32",      // AWS IMDS IPv6
)

// SafeDialContext returns a DialContext that blocks cloud metadata addresses after DNS
// resolution and before connecting. Private and loopback addresses remain allowed for
// legitimate in-cluster services.
func SafeDialContext(dialer *net.Dialer) func(context.Context, string, string) (net.Conn, error) {
	d := *dialer
	d.Control = func(_, address string, _ syscall.RawConn) error {
		host, _, err := net.SplitHostPort(address)
		if err != nil {
			return err
		}
		ip := net.ParseIP(host)
		if ip == nil {
			return fmt.Errorf("unable to parse resolved IP address %q", address)
		}
		return blockCloudMetadata(ip)
	}
	return d.DialContext
}

func blockCloudMetadata(ip net.IP) error {
	if ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() {
		return fmt.Errorf("connection blocked: resolved IP %s is link-local", ip.String())
	}
	for _, network := range knownCloudMetadataRanges {
		if network.Contains(ip) {
			return fmt.Errorf("connection blocked: resolved IP %s is a known cloud metadata address", ip.String())
		}
	}
	return nil
}

func mustParseCIDRs(cidrs ...string) []*net.IPNet {
	networks := make([]*net.IPNet, 0, len(cidrs))
	for _, cidr := range cidrs {
		_, network, err := net.ParseCIDR(cidr)
		if err != nil {
			panic(err)
		}
		networks = append(networks, network)
	}
	return networks
}
