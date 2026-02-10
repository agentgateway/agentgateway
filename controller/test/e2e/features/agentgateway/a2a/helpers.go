//go:build e2e

package a2a

import (
	"fmt"
	"io"

	"github.com/google/uuid"

	"github.com/kgateway-dev/kgateway/v2/pkg/utils/requestutils/curl"
	"github.com/kgateway-dev/kgateway/v2/test/e2e/common"
)

func buildMessageSendRequest(text string, id string) string {
	if id == "" {
		id = uuid.New().String()
	}
	messageID := uuid.New().String()
	taskID := fmt.Sprintf("task-%s", uuid.New().String())

	return fmt.Sprintf(`{
		"jsonrpc": "2.0",
		"id": "%s",
		"method": "tasks/send",
		"params": {
			"id": "%s",
			"message": {
				"kind": "message",
				"messageId": "%s",
				"role": "user",
				"parts": [
					{
						"kind": "text",
						"text": "%s"
					}
				]
			}
		}
	}`, id, taskID, messageID, text)
}

func a2aHeaders() map[string]string {
	return map[string]string{
		"Content-Type":  "application/json",
		"Accept":        "application/json",
		"Authorization": "Bearer secret-token",
	}
}

func (s *testingSuite) execCurlA2A(path string, headers map[string]string, body string) (string, error) {
	// Build curl options using the existing curl utilities
	curlOpts := []curl.Option{
		curl.WithHost(common.BaseGateway.Address),
		curl.WithPath(path),
		curl.Silent(),
	}

	// Add headers
	for k, v := range headers {
		curlOpts = append(curlOpts, curl.WithHeader(k, v))
	}

	// Add body
	if body != "" {
		curlOpts = append(curlOpts, curl.WithBody(body))
	}

	// Execute curl request
	resp, err := curl.ExecuteRequest(curlOpts...)
	if err != nil {
		s.T().Logf("curl error: %v", err)
		return "", err
	}
	defer resp.Body.Close()

	// Read response body
	bodyBytes, err := io.ReadAll(resp.Body)
	if err != nil {
		s.T().Logf("read body error: %v", err)
		return "", err
	}

	responseBody := string(bodyBytes)
	s.T().Logf("curl response: %s", responseBody)
	return responseBody, nil
}
