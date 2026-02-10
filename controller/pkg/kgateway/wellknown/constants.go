package wellknown

import (
	"k8s.io/apimachinery/pkg/types"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/namespaces"
)

const (
	// OAuth2HMACSecretKey is the key within the OAuth2HMACSecret that holds the HMAC secret key for OAuth2
	OAuth2HMACSecretKey = "hmac-secret"
)

// AWS constants for lambda and bedrock configuration
const (
	// AccessKey is the key name for in the secret data for the access key id.
	AccessKey = "accessKey"
	// SessionToken is the key name for in the secret data for the session token.
	SessionToken = "sessionToken"
	// SecretKey is the key name for in the secret data for the secret access key.
	SecretKey = "secretKey"
)

// OAuth2HMACSecret is the secret that holds the HMAC key for OAuth2
var OAuth2HMACSecret = types.NamespacedName{Name: "oauth2-hmac-secret", Namespace: namespaces.GetPodNamespace()}
