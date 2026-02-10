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
	"os"
	"testing"

	"github.com/stretchr/testify/assert"
	"k8s.io/utils/ptr"
)

func TestOptions(t *testing.T) {
	tests := []struct {
		name string
		opts Options
		want Options
	}{
		{
			name: "default options",
			opts: Options{},
			want: Options{
				Format: JSONFormat,
				Writer: os.Stderr,
			},
		},
		{
			name: "custom options",
			opts: Options{
				Level:     ptr.To(slog.LevelDebug),
				Format:    TextFormat,
				Writer:    nil,
				AddSource: true,
			},
			want: Options{
				Level:     ptr.To(slog.LevelDebug),
				Format:    TextFormat,
				Writer:    os.Stderr,
				AddSource: true,
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			a := assert.New(t)
			tt.opts.Default()
			a.Equal(tt.want, tt.opts)
		})
	}
}
