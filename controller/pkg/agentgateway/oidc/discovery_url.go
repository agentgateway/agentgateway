package oidc

import (
	"fmt"
	"net/url"
	"strings"
)

func OidcDiscoveryURL(issuerURL string) (string, error) {
	u, err := url.Parse(issuerURL)
	if err != nil {
		return "", fmt.Errorf("invalid issuer URL %q: %w", issuerURL, err)
	}
	if !u.IsAbs() || u.Host == "" || u.RawQuery != "" || u.Fragment != "" {
		return "", fmt.Errorf("issuer URL must be absolute with a host and no query or fragment")
	}
	base := *u
	base.Path = strings.TrimRight(u.Path, "/") + "/.well-known/openid-configuration"
	return base.String(), nil
}
