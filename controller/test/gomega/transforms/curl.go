package transforms

import (
	"bytes"
	"io"
	"net/http"
	"strconv"
	"strings"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/kubeutils/kubectl"
)

const (
	responseHeaderPrefix      = "< "
	responseStatusPrefix1dot1 = "< HTTP/1.1 "
	responseStatusPrefix2     = "< HTTP/2 "
)

func WithCurlResponse(curlResponse *kubectl.CurlResponse) *http.Response {
	headers := make(http.Header)
	statusCode := 0
	var bodyBuf bytes.Buffer

	// Curl writes the body to stdout and the headers/status to stderr
	// Headers/response code
	for line := range strings.SplitSeq(curlResponse.StdErr, "\n") {
		k, v := processResponseHeader(line)
		if k != "" {
			headers.Add(k, v)
			continue
		}

		code := processResponseCode(line)
		if code != 0 {
			statusCode = code
		}
	}

	// Body
	bodyBuf.WriteString(curlResponse.StdOut)

	return &http.Response{
		StatusCode: statusCode,
		Header:     headers,
		Body:       bytesBody(bodyBuf.Bytes()),
	}
}

// processResponseHeader processes the current line if it's a response header.
// Returns header key and value if the line was processed, otherwise returns empty strings.
func processResponseHeader(line string) (string, string) {
	// check for response headers
	if strings.HasPrefix(line, responseHeaderPrefix) {
		headerParts := strings.Split(line[len(responseHeaderPrefix):], ": ")
		if len(headerParts) == 2 {
			// strip "\r" from the end of the value
			return strings.ToLower(headerParts[0]), strings.TrimSuffix(headerParts[1], "\r")
		}
	}
	return "", ""
}

// processResponseCode processes the current line if it's a response status code.
// Returns the status code if the line was processed, otherwise returns 0.
func processResponseCode(line string) int {
	// check for response status. the line with the response code will be in the format
	// `< HTTP/1.1 <code> <message>` or `< HTTP/2 <code> <message>`
	if strings.HasPrefix(line, responseStatusPrefix1dot1) || strings.HasPrefix(line, responseStatusPrefix2) {
		statusParts := strings.Split(line, " ")
		if len(statusParts) >= 4 {
			statusCode, err := strconv.Atoi(statusParts[2])
			if err == nil {
				return statusCode
			}
		}
	}
	return 0
}

func bytesBody(bodyBytes []byte) io.ReadCloser {
	return io.NopCloser(bytes.NewReader(bodyBytes))
}
