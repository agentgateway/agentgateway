package virtualkey

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
	"encoding/json"
	"fmt"
	"strings"
	"unicode"

	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	corev1client "k8s.io/client-go/kubernetes/typed/core/v1"
)

const (
	keyPrefix       = "sk"
	keyRandomLength = 18
	maxLabelLength  = 24
)

var keyAlphabet = []byte("0123456789abcdefghijklmnopqrstuvwxyz")

type apiKeyEntry struct {
	Key      string            `json:"key"`
	Metadata map[string]string `json:"metadata,omitempty"`
}

type existingKey struct {
	Secret string
	Entry  string
	Key    string
}

func generateAPIKey(label string) (string, error) {
	random, err := randomBase36(keyRandomLength)
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("%s-%s-%s", keyPrefix, sanitizeLabel(label), random), nil
}

func sanitizeLabel(s string) string {
	var b strings.Builder
	lastHyphen := false
	for _, r := range strings.ToLower(s) {
		if unicode.IsLetter(r) || unicode.IsDigit(r) {
			b.WriteRune(r)
			lastHyphen = false
			continue
		}
		if !lastHyphen {
			b.WriteByte('-')
			lastHyphen = true
		}
	}
	out := strings.Trim(b.String(), "-")
	if out == "" {
		return "key"
	}
	if len(out) > maxLabelLength {
		out = strings.Trim(out[:maxLabelLength], "-")
	}
	if out == "" {
		return "key"
	}
	return out
}

func randomBase36(n int) (string, error) {
	if n < 0 {
		return "", fmt.Errorf("random length must be non-negative")
	}
	out := make([]byte, 0, n)
	buf := make([]byte, n)
	for len(out) < n {
		if _, err := rand.Read(buf); err != nil {
			return "", fmt.Errorf("generate random key material: %w", err)
		}
		for _, b := range buf {
			// 252 is the largest multiple of 36 below 256; rejecting larger
			// values keeps every alphabet character equally likely.
			if b >= 252 {
				continue
			}
			out = append(out, keyAlphabet[int(b)%len(keyAlphabet)])
			if len(out) == n {
				break
			}
		}
	}
	return string(out), nil
}

func loadExistingKeys(ctx context.Context, secrets corev1client.SecretInterface, secretName, selector string) ([]existingKey, error) {
	switch {
	case secretName != "":
		secret, err := secrets.Get(ctx, secretName, metav1.GetOptions{})
		if err != nil {
			return nil, fmt.Errorf("fetch collision-check secret %q: %w", secretName, err)
		}
		return existingKeysFromSecret(secret)
	case selector != "":
		list, err := secrets.List(ctx, metav1.ListOptions{LabelSelector: selector})
		if err != nil {
			return nil, fmt.Errorf("fetch collision-check secrets matching %q: %w", selector, err)
		}
		var keys []existingKey
		for i := range list.Items {
			parsed, err := existingKeysFromSecret(&list.Items[i])
			if err != nil {
				return nil, err
			}
			keys = append(keys, parsed...)
		}
		return keys, nil
	default:
		return nil, nil
	}
}

func existingKeysFromSecret(secret *corev1.Secret) ([]existingKey, error) {
	keys := make([]existingKey, 0, len(secret.Data))
	for entry, value := range secret.Data {
		raw, err := keyFromSecretValue(value)
		if err != nil {
			return nil, fmt.Errorf("secret %q entry %q has invalid API key data: %w", secret.Name, entry, err)
		}
		if raw == "" {
			continue
		}
		keys = append(keys, existingKey{
			Secret: secret.Name,
			Entry:  entry,
			Key:    raw,
		})
	}
	return keys, nil
}

func keyFromSecretValue(value []byte) (string, error) {
	trimmed := strings.TrimSpace(string(value))
	if trimmed == "" {
		return "", nil
	}
	if !strings.HasPrefix(trimmed, "{") {
		return string(value), nil
	}
	var entry struct {
		Key string `json:"key"`
	}
	if err := json.Unmarshal([]byte(trimmed), &entry); err != nil {
		return "", err
	}
	return entry.Key, nil
}

func checkCollisions(imported map[int]string, existing []existingKey) error {
	for row, key := range imported {
		for _, candidate := range existing {
			if subtle.ConstantTimeCompare([]byte(key), []byte(candidate.Key)) == 1 {
				return fmt.Errorf("row %d collides with existing secret %q entry %q", row, candidate.Secret, candidate.Entry)
			}
		}
	}
	return nil
}
