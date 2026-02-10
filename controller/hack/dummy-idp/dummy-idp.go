package main

import (
	"crypto/tls"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"log"
	"net/http"
	"strings"
	"time"

	_ "embed"
)

//go:embed dummy-idp.cert
var cert []byte

//go:embed dummy-idp.key
var key []byte

func main() {
	roots := x509.NewCertPool()
	if !roots.AppendCertsFromPEM(cert) {
		log.Fatal("failed to append Cert from PEM")
	}

	cert, err := tls.X509KeyPair(cert, key)
	if err != nil {
		log.Panic(err)
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

	cfg := &tls.Config{
		RootCAs:      roots,
		Certificates: []tls.Certificate{cert},
		NextProtos:   []string{"http/1.1"},
	}

	srv := &http.Server{
		Addr:         "0.0.0.0:8443",
		Handler:      muxWithCORS,
		TLSConfig:    cfg,
		TLSNextProto: make(map[string]func(*http.Server, *tls.Conn, http.Handler), 0),
	}

	log.Fatal(srv.ListenAndServeTLS("", ""))
}

// OAuth2/OIDC constants
const (
	hardcodedClientID     = "mcp_gi3APARn2_uHv2oxfJJqq2yZBDV4OyNo"
	hardcodedCode         = "fixed_auth_code_123"
	hardcodedClientSecret = "secret_2nGx_bjvo9z72Aw3-hKTWMusEo2-yTfH"
	hardcodedRefreshToken = "fixed_refresh_token_123"
	redirectURI           = "http://localhost:8081/callback"
)

// sendJSONResponse sends a JSON response with CORS headers
func sendJSONResponse(w http.ResponseWriter, r *http.Request, data interface{}, statusCode int) {
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

	registration := map[string]interface{}{
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
		response := map[string]interface{}{
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
		response := map[string]interface{}{
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

	discovery := map[string]interface{}{
		"issuer":                                "https://kgateway.dev",
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
	//   "iss": "https://kgateway.dev",
	//   "sub": "ignore@kgateway.dev",
	orgOneJwks = []byte(`{"keys":[{"use":"sig","kty":"RSA","kid":"5350231219306038692","n":"nZPFlqxzFp6fpDjtBV4mj9DDqgD2VEm3Ji4cFe99IKBk2B5hT8RFDXHahLwxmUSHcgZkY1cZW167pByxBAL69xqiGhbTDt0LuvKiRo4wysDP_Vod28Pmnh1mCdXxlweH4iDHyjPmEV3bh6AqlDAPX0ZvT3pZnzoVkBIAYeP00_Xo6fUleVMq-b7u6CRbhEX4xdQug7VGd5ZwE2vlWOARAAkaQj0XY6Kz6EHGi1PY5yzHz9hIZhWo0qA9CZ_XIyA12J9ICNFoEpqwCzeSJOeh6jJgPaCQbRe4lBDeHJFa4SKSR_Imau--MpWcN7_2JZ72HUmZRU-9aIhmYkZtdfjwXw","e":"AQAB","x5c":["MIIC3jCCAcagAwIBAgIBITANBgkqhkiG9w0BAQsFADAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwHhcNMjUxMTE5MTkxMDA3WhcNMjUxMTE5MjExMDA3WjAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCdk8WWrHMWnp+kOO0FXiaP0MOqAPZUSbcmLhwV730goGTYHmFPxEUNcdqEvDGZRIdyBmRjVxlbXrukHLEEAvr3GqIaFtMO3Qu68qJGjjDKwM/9Wh3bw+aeHWYJ1fGXB4fiIMfKM+YRXduHoCqUMA9fRm9PelmfOhWQEgBh4/TT9ejp9SV5Uyr5vu7oJFuERfjF1C6DtUZ3lnATa+VY4BEACRpCPRdjorPoQcaLU9jnLMfP2EhmFajSoD0Jn9cjIDXYn0gI0WgSmrALN5Ik56HqMmA9oJBtF7iUEN4ckVrhIpJH8iZq774ylZw3v/YlnvYdSZlFT71oiGZiRm11+PBfAgMBAAGjNTAzMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAMBgNVHRMBAf8EAjAAMA0GCSqGSIb3DQEBCwUAA4IBAQA8ZNw+i8b1mvbPfRXyez2t0B68Eodg+OO2Dki4WTPtIgQaTrC3vHRyHrol479Mmete+3F00NRqfT8Fo06MVbLXv1Zv1d+JQjJmcy4tyVyBm+pKqYXBxuhEIdBmzXGIV36vyZ1rFcm9O81k0OouBVbpKn0JGbpXR4P9GBn50G26lmqBsMIsQ3K0zJl7b9vlVgvZeV4RPBWUTAK9F4LdwrB3NeEdRcI4ri91PfwgOoPe2h3rUcfCb+XSl9tqgrfkX2Gt0H3PCRgre+XdOAwNHaVhrxxWrkacTAK8oQdftBKLiRVsEMqXmV4PpayB0PxEGDDa+XYmEKuF8br4Z+MgFdsJ"]}]}`)
	orgOneJwt  = []byte(`eyJhbGciOiJSUzI1NiIsImtpZCI6IjUzNTAyMzEyMTkzMDYwMzg2OTIiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzExNjM0MDcsIm5iZiI6MTc2MzU3OTQwNywiaWF0IjoxNzYzNTc5NDA3fQ.TsHCCdd0_629wibU4EviEi1-_UXaFUX1NuLgXCrC-tr7kqlcnUJIJC0WSab1EgXKtF8gTfwTUeQcAQNrunwngQU-K9DFcH5-2vnGeiXV3_X3SokkPq74ceRrCFEL2d7YNaGfhq_UNyvKRJsRz-pwdKK7QIPXALmWaUHn7EV7zU-CcPCKNwmt62P88qNp5HYSbgqz_WfnzIIH8LANpCC8fUqVedgTJMJ86E06pfDNUuuXe_fhjgMQXlfyDeUxIuzJunvS2qIqt4IYMzjcQbl2QI1QK3xz37tridSP_WVuuMUe2Lqo0oDjWVpxqPb5fb90W6a6khRP59Pf6qKMbQ9SQg`)

	orgTwoJwks = []byte(`{"keys":[{"use":"sig","kty":"RSA","kid":"2899564237214684947","n":"rMuPE6L_ooj9lg_E55aCxNkqpTj9RN7N9C1aeCbSMwQt2fiAGhze_GQSkEjea3ofYRL9oQpD9xd2e2HBdRyGHtMY6MWOVueAKWqtBNbTgqol0m0X2WzAsjuYyDd52_r985T9DyZNzy-9wd0-BUplKOP2ESpNmrPnz_EEWOKrM2b4BPFfCWxCFFJ12N_gP7Qc6lNBovpWLwfuwdUJpRQ7vJAJP4axObrlOcF78Dz-JelDvn9ZrHMlSMhaSGsQ6u10d_GZ-I_WZx3VxrCIj2mJ340BK4kWLlphH_PGmy51a1zT7Qu7SwwISIEQky9V7JrPXG1bnt6uiqtIH6dSxDm_yQ","e":"AQAB","x5c":["MIIC3jCCAcagAwIBAgIBLDANBgkqhkiG9w0BAQsFADAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwHhcNMjUxMTE5MTkxMjEyWhcNMjUxMTE5MjExMjEyWjAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCsy48Tov+iiP2WD8TnloLE2SqlOP1E3s30LVp4JtIzBC3Z+IAaHN78ZBKQSN5reh9hEv2hCkP3F3Z7YcF1HIYe0xjoxY5W54Apaq0E1tOCqiXSbRfZbMCyO5jIN3nb+v3zlP0PJk3PL73B3T4FSmUo4/YRKk2as+fP8QRY4qszZvgE8V8JbEIUUnXY3+A/tBzqU0Gi+lYvB+7B1QmlFDu8kAk/hrE5uuU5wXvwPP4l6UO+f1mscyVIyFpIaxDq7XR38Zn4j9ZnHdXGsIiPaYnfjQEriRYuWmEf88abLnVrXNPtC7tLDAhIgRCTL1Xsms9cbVue3q6Kq0gfp1LEOb/JAgMBAAGjNTAzMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAMBgNVHRMBAf8EAjAAMA0GCSqGSIb3DQEBCwUAA4IBAQBY0Z+1dg/TQbNPuBDO+1z0vI83zKsQBUE0IvN4W7mPBd8AV4/Zv+yiD6HhUG4Rs5Y7nKIdIJEBxo14pu8Ve2gdel/2E1hLomot6yKDq3qP7G5zmvDhPharuxuTb1hkEyWOWCbX9F8MANrQUyAJdebBlrdRPUjDpF1wmoKRM6NIh61oeS3ozOaAnuK6crW4/UZPZQ8/Roy68lfGtyWfzWqxawxhQLWZB6VGyipHtk6fqqqSO354TuTYTsMpZY3MCS4GJ9vmAbB6egrFxHmiGSQQY/nc/nxYcrusbyRDeYLYWbU+leTCwuXIkUdEfLRApn4KmyVA6PlakvHY7sd0f+Es"]}]}`)
	orgTwoJwt  = []byte(`eyJhbGciOiJSUzI1NiIsImtpZCI6IjI4OTk1NjQyMzcyMTQ2ODQ5NDciLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzExNjM1MzIsIm5iZiI6MTc2MzU3OTUzMiwiaWF0IjoxNzYzNTc5NTMyfQ.kLazcb2o_zcVfJ7WECsQJdOaluxAJ-GdOkeuXUOJSeN8PvahjxfpftgeJjcGsp2sl-VIKXIuTLH6csHT_CBq7kI8bVKGDkk8qw3w8gem7MtiXKPMSYiYEHAoCCzsl8O-pGPF6G_PU-CfiWla8CIAjOewLzRmLeAYmwEiUYf8LQ7y6BbVDzvtxIQW3pTurHXFy0TZ6nUGqu_Xwh7uXe42WC0T-9LAI4zsGo5x_FKhlE_6N9_a7R0UIYFeRrbph_b1z47xTZ3YhZBmQmue2j1xR6hwRCnL7mOaCrxdte8SqXNUVA6vPSaiMTSkdmKyeRSzeTliDKiqAmP8eiIaqAoN5A`)

	orgThreeJwks = []byte(`{"keys":[{"use":"sig","kty":"RSA","kid":"8879871533137308459","n":"sjnFKA9NxpP39HykPZX6BqiFXmAAMC0YJ1WC2t_2Vo1kXbI64Pb__eKoGaT2my1xedCqnJVyWDjiRSHSzmiJkJ4_h8d62mzCVN2y3mMCDL75OFjz6Hyn2p5dWoIZ0b5SCiZNvBUxJ6ccN51qctzAeReeMP_xM8sWRAN-Xnp8JCltKLv2Kwme5U7UXwzxUxMJsbm6ZMFy-IUMDdmIHgHkIi8-AIvnP0ddtiH_MrJQ6bMwNjecRJ-f1Ut2FVhVTpLiU43UUYExEHLtMXl60ph0RI0mD--FvNmVaYPsysX7FejR49FyCOiCMznOrc_nnKB0M7oggvmjAr8dGghMmL_7VQ","e":"AQAB","x5c":["MIIC3jCCAcagAwIBAgIBIzANBgkqhkiG9w0BAQsFADAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwHhcNMjUxMTE5MTkxMjU4WhcNMjUxMTE5MjExMjU4WjAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCyOcUoD03Gk/f0fKQ9lfoGqIVeYAAwLRgnVYLa3/ZWjWRdsjrg9v/94qgZpPabLXF50KqclXJYOOJFIdLOaImQnj+Hx3rabMJU3bLeYwIMvvk4WPPofKfanl1aghnRvlIKJk28FTEnpxw3nWpy3MB5F54w//EzyxZEA35eenwkKW0ou/YrCZ7lTtRfDPFTEwmxubpkwXL4hQwN2YgeAeQiLz4Ai+c/R122If8yslDpszA2N5xEn5/VS3YVWFVOkuJTjdRRgTEQcu0xeXrSmHREjSYP74W82ZVpg+zKxfsV6NHj0XII6IIzOc6tz+ecoHQzuiCC+aMCvx0aCEyYv/tVAgMBAAGjNTAzMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAMBgNVHRMBAf8EAjAAMA0GCSqGSIb3DQEBCwUAA4IBAQCB8Dj9WYuJ5bK89WNtCQw8XKlBIOUwUyYxU2X5bvIqQPRnOyBR62GaFDY3ER3gdCqVVwcW01cpBHk91cTPdZnWh5wnFTrQuUUA65FcbN8haNIY75OfCQmxxob+yPNJB1wqvTXcUXcF4lN7/7LVpy5jbaJDdWmIKhDPXumgb+pjNsN4VwsF5vbtkdXEDwfA9/BI2POyjlstbz1aYwvrLM6KlOFkE/2oq9r1IksMMg9RIHhAHX1vEDrmxGYdYmPF/mHpQzBu9vdgCUx2pR11vvShc7T2JxaZrsTB0eA4Zli6CayOjWJQILBGxt5btUJxNjKCAwTyaq87iY4CwtxB2jip"]}]}`)
	orgThreeJwt  = []byte(`eyJhbGciOiJSUzI1NiIsImtpZCI6Ijg4Nzk4NzE1MzMxMzczMDg0NTkiLCJ0eXAiOiJKV1QifQ.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6Imlnbm9yZUBrZ2F0ZXdheS5kZXYiLCJleHAiOjIwNzExNjM1NzgsIm5iZiI6MTc2MzU3OTU3OCwiaWF0IjoxNzYzNTc5NTc4fQ.IOrJpU5RY8uhU403MiwRuSa5u6SHAtTeGkTEzn9Hg1DH963AH0NAOMfhx4orSKYbqKhjCPfo-cpKpxizafKFP6j9Ln4Is8ycfk9oPC8Sor_GfhAsJuK3N8fC8mnhm5xQMGk9XErvn9ZY4FCXxpK8vUUMUNUhIsE_zKxJR_Wt6HQ43SGaxuLggR5ETbLvSMDESJEuUdeY_fB_5tYaAznYxOLJ4zp87gKeFPPmEqyzISnRgcEHpyev7BM88uRQGrvF34AiWZO2uDuDGv5zJF9dFm_HQ4-QPe7xEZPvj9w_mbSRQn_RilE2mXduXcU1t-XLxFUVmYj2poiAuUXpwLciXw`)

	orgFourJwks = []byte(`{"keys":[{"use":"sig","kty":"RSA","kid":"292910025153196340","n":"pq97a9fOT8ycnVo_xREFh4TW3Fo-zM-tk5xOxWv2rXRz1fWauxrKdTNaX8FgqKy8Pt2Y7UaWQQRnUnalPARBcPbYShTzOf1GbzIhwgjPbUTtD0WzeVVHk9so76Ab95O2kfaKhpWEnne43g06LKXKQMqOOUttXGjL6YzJT0F59oo5N-Je--XEDtV_QCfb3Qh73QbRO29rw7SAJePse32gKYB7-F1IGZm_P8S7nEXqZ1ZwudBifyQ7KBiP6PsKhonWZRA_4ocSTIwADnsU1VUACxi1FaS2rYl16t6UzT-uzYdhaVWlcRcJblsM66TZPDLwGZxw9IFgx9QAsIeZ_YAcKw","e":"AQAB","x5c":["MIIC3jCCAcagAwIBAgIBWzANBgkqhkiG9w0BAQsFADAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwHhcNMjUxMjEyMjA1NjE5WhcNMjUxMjEyMjI1NjE5WjAXMRUwEwYDVQQKEwxrZ2F0ZXdheS5kZXYwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQCmr3tr185PzJydWj/FEQWHhNbcWj7Mz62TnE7Fa/atdHPV9Zq7Gsp1M1pfwWCorLw+3ZjtRpZBBGdSdqU8BEFw9thKFPM5/UZvMiHCCM9tRO0PRbN5VUeT2yjvoBv3k7aR9oqGlYSed7jeDTospcpAyo45S21caMvpjMlPQXn2ijk34l775cQO1X9AJ9vdCHvdBtE7b2vDtIAl4+x7faApgHv4XUgZmb8/xLucRepnVnC50GJ/JDsoGI/o+wqGidZlED/ihxJMjAAOexTVVQALGLUVpLatiXXq3pTNP67Nh2FpVaVxFwluWwzrpNk8MvAZnHD0gWDH1ACwh5n9gBwrAgMBAAGjNTAzMA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAMBgNVHRMBAf8EAjAAMA0GCSqGSIb3DQEBCwUAA4IBAQAfRX+g3QtPKqmejs+uu4r8x2g4cqU/KG7tqk/cAKIyiDLhlD2dJB47kcRMWmxvqEDamYROho/JPm8SJFPCMwNo9mVE0VcudYPbRkCn6yyKEGZFuimddQeL7KDoLLinbsDmGGXyEdHU/fPRi3zL8FlnCG1OWzSmevdq2p1HsNllJ9QdCiPEIgv0W9V0u+SxD0drMusF0jI/GUYRnbPniY7ieX0HDkdds5zmw1WNCV2gv1YZg2sUJll6BEEmy4TxmSu0+DzmjDbeqvs4HGJpzTDUvgwdTpawKyKlZaFcF6w7sZ41C6RTRDy903vaeDQBI8quP6iaUWBZ4ruJ41ns4l1+"]}]}`)
	// "sub": "boom@kgateway.dev",
	orgFourJwt = []byte(`eyJhbGciOiJSUzI1NiIsImtpZCI6IjI5MjkxMDAyNTE1MzE5NjM0MCIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJodHRwczovL2tnYXRld2F5LmRldiIsInN1YiI6ImJvb21Aa2dhdGV3YXkuZGV2IiwiZXhwIjoyMDczMTU2OTc5LCJuYmYiOjE3NjU1NzI5NzksImlhdCI6MTc2NTU3Mjk3OX0.juMOUmoChZEE_AQVZv3jwtZjytWfzN23-palLXA-DIsSa4-f-lmf3CQiwXz0n1YlSY_dt3rGO6OsDdkYn8wkYEVoQVh11crJvZ5FhpIlZlROOSp03KTW2mQ1XwGYRxffzdzBv65LrFYWK0iNQH2NKfqOzVo5xt3SLTJuxIvCE8-qnqXUWrADw3b2TIzE7SgN7xXzeRGwTpgltq4BswdkB0R5g_1xtbrcdFgT533vt3nCiumhqrBkmk4g02x3L1iSjDCnnwJX2YLHYfpUN0i7SooguTkta067lwBiOi3NOTQjRBOBlZmkoj6sz4YNQ9EwsD74pkNBW9pN-__2cVPBxw`)
)
