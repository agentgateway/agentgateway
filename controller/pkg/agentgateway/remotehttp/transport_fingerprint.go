package remotehttp

type VerificationMode string

const (
	VerificationModeStrict   VerificationMode = "strict"
	VerificationModeHostname VerificationMode = "hostname"
	VerificationModeInsecure VerificationMode = "insecure"
)

type TransportFingerprint struct {
	Verification VerificationMode `json:"verification,omitempty"`
	ServerName   string           `json:"serverName,omitempty"`
	CABundleHash string           `json:"caBundleHash,omitempty"`
	NextProtos   []string         `json:"nextProtos,omitempty"`
}
