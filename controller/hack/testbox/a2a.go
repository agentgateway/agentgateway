package main

import (
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"time"
)

type a2aRequest struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id,omitempty"`
	Method  string          `json:"method"`
	Params  struct {
		ID      string `json:"id"`
		Message struct {
			Parts []struct {
				Text string `json:"text"`
			} `json:"parts"`
		} `json:"message"`
	} `json:"params"`
}

func startA2AServer() (shutdownFunc, error) {
	mux := http.NewServeMux()
	mux.HandleFunc("/agent-card", handleAgentCard)
	mux.HandleFunc("/", handleA2ARequest)

	// nolint: gosec // Test code only
	httpServer := &http.Server{
		Addr:    ":9999",
		Handler: mux,
	}

	return serveHTTP("a2a", httpServer, httpServer.ListenAndServe), nil
}

func handleAgentCard(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(map[string]any{
		"name":        "Example A2A Agent",
		"version":     "1.0.0",
		"description": "An example A2A agent using the a2a-protocol crate",
		"skills": []map[string]any{
			{
				"id":          "echo",
				"name":        "Echo Skill",
				"description": "Echoes back the user's message",
			},
		},
	})
}

func handleA2ARequest(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}

	body, err := io.ReadAll(r.Body)
	if err != nil {
		w.WriteHeader(http.StatusBadRequest)
		return
	}

	var req a2aRequest
	if err := json.Unmarshal(body, &req); err != nil {
		w.WriteHeader(http.StatusBadRequest)
		return
	}

	if req.Method != "tasks/send" {
		w.WriteHeader(http.StatusBadRequest)
		return
	}

	userText := ""
	if len(req.Params.Message.Parts) > 0 {
		userText = req.Params.Message.Parts[0].Text
	}
	if userText == "" {
		userText = "hello"
	}

	agentMessage := map[string]any{
		"kind":      "message",
		"messageId": "agent-reply-1",
		"role":      "agent",
		"parts": []map[string]any{
			{"kind": "text", "text": "Echo: " + strings.TrimSpace(userText)},
		},
	}

	result := map[string]any{
		"contextId": "ctx-1",
		"history":   []any{agentMessage},
		"id":        req.Params.ID,
		"kind":      "task",
		"status": map[string]any{
			"message":   agentMessage,
			"state":     "working",
			"timestamp": time.Now().UTC().Format(time.RFC3339Nano),
		},
	}

	resp := map[string]any{
		"jsonrpc": "2.0",
		"id":      req.ID,
		"result":  result,
	}

	w.Header().Set("Content-Type", "application/json")
	_ = json.NewEncoder(w).Encode(resp)
}
