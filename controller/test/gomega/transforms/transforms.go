package transforms

import (
	"encoding/json"
	"strings"

	"istio.io/istio/pkg/test/echo"
	"istio.io/istio/pkg/test/echo/proto"
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

// WithEchoBody returns a Gomega Transform that extracts the Echo server body from the
// response and returns it as a map[string]interface{}
func WithEchoBody() func(b []byte) echo.Response {
	return func(b []byte) echo.Response {
		dummyReq := &proto.ForwardEchoRequest{}
		dummyResp := &proto.ForwardEchoResponse{Output: []string{string(b)}}
		resp := echo.ParseResponses(dummyReq, dummyResp)
		bb := resp[0]
		return bb
	}
}

func WithEchoHeaders() func(b []byte) map[string]string {
	return func(b []byte) map[string]string {
		bb := WithEchoBody()(b)
		h := make(map[string]string)
		for k, v := range bb.RequestHeaders {
			h[k] = strings.Join(v, ",")
		}
		return h
	}
}
