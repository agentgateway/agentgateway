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
	"context"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/stretchr/testify/assert"
	"k8s.io/utils/ptr"
)

func TestLogging(t *testing.T) {
	tests := []struct {
		name           string
		components     []string
		query          string
		setLevel       map[string]slog.Level
		wantStatusCode int
		wantBody       string
		wantLevels     map[string]slog.Level
	}{
		{
			name:           "only default logger",
			wantStatusCode: http.StatusOK,
			wantLevels: map[string]slog.Level{
				DefaultComponent: GlobalLevel.Level(),
			},
		},
		{
			name:           "update default level to debug",
			query:          "level=debug",
			wantStatusCode: http.StatusOK,
			wantLevels: map[string]slog.Level{
				DefaultComponent: slog.LevelDebug,
			},
		},
		{
			name:           "update all loggers to debug level",
			components:     []string{"c1", "c2", "c3"},
			query:          "level=debug",
			wantStatusCode: http.StatusOK,
			wantLevels: map[string]slog.Level{
				DefaultComponent: slog.LevelDebug,
				"c1":             slog.LevelDebug,
				"c2":             slog.LevelDebug,
				"c3":             slog.LevelDebug,
			},
		},
		{
			name:           "ignore component levels when updating specific logger levels",
			components:     []string{"c1", "c2", "c3"},
			query:          "level=debug&c1=error&c2=warn&c3=trace",
			wantStatusCode: http.StatusOK,
			wantLevels: map[string]slog.Level{
				DefaultComponent: slog.LevelDebug,
				"c1":             slog.LevelDebug,
				"c2":             slog.LevelDebug,
				"c3":             slog.LevelDebug,
			},
		},
		{
			name:           "update default and component levels",
			components:     []string{"c1", "c2", "c3"},
			query:          "default=debug&c1=error&c2=warn&c3=trace",
			wantStatusCode: http.StatusOK,
			wantLevels: map[string]slog.Level{
				DefaultComponent: slog.LevelDebug,
				"c1":             slog.LevelError,
				"c2":             slog.LevelWarn,
				"c3":             LevelTrace,
			},
		},
		{
			name:           "incorrect global log level should error and preserve current level",
			query:          "level=foo",
			wantStatusCode: http.StatusBadRequest,
			wantBody:       "unknown log level foo",
			wantLevels: map[string]slog.Level{
				DefaultComponent: slog.LevelInfo,
			},
		},
		{
			name:           "incorrect component log level should error and preserve current level",
			components:     []string{"c1"},
			query:          "c1=foo",
			wantStatusCode: http.StatusBadRequest,
			wantBody:       "component c1: unknown log level foo",
			wantLevels: map[string]slog.Level{
				"c1": slog.LevelInfo,
			},
		},
		{
			name:           "update default and component levels using SetLevel",
			components:     []string{"c1", "c2", "c3"},
			setLevel:       map[string]slog.Level{"default": slog.LevelDebug, "c1": slog.LevelError, "c2": slog.LevelWarn, "c3": LevelTrace},
			wantStatusCode: http.StatusOK,
			wantLevels: map[string]slog.Level{
				DefaultComponent: slog.LevelDebug,
				"c1":             slog.LevelError,
				"c2":             slog.LevelWarn,
				"c3":             LevelTrace,
			},
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			a := assert.New(t)

			// Reset component levels to default level
			Reset(slog.LevelInfo)

			loggers := map[string]*slog.Logger{DefaultComponent: slog.Default()}
			for _, component := range tc.components {
				logger := New(component)
				a.NotNil(logger)
				loggers[component] = logger
			}

			// Test HTTP handler
			path := "/logging"
			if tc.query != "" {
				path += "?" + tc.query
			}
			req := httptest.NewRequest(http.MethodPost, path, nil)
			w := httptest.NewRecorder()
			HTTPLevelHandler(w, req)
			resp := w.Result()
			a.Equal(tc.wantStatusCode, resp.StatusCode)
			data, err := io.ReadAll(resp.Body)
			a.NoError(err)
			a.NotEmpty(data)
			a.Contains(string(data), tc.wantBody)

			// Test SetLevel
			for component, level := range tc.setLevel {
				err := SetLevel(component, level)
				a.NoError(err)
			}

			for component, level := range tc.wantLevels {
				a.Equal(level, MustGetLevel(component), component)
				a.True(loggers[component].Enabled(context.TODO(), level), component)
			}
		})
	}
}

func TestGetComponentLevels(t *testing.T) {
	a := assert.New(t)

	_ = NewWithOptions("TestGetComponentLevels1", Options{Level: ptr.To(slog.LevelDebug)})
	_ = NewWithOptions("TestGetComponentLevels2", Options{Level: ptr.To(slog.LevelError)})

	got := GetComponentLevels()
	a.Equal(slog.LevelDebug, got["TestGetComponentLevels1"], "TestGetComponentLevels1")
	a.Equal(slog.LevelError, got["TestGetComponentLevels2"], "TestGetComponentLevels2")
}
