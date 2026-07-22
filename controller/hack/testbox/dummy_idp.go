package main

import (
	"crypto/tls"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"slices"
	"strings"
	"time"

	_ "embed"

	"github.com/agentgateway/agentgateway/controller/test/testutils/testjwt"
)

//go:embed dummy-idp.cert
var cert []byte

//go:embed dummy-idp.key
var key []byte

func startDummyIDP() (shutdownFunc, error) {
	roots := x509.NewCertPool()
	if !roots.AppendCertsFromPEM(cert) {
		return nil, fmt.Errorf("failed to append Cert from PEM")
	}

	cert, err := tls.X509KeyPair(cert, key)
	if err != nil {
		return nil, err
	}

	mux := http.NewServeMux()
	mux.HandleFunc("/org-one/keys", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgOneJwks)
	})
	mux.HandleFunc("/org-two/keys", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgTwoJwks)
	})
	mux.HandleFunc("/org-three/keys", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgThreeJwks)
	})
	mux.HandleFunc("/org-four/keys", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgFourJwks)
	})
	mux.HandleFunc("/org-one/jwt", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgOneJwt)
	})
	mux.HandleFunc("/org-two/jwt", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgTwoJwt)
	})
	mux.HandleFunc("/org-three/jwt", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgThreeJwt)
	})
	mux.HandleFunc("/org-four/jwt", func(w http.ResponseWriter, req *http.Request) {
		w.Header().Add("content-type", "application/json")
		w.Write(orgFourJwt)
	})

	// OAuth2/OIDC endpoints
	mux.HandleFunc("/register", handleRegister)
	mux.HandleFunc("/authorize", handleAuthorize)
	mux.HandleFunc("/token", handleToken)
	mux.HandleFunc("/token-exchange", handleOAuthTokenExchange)
	// Handle .well-known paths - register each path explicitly
	mux.HandleFunc("/.well-known/jwks.json", handleJWKS)
	mux.HandleFunc("/.well-known/oauth-authorization-server", handleDiscovery)

	// Add CORS middleware for all routes
	muxWithCORS := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodOptions {
			handleOPTIONS(w, r)
			return
		}
		mux.ServeHTTP(w, r)
	})

	// nolint: gosec // Test code only
	cfg := &tls.Config{
		RootCAs:      roots,
		Certificates: []tls.Certificate{cert},
		NextProtos:   []string{"http/1.1"},
	}

	// nolint: gosec // Test code only
	srv := &http.Server{
		Addr:         "0.0.0.0:8443",
		Handler:      muxWithCORS,
		TLSConfig:    cfg,
		TLSNextProto: make(map[string]func(*http.Server, *tls.Conn, http.Handler), 0),
	}

	return serveHTTP("dummy-idp", srv, func() error {
		return srv.ListenAndServeTLS("", "")
	}), nil
}

// OAuth2/OIDC constants
const (
	hardcodedClientID = "mcp_gi3APARn2_uHv2oxfJJqq2yZBDV4OyNo"
	hardcodedCode     = "fixed_auth_code_123"
	// nolint: gosec // Test code only
	hardcodedClientSecret = "secret_2nGx_bjvo9z72Aw3-hKTWMusEo2-yTfH"
	hardcodedRefreshToken = "fixed_refresh_token_123"
	redirectURI           = "http://localhost:8081/callback"
)

const (
	oauthGrantTypeTokenExchange = "urn:ietf:params:oauth:grant-type:token-exchange" // #nosec G101
	oauthGrantTypeJWTBearer     = "urn:ietf:params:oauth:grant-type:jwt-bearer"     // #nosec G101
	oauthTokenTypeAccessToken   = "urn:ietf:params:oauth:token-type:access_token"   // #nosec G101
	oauthTokenTypeIDToken       = "urn:ietf:params:oauth:token-type:id_token"       // #nosec G101
	oauthTokenTypeIDJAG         = "urn:ietf:params:oauth:token-type:id-jag"         // #nosec G101
	oauthTokenTypeJWT           = "urn:ietf:params:oauth:token-type:jwt"            // #nosec G101
)

// sendJSONResponse sends a JSON response with CORS headers
func sendJSONResponse(w http.ResponseWriter, r *http.Request, data any, statusCode int) {
	w.Header().Set("Content-Type", "application/json")
	origin := r.Header.Get("Origin")
	if origin == "" {
		origin = "*"
	}
	w.Header().Set("Access-Control-Allow-Origin", origin)
	w.Header().Set("Vary", "Origin")
	w.Header().Set("Access-Control-Allow-Credentials", "true")
	requestHeaders := r.Header.Get("Access-Control-Request-Headers")
	if requestHeaders == "" {
		requestHeaders = "content-type, authorization"
	}
	w.Header().Set("Access-Control-Allow-Headers", requestHeaders)
	w.WriteHeader(statusCode)
	json.NewEncoder(w).Encode(data)
}

// handleRegister handles OAuth2 client registration
func handleRegister(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}

	registration := map[string]any{
		"client_id":                  hardcodedClientID,
		"client_secret":              hardcodedClientSecret,
		"client_name":                "Test Client",
		"client_description":         "A test MCP client",
		"redirect_uris":              []string{redirectURI},
		"grant_types":                []string{"authorization_code", "refresh_token"},
		"response_types":             []string{"code"},
		"token_endpoint_auth_method": "client_secret_basic",
		"created_at":                 time.Now().Format(time.RFC3339Nano),
		"updated_at":                 time.Now().Format(time.RFC3339Nano),
	}
	sendJSONResponse(w, r, registration, http.StatusOK)
}

// handleAuthorize handles OAuth2 authorization endpoint
func handleAuthorize(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}

	query := r.URL.Query()
	clientID := query.Get("client_id")
	redirectURI := query.Get("redirect_uri")

	if clientID != hardcodedClientID {
		sendJSONResponse(w, r, map[string]string{"error": "invalid_client"}, http.StatusBadRequest)
		return
	}

	callbackURL := fmt.Sprintf("%s?code=%s", redirectURI, hardcodedCode)
	sendJSONResponse(w, r, map[string]string{"redirect_to": callbackURL}, http.StatusOK)
}

// handleToken handles OAuth2 token endpoint
func handleToken(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}

	if err := r.ParseForm(); err != nil {
		sendJSONResponse(w, r, map[string]string{"error": "invalid_request"}, http.StatusBadRequest)
		return
	}

	grantType := r.FormValue("grant_type")
	clientID := r.FormValue("client_id")
	clientSecret := r.FormValue("client_secret")

	// Extract Basic auth header if client_id not in body
	authHeader := r.Header.Get("Authorization")
	if clientID == "" && strings.HasPrefix(authHeader, "Basic ") {
		decoded, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(authHeader, "Basic "))
		if err == nil {
			parts := strings.SplitN(string(decoded), ":", 2)
			if len(parts) == 2 {
				clientID = parts[0]
				clientSecret = parts[1]
			}
		}
	}

	switch grantType {
	case "authorization_code":
		// Be lenient for generic MCP inspectors/SPAs using PKCE:
		// - Do not require client_secret (public client)
		// - Accept any code/redirect_uri/code_verifier
		response := map[string]any{
			"access_token":  string(orgOneJwt),
			"refresh_token": hardcodedRefreshToken,
			"token_type":    "bearer",
			"expires_in":    3600,
		}
		sendJSONResponse(w, r, response, http.StatusOK)

	case "refresh_token":
		// For refresh token, still require confidential client auth
		if clientID != hardcodedClientID || clientSecret != hardcodedClientSecret {
			sendJSONResponse(w, r, map[string]string{"error": "invalid_client"}, http.StatusBadRequest)
			return
		}
		// Accept any refresh_token for testing purposes
		response := map[string]any{
			"access_token":  string(orgOneJwt),
			"refresh_token": hardcodedRefreshToken,
			"token_type":    "bearer",
			"expires_in":    3600,
		}
		sendJSONResponse(w, r, response, http.StatusOK)

	default:
		sendJSONResponse(w, r, map[string]string{"error": "unsupported_grant_type"}, http.StatusBadRequest)
	}
}

func handleOAuthTokenExchange(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		writeOAuthTokenExchangeError(w, r, http.StatusMethodNotAllowed, "method_not_allowed")
		return
	}
	if err := r.ParseForm(); err != nil {
		writeOAuthTokenExchangeError(w, r, http.StatusBadRequest, "invalid_request")
		return
	}

	switch r.FormValue("grant_type") {
	case oauthGrantTypeTokenExchange:
		handleTokenExchangeGrant(w, r, r.Form)
	case oauthGrantTypeJWTBearer:
		handleJWTBearerGrant(w, r, r.Form)
	default:
		writeOAuthTokenExchangeError(w, r, http.StatusBadRequest, "unsupported_grant_type")
	}
}

func handleTokenExchangeGrant(w http.ResponseWriter, r *http.Request, form url.Values) {
	if form.Get("requested_token_type") == oauthTokenTypeIDJAG {
		handleCrossAppAccessIdentityAssertionGrant(w, r, form)
		return
	}

	required := map[string][]string{
		"subject_token":        {"subject-token"},
		"subject_token_type":   {oauthTokenTypeAccessToken},
		"actor_token":          {"actor-token"},
		"actor_token_type":     {oauthTokenTypeJWT},
		"audience":             {"api://echo"},
		"resource":             {"https://echo.example.com"},
		"scope":                {"echo.read"},
		"requested_token_type": {oauthTokenTypeAccessToken},
		"client_id":            {"oauth-e2e-client"},
		"tenant":               {"tenant-a"},
	}
	if !formContainsValues(form, required) {
		writeOAuthTokenExchangeError(w, r, http.StatusBadRequest, "invalid_request")
		return
	}

	writeOAuthTokenExchangeResponse(w, r, "token-exchange-access")
}

func handleCrossAppAccessIdentityAssertionGrant(w http.ResponseWriter, r *http.Request, form url.Values) {
	required := map[string][]string{
		"subject_token":        {"subject-id-token"},
		"subject_token_type":   {oauthTokenTypeIDToken},
		"audience":             {"api://cross-app-resource"},
		"resource":             {"https://cross-app.example.com"},
		"scope":                {"cross-app.read"},
		"requested_token_type": {oauthTokenTypeIDJAG},
		"client_id":            {"cross-app-idp-client"},
	}
	if !formContainsValues(form, required) {
		writeOAuthTokenExchangeError(w, r, http.StatusBadRequest, "invalid_request")
		return
	}

	if form.Has("actor_token") || form.Has("actor_token_type") {
		writeOAuthTokenExchangeError(w, r, http.StatusBadRequest, "invalid_request")
		return
	}

	sendJSONResponse(w, r, map[string]any{
		"access_token":      "id-jag-assertion",
		"token_type":        "N_A",
		"issued_token_type": oauthTokenTypeIDJAG,
		"expires_in":        60,
	}, http.StatusOK)
}

func handleJWTBearerGrant(w http.ResponseWriter, r *http.Request, form url.Values) {
	if form.Get("assertion") == "id-jag-assertion" {
		handleCrossAppAccessResourceGrant(w, r, form)
		return
	}

	required := map[string][]string{
		"assertion": {"jwt-assertion"},
		"client_id": {"oauth-e2e-jwt-client"},
	}
	if !formContainsValues(form, required) {
		writeOAuthTokenExchangeError(w, r, http.StatusBadRequest, "invalid_request")
		return
	}

	if form.Has("subject_token") || form.Has("actor_token") || form.Has("requested_token_type") {
		writeOAuthTokenExchangeError(w, r, http.StatusBadRequest, "invalid_request")
		return
	}

	writeOAuthTokenExchangeResponse(w, r, "jwt-bearer-access")
}

func handleCrossAppAccessResourceGrant(w http.ResponseWriter, r *http.Request, form url.Values) {
	required := map[string][]string{
		"assertion": {"id-jag-assertion"},
		"client_id": {"cross-app-resource-client"},
		"scope":     {"cross-app.read"},
	}
	if !formContainsValues(form, required) {
		writeOAuthTokenExchangeError(w, r, http.StatusBadRequest, "invalid_request")
		return
	}

	if form.Has("subject_token") || form.Has("actor_token") || form.Has("requested_token_type") || form.Has("resource") {
		writeOAuthTokenExchangeError(w, r, http.StatusBadRequest, "invalid_request")
		return
	}

	writeOAuthTokenExchangeResponse(w, r, "cross-app-access-token")
}

func formContainsValues(form url.Values, required map[string][]string) bool {
	for key, wantValues := range required {
		gotValues := form[key]
		if len(gotValues) != len(wantValues) {
			return false
		}
		for _, want := range wantValues {
			if !slices.Contains(gotValues, want) {
				return false
			}
		}
	}
	return true
}

func writeOAuthTokenExchangeResponse(w http.ResponseWriter, r *http.Request, accessToken string) {
	sendJSONResponse(w, r, map[string]any{
		"access_token":      accessToken,
		"token_type":        "Bearer",
		"issued_token_type": oauthTokenTypeAccessToken,
		"expires_in":        60,
	}, http.StatusOK)
}

func writeOAuthTokenExchangeError(w http.ResponseWriter, r *http.Request, status int, err string) {
	sendJSONResponse(w, r, map[string]string{
		"error": err,
	}, status)
}

// handleJWKS handles JWKS endpoint using orgOneJwks
func handleJWKS(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}
	// Set CORS headers
	origin := r.Header.Get("Origin")
	if origin == "" {
		origin = "*"
	}
	w.Header().Set("Access-Control-Allow-Origin", origin)
	w.Header().Set("Vary", "Origin")
	w.Header().Set("Access-Control-Allow-Credentials", "true")
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusOK)
	w.Write(orgOneJwks)
}

// handleDiscovery handles OAuth2 discovery endpoint
func handleDiscovery(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		sendJSONResponse(w, r, map[string]string{"error": "method_not_allowed"}, http.StatusMethodNotAllowed)
		return
	}

	// Determine base URL from request
	scheme := "https"
	if r.TLS == nil {
		scheme = "http"
	}
	host := r.Host
	if host == "" {
		host = "localhost:8443"
	}
	baseURL := fmt.Sprintf("%s://%s", scheme, host)

	discovery := map[string]any{
		"issuer":                                "https://agentgateway.dev",
		"authorization_endpoint":                fmt.Sprintf("%s/authorize", baseURL),
		"token_endpoint":                        fmt.Sprintf("%s/token", baseURL),
		"jwks_uri":                              fmt.Sprintf("%s/.well-known/jwks.json", baseURL),
		"registration_endpoint":                 fmt.Sprintf("%s/register", baseURL),
		"response_types_supported":              []string{"code"},
		"grant_types_supported":                 []string{"authorization_code", "refresh_token"},
		"token_endpoint_auth_methods_supported": []string{"none", "client_secret_basic", "client_secret_post"},
		"code_challenge_methods_supported":      []string{"S256"},
	}
	sendJSONResponse(w, r, discovery, http.StatusOK)
}

// handleOPTIONS handles CORS preflight requests
func handleOPTIONS(w http.ResponseWriter, r *http.Request) {
	origin := r.Header.Get("Origin")
	if origin == "" {
		origin = "*"
	}
	requestHeaders := r.Header.Get("Access-Control-Request-Headers")
	if requestHeaders == "" {
		requestHeaders = "content-type"
	}

	w.Header().Set("Access-Control-Allow-Origin", origin)
	w.Header().Set("Vary", "Origin")
	w.Header().Set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
	w.Header().Set("Access-Control-Allow-Headers", requestHeaders)
	w.Header().Set("Access-Control-Allow-Credentials", "true")
	w.WriteHeader(http.StatusNoContent)
}

var (
	// jwks and jwts were generated using hack/utils/jwt/jwt-generator.go
	// jwts are valid until Aug 2035
	//   "iss": "https://agentgateway.dev",
	//   "sub": "ignore@agentgateway.dev",
	orgOneJwks = testjwt.OrgOneJWKS
	orgOneJwt  = []byte(testjwt.OrgOneJWT)
	orgTwoJwks = testjwt.OrgTwoJWKS
	orgTwoJwt  = []byte(testjwt.OrgTwoJWT)

	orgThreeJwks = []byte(`{"keys":[{"use":"sig","kty":"RSA","kid":"9005476577230381302","n":"vL5EM7MYEP85dQ5XoZUZjWvQ4v572jb3At6zj5LhdcBe2HjPxrdmoQCnrB1vyQXVflFGHgrPYdlEKQkY1Jr3FLjHdV8QryxzXKDsNHtA_jltALqhldFoVqRUp0teh7GzVOnwynPrt4gNsJbhldhD7mi4ILX0dYE45EtsYKjj_sUMaImArwLbhTW4eJ0eWtha7fBd42MKp4mT_DsIh6WhnFZUZU-NayqSaN6xcERrcNZ0Mc5lE_M10JiMZYAuxeE84Swg2DuDcvqDJlxEB4yhvwQ9yxY0fR2M62zMNg8D9qabkIHjWpbRRLpGuFVZYKcdZZnAGrtAoRdti13vUCdXnQ","e":"AQAB"}]}`)
	orgThreeJwt  = []byte(`eyJhbGciOiJSUzI1NiIsImtpZCI6IjkwMDU0NzY1NzcyMzAzODEzMDIiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2FnZW50Z2F0ZXdheS5kZXYiLCJzdWIiOiJpZ25vcmVAYWdlbnRnYXRld2F5LmRldiIsImV4cCI6MjA4NTU4MDcxOSwibmJmIjoxNzc3OTk2NzE5LCJpYXQiOjE3Nzc5OTY3MTl9.YBNFBVgjQyPjoHerQG26W6P8pl__pDU9mUIYP4yiMwiMQ4f1LY_L46up1uvdIOsEcEdpFU_6hMFJVVXMyNfENlczlTuSLtRj3T-bzArdo3vR67rTTh-tawAv-UerDZgEfNXUjJYNrIXWEgzsxZ7-1_AtgyLzxldcwePJBJH9kfcwceKh7cbK46JT45ZA9CQ2RCBZ8682b64AestRF3yVTQGnMlKW7vlXtEo4dxHrnyI67ZCfcWMvd_wbsvfAow6W7sOUERD4vhtO0NU8W3fX9QtwchYIpO8ZqvHp-Ehk_WCPmBb7ANTmZgjx4uVGnPYSYndaLNUYif0jxT9K00Mnag`)

	orgFourJwks = testjwt.OrgFourJWKS
	orgFourJwt  = []byte(testjwt.OrgFourJWT)
)
