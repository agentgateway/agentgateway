package wellknown

// AWS constants for lambda and bedrock configuration
const (
	// AccessKey is the key name for in the secret data for the access key id.
	AccessKey = "accessKey"
	// SessionToken is the key name for in the secret data for the session token.
	SessionToken = "sessionToken"
	// SecretKey is the key name for in the secret data for the secret access key.
	SecretKey = "secretKey"
	// ClientID is the key name for in the secret data for the client id.
	ClientID = "clientID"
	// TenantID is the key name for in the secret data for the tenant id.
	TenantID = "tenantID"
	// ClientSecret is the key name for in the secret data for the client secret.
	ClientSecret = "clientSecret"
	// GCPCredentialsJSON is the key name for GCP ADC-compatible credential JSON.
	GCPCredentialsJSON = "credentials.json"
)
