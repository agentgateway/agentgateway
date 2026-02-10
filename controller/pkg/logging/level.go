/*
Portions of this file are derived from the slog-leveler project
(https://github.com/shashankram/slog-leveler)
which is licensed under the MIT License.

# MIT License

# Copyright (c) 2025 Shashank Ram

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
*/
package logging

import (
	"fmt"
	"log/slog"
	"net/http"
	"strings"
)

// Extra slog log levels
const (
	LevelTrace = slog.Level(-5) // 1 lower than slog.LevelDebug
)

const (
	levelQuery = "level"
)

// Level strings
const (
	errorLevel = "error"
	warnLevel  = "warn"
	infoLevel  = "info"
	debugLevel = "debug"
	traceLevel = "trace"
)

var (
	// GlobalLevel is the slog.LevelVar for the default logger
	GlobalLevel = &slog.LevelVar{} // default is INFO

	levelNames = map[slog.Leveler]string{
		LevelTrace: traceLevel,
	}
)

// GetLevel returns the current log level for the component
func GetLevel(component string) (slog.Level, error) {
	if component == "" {
		component = DefaultComponent
	}
	lvl, ok := componentLeveler.Load(component)
	if !ok {
		return slog.Level(0), fmt.Errorf("logger not found for component: %s", component)
	}
	levelr := lvl.(*slog.LevelVar)
	return levelr.Level(), nil
}

// MustGetLevel returns the current log level for the component or panics if the component is not found
func MustGetLevel(component string) slog.Level {
	level, err := GetLevel(component)
	if err != nil {
		panic(err)
	}
	return level
}

// SetLevel sets the log level for the component
func SetLevel(component string, level slog.Level) error {
	if component == "" {
		component = DefaultComponent
	}
	lvl, ok := componentLeveler.Load(component)
	if !ok {
		return fmt.Errorf("logger not found for component: %s", component)
	}
	levelr := lvl.(*slog.LevelVar)
	levelr.Set(level)
	return nil
}

// MustSetLevel sets the log level for the component or panics if the component is not found
func MustSetLevel(component string, level slog.Level) {
	if err := SetLevel(component, level); err != nil {
		panic(err)
	}
}

// Reset resets the log level for all components to the given level
func Reset(level slog.Level) {
	componentLeveler.Range(func(key any, value any) bool {
		MustSetLevel(key.(string), level)
		return true
	})
}

// HTTPLelevelHandler handles HTTP requests to the log level of the default or
// component specific loggers
// It accepts POST and PUT requests with the following query parameters:
// - level=<level>: updates log level across all component loggers
// - <component>=<level>&<component=<level2>...: updates log level for specific components
//
// If no query parameters are provided, it returns the current log levels of all components
func HTTPLevelHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost && r.Method != http.MethodPut {
		http.Error(w, "method must be one of POST|PUT", http.StatusMethodNotAllowed)
		return
	}

	componentValues := r.URL.Query()
	if lvl := componentValues.Get(levelQuery); lvl != "" {
		level, err := ParseLevel(lvl)
		if err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		Reset(level)
		w.Write(fmt.Appendf(nil, "all logger levels updated to level: %s\n", lvl)) // nolint: errcheck
		return
	}

	levels := make(map[string]slog.Level)
	// Parse ?level= or ?c1=level1&c2=level2,...
	for component := range componentValues {
		l := componentValues.Get(component)
		if l == "" {
			http.Error(w, fmt.Sprintf("component %s: empty value", component), http.StatusBadRequest)
			return
		}

		level, err := ParseLevel(l)
		if err != nil {
			http.Error(w, fmt.Sprintf("component %s: %v", component, err), http.StatusBadRequest)
			return
		}
		levels[component] = level
	}

	// Update component specific log levels
	for component, level := range levels {
		err := SetLevel(component, level)
		if err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		w.Write(fmt.Appendf(nil, "component %s log level set to: %s\n", component, strings.ToLower(levelName(level)))) // nolint: errcheck
	}

	// If no levels were set, write the current log levels
	if len(levels) == 0 {
		// Print current component log levels
		w.Write([]byte("current log levels:\n---\n")) // nolint: errcheck
		componentLeveler.Range(func(key any, value any) bool {
			w.Write(fmt.Appendf(nil, "%s: %s\n", key, LevelToString(value.(*slog.LevelVar).Level()))) // nolint: errcheck
			return true
		})
	}
}

// slogLevelReplacer replaces the slog.Level with a string representation
func slogLevelReplacer(groups []string, attr slog.Attr) slog.Attr {
	if attr.Key == slog.LevelKey {
		level := attr.Value.Any().(slog.Level)
		levelname := levelName(level)
		attr.Value = slog.StringValue(levelname)
	}
	return attr
}

// levelName returns the string representation of slog.Level
func levelName(level slog.Level) string {
	levelname, ok := levelNames[level]
	if !ok {
		levelname = strings.ToLower(level.String())
	}
	return levelname
}

// ParseLevel parses the given level string to slog.Level,
// and returns an error if the level is unknown
func ParseLevel(level string) (slog.Level, error) {
	switch strings.ToLower(level) {
	case traceLevel:
		return LevelTrace, nil
	case debugLevel:
		return slog.LevelDebug, nil
	case infoLevel:
		return slog.LevelInfo, nil
	case warnLevel:
		return slog.LevelWarn, nil
	case errorLevel:
		return slog.LevelError, nil
	default:
		return slog.LevelInfo, fmt.Errorf("unknown log level %s; should be one of error|warn|info|debug|trace", level)
	}
}

// LevelToString returns the string representation of slog.Level
func LevelToString(level slog.Level) string {
	switch level {
	case LevelTrace:
		return traceLevel
	case slog.LevelDebug:
		return debugLevel
	case slog.LevelInfo:
		return infoLevel
	case slog.LevelWarn:
		return warnLevel
	case slog.LevelError:
		return errorLevel
	default:
		return level.String()
	}
}
