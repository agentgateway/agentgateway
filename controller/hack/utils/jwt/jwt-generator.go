package main

import (
	"crypto/rand"
	"crypto/rsa"
	"encoding/json"
	"flag"
	"fmt"
	random "math/rand"
	"os"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/go-jose/go-jose/v4"
	"github.com/golang-jwt/jwt/v5"
)

// use this to generate jwks and a jwt signed by the key in it

type actorChains []string

func (a *actorChains) String() string { return strings.Join(*a, " ") }

func (a *actorChains) Set(v string) error {
	*a = append(*a, v)
	return nil
}

type claims struct {
	jwt.RegisteredClaims
	Act map[string]any `json:"act,omitempty"`
}

func main() {
	audience := flag.String("audience", "", "optional JWT audience")
	var actors actorChains
	flag.Var(&actors, "actors", "comma-separated RFC 8693 actor subs, current actor first; repeat for more tokens")
	flag.Parse()

	kid := strconv.Itoa(random.Int()) //nolint:gosec
	jwks, key, err := generateJWKS(kid)
	if err != nil {
		fmt.Printf("error generating jwks: %s", err.Error())
		os.Exit(1)
	}

	serializedJwks, err := json.Marshal(jwks)
	if err != nil {
		fmt.Printf("error serializing jwks: %s", err.Error())
		os.Exit(1)
	}

	jwt, err := generateJwt("ignore@agentgateway.dev", *audience, kid, key, nil)
	if err != nil {
		fmt.Printf("error generating jwt: %s", err.Error())
		os.Exit(1)
	}

	jwt1, err := generateJwt("boom@agentgateway.dev", *audience, kid, key, nil)
	if err != nil {
		fmt.Printf("error generating jwt: %s", err.Error())
		os.Exit(1)
	}

	fmt.Printf("jwks: %s\n", string(serializedJwks))
	fmt.Printf("jwt, sub: 'ignore@agentgateway.dev': %s\n", jwt)
	fmt.Printf("jwt, sub: 'boom@agentgateway.dev': %s\n", jwt1)

	for _, chain := range actors {
		token, err := generateJwt("ignore@agentgateway.dev", *audience, kid, key, strings.Split(chain, ","))
		if err != nil {
			fmt.Printf("error generating jwt: %s", err.Error())
			os.Exit(1)
		}
		fmt.Printf("jwt, sub: 'ignore@agentgateway.dev', actors: '%s': %s\n", chain, token)
	}
}

// actClaim nests actors so the first one is the outermost act.
func actClaim(actors []string) map[string]any {
	var act map[string]any
	for _, actor := range slices.Backward(actors) {
		next := map[string]any{"sub": actor}
		if act != nil {
			next["act"] = act
		}
		act = next
	}
	return act
}

func generateJWKS(kid string) (*jose.JSONWebKeySet, *rsa.PrivateKey, error) {
	rsaKey, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		return nil, nil, err
	}

	return &jose.JSONWebKeySet{
		Keys: []jose.JSONWebKey{
			{
				Key:   &rsaKey.PublicKey,
				KeyID: kid,
				Use:   "sig",
			},
		},
	}, rsaKey, nil
}

func generateJwt(sub, audience, kid string, key *rsa.PrivateKey, actors []string) (string, error) {
	var audiences jwt.ClaimStrings
	if audience != "" {
		audiences = jwt.ClaimStrings{audience}
	}
	token := jwt.NewWithClaims(jwt.SigningMethodRS256, claims{
		RegisteredClaims: jwt.RegisteredClaims{
			Issuer:    "https://agentgateway.dev",
			Subject:   sub,
			Audience:  audiences,
			IssuedAt:  jwt.NewNumericDate(time.Now()),
			NotBefore: jwt.NewNumericDate(time.Now()),
			ExpiresAt: jwt.NewNumericDate(time.Now().Add(85440 * time.Hour)), // 10 years
		},
		Act: actClaim(actors),
	})
	token.Header["kid"] = kid
	return token.SignedString(key)
}
