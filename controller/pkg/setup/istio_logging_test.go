package setup

import (
	"bytes"
	"encoding/json"
	"log/slog"
	"testing"
	"time"

	"go.uber.org/zap/zapcore"
	"istio.io/istio/pkg/test/util/assert"
)

func TestIstioComponentCoreWritesControllerJSONShape(t *testing.T) {
	tests := map[string]struct {
		scope     string
		component string
	}{
		"default scope": {
			component: istioComponent,
		},
		"klog scope": {
			scope:     "klog",
			component: klogComponent,
		},
	}

	for name, tt := range tests {
		t.Run(name, func(t *testing.T) {
			var out bytes.Buffer
			previousHandler := istioSlogHandler
			istioSlogHandler = slog.NewJSONHandler(&out, &slog.HandlerOptions{ReplaceAttr: istioSlogLevelReplacer})
			t.Cleanup(func() {
				istioSlogHandler = previousHandler
			})

			core := istioComponentCore{Core: zapcore.NewNopCore()}.With([]zapcore.Field{{
				Key: "cluster", Type: zapcore.StringType, String: "primary",
			}})
			if err := core.Write(zapcore.Entry{
				Time:       time.Unix(0, 0).UTC(),
				Level:      zapcore.ErrorLevel,
				LoggerName: tt.scope,
				Message:    "watch error",
			}, []zapcore.Field{{Key: "resource", Type: zapcore.StringType, String: "gateway"}}); err != nil {
				t.Fatalf("write log entry: %v", err)
			}

			var fields map[string]any
			if err := json.Unmarshal(out.Bytes(), &fields); err != nil {
				t.Fatalf("unmarshal log output %q: %v", out.String(), err)
			}

			assert.Equal(t, fields, map[string]any{
				"time":      "1970-01-01T00:00:00Z",
				"level":     "error",
				"msg":       "watch error",
				"component": tt.component,
				"cluster":   "primary",
				"resource":  "gateway",
			})
		})
	}
}
