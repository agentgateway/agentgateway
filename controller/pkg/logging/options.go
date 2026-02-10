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
	"io"
	"log/slog"
	"os"
)

// Options to configure the logger
type Options struct {
	// Logger level
	Level *slog.Level

	// Log format: text or json
	Format LogFormat

	// Writer to write logs to
	Writer io.Writer

	// AddSource adds the source code position of the log statement to the output
	AddSource bool
}

// LogFormat represents the format of the log output
type LogFormat string

const (
	// TextFormat represents plain text format
	TextFormat LogFormat = "text"

	// JSONFormat represents JSON format
	JSONFormat LogFormat = "json"
)

// Default sets default values on Options
func (o *Options) Default() {
	// Level implicitly defaults to INFO
	if o.Format == "" {
		o.Format = JSONFormat
	}
	if o.Writer == nil {
		o.Writer = os.Stderr
	}
}
