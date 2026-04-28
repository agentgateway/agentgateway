package remotehttp

import "github.com/agentgateway/agentgateway/controller/api/v1alpha1/agentgateway"

type TransportFingerprint struct {
	// Zero value means strict/default verification.
	Verification agentgateway.InsecureTLSMode `json:"verification,omitempty"`
	ServerName   string                       `json:"serverName,omitempty"`
	CABundleHash string                       `json:"caBundleHash,omitempty"`
	NextProtos   []string                     `json:"nextProtos,omitempty"`
}
