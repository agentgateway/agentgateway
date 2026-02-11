package transforms

import (
	"encoding/json"
)

// WithJsonBody returns a Gomega Transform that extracts the JSON body from the
// response and returns it as a map[string]interface{}
func WithJsonBody() func(b []byte) map[string]any {
	return func(b []byte) map[string]any {
		// parse the response body as JSON
		var bodyJson map[string]any
		json.Unmarshal(b, &bodyJson)

		return bodyJson
	}
}
