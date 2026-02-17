package kubectl

import (
	"strconv"
	"time"
)

// LogOption represents an option for a kubectl logs request.
type LogOption func(config *logConfig)

type logConfig struct {
	container string
	since     time.Duration
	sinceTime string
	tail      int
}

// WithContainer sets the container name to get logs from (-c, --container)
func WithContainer(container string) LogOption {
	return func(config *logConfig) {
		config.container = container
	}
}

// WithSince sets the relative time to return logs from (--since)
// Example: 5m, 1h, 2h30m
func WithSince(since time.Duration) LogOption {
	return func(config *logConfig) {
		config.since = since
	}
}

// WithSinceTime sets the absolute time to return logs from (--since-time)
// Should be in RFC3339 format, e.g., "2024-01-01T00:00:00Z"
func WithSinceTime(sinceTime string) LogOption {
	return func(config *logConfig) {
		config.sinceTime = sinceTime
	}
}

// WithTail sets the number of lines from the end of the logs to show (--tail)
// Use -1 to show all lines
func WithTail(lines int) LogOption {
	return func(config *logConfig) {
		config.tail = lines
	}
}

// BuildLogArgs constructs the kubectl logs arguments from the provided options
func BuildLogArgs(options ...LogOption) []string {
	// Default config
	cfg := &logConfig{
		container: "",
		since:     0,
		sinceTime: "",
		tail:      -1,
	}

	// Apply options
	for _, opt := range options {
		opt(cfg)
	}

	var args []string

	if cfg.container != "" {
		args = append(args, "-c", cfg.container)
	}

	// --since and --since-time are mutually exclusive, but let kubectl handle that and the messaging
	if cfg.since > 0 {
		args = append(args, "--since", cfg.since.String())
	}

	if cfg.sinceTime != "" {
		args = append(args, "--since-time", cfg.sinceTime)
	}

	if cfg.tail >= 0 {
		args = append(args, "--tail", strconv.Itoa(cfg.tail))
	}

	return args
}

// FormatSinceTime is a helper function to format a time.Time into RFC3339 format
// suitable for use with WithSinceTime
func FormatSinceTime(t time.Time) string {
	return t.Format(time.RFC3339)
}
