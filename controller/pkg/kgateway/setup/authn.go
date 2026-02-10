package setup

import (
	"context"
	"fmt"
	"log/slog"
	"net/http"
	"strings"

	"istio.io/istio/pkg/security"
	"istio.io/istio/security/pkg/k8s/tokenreview"
	"k8s.io/client-go/kubernetes"
)

const (
	KubeJWTAuthenticatorType = "KubeJWTAuthenticator"

	authorizationHeader = "authorization"

	bearerTokenPrefix = "Bearer "
)

var xdsTokenAudiences = []string{"kgateway"}

// KubeJWTAuthenticator authenticates K8s JWTs.
type KubeJWTAuthenticator struct {
	kubeClient kubernetes.Interface
}

var _ security.Authenticator = &KubeJWTAuthenticator{}

// NewKubeJWTAuthenticator creates a new kubeJWTAuthenticator.
func NewKubeJWTAuthenticator(
	client kubernetes.Interface,
) *KubeJWTAuthenticator {
	out := &KubeJWTAuthenticator{
		kubeClient: client,
	}

	return out
}

func (a *KubeJWTAuthenticator) AuthenticatorType() string {
	return KubeJWTAuthenticatorType
}

// Authenticate authenticates the call using the K8s JWT from the request's context
func (a *KubeJWTAuthenticator) Authenticate(authRequest security.AuthContext) (*security.Caller, error) {
	if authRequest.GrpcContext != nil {
		return a.authenticateGrpc(authRequest.GrpcContext)
	}
	if authRequest.Request != nil {
		return a.authenticateHTTP(authRequest.Request)
	}
	return nil, nil
}

func (a *KubeJWTAuthenticator) authenticateHTTP(req *http.Request) (*security.Caller, error) {
	targetJWT, err := extractRequestToken(req)
	if err != nil {
		return nil, fmt.Errorf("target JWT extraction error: %v", err)
	}
	return a.authenticate(targetJWT)
}

func (a *KubeJWTAuthenticator) authenticateGrpc(ctx context.Context) (*security.Caller, error) {
	targetJWT, err := security.ExtractBearerToken(ctx)
	if err != nil {
		return nil, fmt.Errorf("target JWT extraction error: %v", err)
	}

	return a.authenticate(targetJWT)
}

func (a *KubeJWTAuthenticator) authenticate(targetJWT string) (*security.Caller, error) {
	id, err := tokenreview.ValidateK8sJwt(a.kubeClient, targetJWT, xdsTokenAudiences)
	if err != nil {
		return nil, fmt.Errorf("failed to validate the JWT token: %v", err)
	}
	if id.PodServiceAccount == "" {
		return nil, fmt.Errorf("failed to parse the JWT; service account required")
	}
	if id.PodNamespace == "" {
		return nil, fmt.Errorf("failed to parse the JWT; namespace required")
	}
	return &security.Caller{
		AuthSource:     security.AuthSourceIDToken,
		KubernetesInfo: id,
		// We do not set any SPIFFE identities
		Identities: nil,
	}, nil
}

func extractRequestToken(req *http.Request) (string, error) {
	value := req.Header.Get(authorizationHeader)
	if value == "" {
		return "", fmt.Errorf("no HTTP authorization header exists")
	}

	if after, ok := strings.CutPrefix(value, bearerTokenPrefix); ok {
		return after, nil
	}

	return "", fmt.Errorf("no bearer token exists in HTTP authorization header")
}

// authenticationManager orchestrates all authenticators to perform authentication.
type authenticationManager struct {
	Authenticators []security.Authenticator
	// authFailMsgs contains list of messages that authenticator wants to record - mainly used for logging.
	authFailMsgs []string
}

// Authenticate loops through all the configured Authenticators and returns if one of the authenticator succeeds.
func (am *authenticationManager) authenticate(ctx context.Context) *security.Caller {
	req := security.AuthContext{GrpcContext: ctx}
	for _, authn := range am.Authenticators {
		u, err := authn.Authenticate(req)
		if u != nil && err == nil { // we don't validate len(u.Identities) here like Istio does since this isn't relevant
			slog.Debug("authentication succeeded", "auth_source", u.AuthSource)
			return u
		}
		am.authFailMsgs = append(am.authFailMsgs, fmt.Sprintf("Authenticator %s: %v", authn.AuthenticatorType(), err))
	}
	return nil
}
