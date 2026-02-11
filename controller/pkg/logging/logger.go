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
	"log/slog"
	"sync"
)

const (
	DefaultComponent = "default"
)

// componentLeveler maps component names to their respective slog.LevelVar instance
var componentLeveler sync.Map

func init() {
	defaultLogger := New(DefaultComponent)
	slog.SetDefault(defaultLogger)
}

// New returns a new slog.Logger instance for the given component with default Options.
// If the component is empty, it returns the default logger.
// NOTE: the logger's leveler is tracked in memory, so this should be used to create long lived loggers
// whose log levels are required to be configurable at runtime.
// Short lived loggers should be instantiated using logger.With() or call DeleteLeveler if instantiated
// with New, to avoid leaking memory.
func New(component string) *slog.Logger {
	return NewWithOptions(component, Options{})
}

// NewWithOptions returns a new slog.Logger instance for the given component with the provided Options
// If the component is empty, it returns the default logger.
// NOTE: the logger's leveler is tracked in memory, so this should be used to create long lived loggers
// whose log levels are required to be configurable at runtime.
// Short lived loggers should be instantiated using logger.With() or call DeleteLeveler if instantiated
// with NewWithOptions, to avoid leaking memory.
func NewWithOptions(component string, opts Options) *slog.Logger {
	if component == "" {
		return slog.Default()
	}

	opts.Default()

	level := &slog.LevelVar{}
	if opts.Level != nil {
		level.Set(*opts.Level)
	} else {
		defaultLvl, ok := componentLeveler.Load(DefaultComponent)
		if ok {
			level.Set(defaultLvl.(*slog.LevelVar).Level())
		}
	}
	handlerOpts := &slog.HandlerOptions{
		AddSource:   opts.AddSource || level.Level() <= slog.LevelDebug,
		Level:       level,
		ReplaceAttr: slogLevelReplacer,
	}

	attrs := []slog.Attr{{Key: "component", Value: slog.StringValue(component)}}

	componentLeveler.Store(component, level)
	var slogHandler slog.Handler
	switch opts.Format {
	case TextFormat:
		slogHandler = slog.NewTextHandler(opts.Writer, handlerOpts).WithAttrs(attrs)
	case JSONFormat:
		slogHandler = slog.NewJSONHandler(opts.Writer, handlerOpts).WithAttrs(attrs)
	default:
		slogHandler = slog.NewTextHandler(opts.Writer, handlerOpts).WithAttrs(attrs)
	}

	return slog.New(slogHandler)
}

// GetComponentLevels returns a map of component names to their respective slog.Level
func GetComponentLevels() map[string]slog.Level {
	levels := make(map[string]slog.Level)
	componentLeveler.Range(func(key any, value any) bool {
		levels[key.(string)] = value.(*slog.LevelVar).Level()
		return true
	})
	return levels
}
