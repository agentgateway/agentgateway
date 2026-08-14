package main

import (
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"time"
)

const llmPort = ":9234"

func startLLMServer() (shutdownFunc, error) {
	mux := http.NewServeMux()
	mux.HandleFunc("/v1/chat/completions", handleOpenAIChatCompletions)
	mux.HandleFunc("/v1/messages", handleAnthropicMessages)
	mux.HandleFunc("/request", handleGuardrailsRequest)
	mux.HandleFunc("/response", handleGuardrailsResponse)

	// nolint: gosec // Test code only
	httpServer := &http.Server{
		Addr:    llmPort,
		Handler: mux,
	}

	return serveHTTP("llm", httpServer, httpServer.ListenAndServe), nil
}

func handleOpenAIChatCompletions(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}

	prompt := readLLMPrompt(r)
	content := "The name of this project is agentgateway"
	if strings.Contains(strings.ToLower(prompt), "ssn") {
		content = "123-45-6789 is an example SSN"
	}

	writeJSON(w, map[string]any{
		"id":      "chatcmpl-testbox",
		"object":  "chat.completion",
		"created": time.Now().Unix(),
		"model":   "gpt-4o-mini",
		"choices": []map[string]any{
			{
				"index": 0,
				"message": map[string]any{
					"role":    "assistant",
					"content": content,
				},
				"finish_reason": "stop",
			},
		},
		"service_tier": "default",
		"usage": map[string]any{
			"prompt_tokens":     10,
			"completion_tokens": 10,
			"total_tokens":      20,
		},
	})
}

func handleAnthropicMessages(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}

	prompt := readLLMPrompt(r)
	content := "Data masking is the process of hiding sensitive information by redacting or obfuscating it"
	if strings.Contains(strings.ToLower(prompt), "blocked") {
		content = "blocked content"
	}

	writeJSON(w, map[string]any{
		"id":            "msg_testbox",
		"type":          "message",
		"role":          "assistant",
		"model":         "claude-3-5-sonnet-20240620",
		"stop_reason":   "end_turn",
		"stop_sequence": nil,
		"usage": map[string]any{
			"input_tokens":  8,
			"output_tokens": 8,
		},
		"content": []map[string]any{
			{
				"type": "text",
				"text": content,
			},
		},
	})
}

func readLLMPrompt(r *http.Request) string {
	body, err := io.ReadAll(r.Body)
	if err != nil {
		return ""
	}
	return string(body)
}

func writeJSON(w http.ResponseWriter, resp any) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(resp)
}

func handleGuardrailsRequest(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}

	body := readLLMPrompt(r)
	if strings.Contains(strings.ToLower(body), "blocked content") {
		writeJSON(w, map[string]any{
			"action": map[string]any{
				"body":        "request blocked",
				"status_code": http.StatusForbidden,
			},
		})
		return
	}
	writeJSON(w, map[string]any{
		"action": map[string]any{
			"reason": "request passed",
		},
	})
}

func handleGuardrailsResponse(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}

	var req struct {
		Body struct {
			Choices []struct {
				Message struct {
					Role    string `json:"role"`
					Content string `json:"content"`
				} `json:"message"`
			} `json:"choices"`
		} `json:"body"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		w.WriteHeader(http.StatusBadRequest)
		return
	}

	choices := make([]map[string]any, 0, len(req.Body.Choices))
	for _, choice := range req.Body.Choices {
		choices = append(choices, map[string]any{
			"message": map[string]string{
				"role":    choice.Message.Role,
				"content": strings.ReplaceAll(choice.Message.Content, "masking", "****ing"),
			},
		})
	}
	writeJSON(w, map[string]any{
		"action": map[string]any{
			"body": map[string]any{
				"choices": choices,
			},
			"reason": "response masked",
		},
	})
}
