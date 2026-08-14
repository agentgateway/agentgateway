package setup

import (
	"context"
	"log/slog"
	"os"
	"strings"

	"go.uber.org/zap/zapcore"
	istiolog "istio.io/istio/pkg/log"
	"k8s.io/klog/v2"
)

const (
	istioComponent = "istio"
	klogComponent  = "klog"
)

var istioSlogHandler = slog.NewJSONHandler(os.Stderr, &slog.HandlerOptions{
	ReplaceAttr: istioSlogLevelReplacer,
})

// configureIstioLogging aligns Istio's package-level logger with the controller's
// slog output. Istio uses Zap internally and does not pick up slog.SetDefault, so
// it needs its own startup configuration.
func configureIstioLogging(level istiolog.Level) error {
	opts := istiolog.DefaultOptions()
	opts.JSONEncoding = true
	opts.OutputPaths = []string{"stderr"}
	opts.ErrorOutputPaths = []string{"stderr"}
	opts.SetDefaultOutputLevel(istiolog.OverrideScopeName, level)
	opts.WithExtension(func(core zapcore.Core) (zapcore.Core, func() error, error) {
		return istioComponentCore{Core: core}, func() error { return nil }, nil
	})
	if err := istiolog.Configure(opts); err != nil {
		return err
	}
	klog.SetLogger(istiolog.NewLogrAdapter(istiolog.KlogScope))
	return nil
}

// istioComponentCore keeps Istio's Zap level filtering, then writes through
// slog so Istio and klog lines match the controller JSON shape.
type istioComponentCore struct {
	zapcore.Core
	fields []zapcore.Field
}

func (c istioComponentCore) With(fields []zapcore.Field) zapcore.Core {
	allFields := make([]zapcore.Field, 0, len(c.fields)+len(fields))
	allFields = append(allFields, c.fields...)
	allFields = append(allFields, fields...)
	return istioComponentCore{Core: c.Core, fields: allFields}
}

func (c istioComponentCore) Check(entry zapcore.Entry, checked *zapcore.CheckedEntry) *zapcore.CheckedEntry {
	if c.Enabled(entry.Level) {
		return checked.AddCore(entry, c)
	}
	return checked
}

func (c istioComponentCore) Write(entry zapcore.Entry, fields []zapcore.Field) error {
	allFields := make([]zapcore.Field, 0, len(c.fields)+len(fields))
	allFields = append(allFields, c.fields...)
	allFields = append(allFields, fields...)

	component := istioComponentName(entry.LoggerName)
	record := slog.NewRecord(entry.Time, slogLevel(entry.Level), entry.Message, 0)
	if !hasZapField(allFields, "component") {
		record.AddAttrs(slog.String("component", component))
	}
	record.AddAttrs(zapFieldsToSlogAttrs(allFields)...)
	return istioSlogHandler.Handle(context.Background(), record)
}

func istioComponentName(scope string) string {
	switch scope {
	case "":
		return istioComponent
	case klogComponent:
		return klogComponent
	default:
		return istioComponent + "." + scope
	}
}

func hasZapField(fields []zapcore.Field, key string) bool {
	for _, field := range fields {
		if field.Key == key {
			return true
		}
	}
	return false
}

func zapFieldsToSlogAttrs(fields []zapcore.Field) []slog.Attr {
	if len(fields) == 0 {
		return nil
	}

	attrs := make([]slog.Attr, 0, len(fields))
	for _, field := range fields {
		enc := zapcore.NewMapObjectEncoder()
		field.AddTo(enc)
		if value, ok := enc.Fields[field.Key]; ok {
			attrs = append(attrs, slog.Any(field.Key, value))
			continue
		}
		for key, value := range enc.Fields {
			attrs = append(attrs, slog.Any(key, value))
		}
	}
	return attrs
}

func slogLevel(level zapcore.Level) slog.Level {
	switch level {
	case zapcore.DebugLevel:
		return slog.LevelDebug
	case zapcore.WarnLevel:
		return slog.LevelWarn
	case zapcore.ErrorLevel, zapcore.DPanicLevel, zapcore.PanicLevel, zapcore.FatalLevel:
		return slog.LevelError
	default:
		return slog.LevelInfo
	}
}

func istioSlogLevelReplacer(_ []string, attr slog.Attr) slog.Attr {
	if attr.Key == slog.LevelKey {
		attr.Value = slog.StringValue(strings.ToLower(attr.Value.String()))
	}
	return attr
}
