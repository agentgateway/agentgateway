package admin

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"net/http"
	"sort"
	"time"

	"istio.io/istio/pkg/kube/krt"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/controller"
	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
	"github.com/kgateway-dev/kgateway/v2/pkg/version"
)

func RunAdminServer(ctx context.Context, setupOpts *controller.SetupOpts) error {
	// serverHandlers defines the custom handlers that the Admin Server will support
	serverHandlers := getServerHandlers(ctx, setupOpts.KrtDebugger)

	startHandlers(ctx, serverHandlers)

	return nil
}

// use a function for the profile descriptions so that every time the admin page is displayed, it can show
// up-to-date info in the description (e.g. the current log level)
type dynamicProfileDescription func() string

// getServerHandlers returns the custom handlers for the Admin Server, which will be bound to the http.ServeMux
// These endpoints serve as the basis for an Admin Interface for the Control Plane (https://github.com/kgateway-dev/kgateway/issues/6494)
func getServerHandlers(_ context.Context, dbg *krt.DebugHandler) func(mux *http.ServeMux, profiles map[string]dynamicProfileDescription) {
	return func(m *http.ServeMux, profiles map[string]dynamicProfileDescription) {
		addKrtSnapshotHandler("/snapshots/krt", m, profiles, dbg)

		addLoggingHandler("/logging", m, profiles)

		addPprofHandler("/debug/pprof/", m, profiles)

		addVersionHandler("/version", m, profiles)
	}
}

// writeJSON writes a json payload, handling content type, marshaling, and errors
func writeJSON(w http.ResponseWriter, obj any, req *http.Request) {
	w.Header().Set("Content-Type", "application/json")
	pretty := req.URL.Query().Has("pretty")
	indent := ""
	if pretty {
		indent = "    "
	}

	var b []byte
	var err error
	if pretty {
		b, err = json.MarshalIndent(obj, "", indent)
	} else {
		b, err = json.Marshal(obj)
	}
	if err != nil {
		w.WriteHeader(http.StatusInternalServerError)
		_, _ = w.Write([]byte(err.Error()))
		return
	}
	_, err = w.Write(b)
	if err != nil {
		w.WriteHeader(http.StatusInternalServerError)
	}
}

func startHandlers(ctx context.Context, addHandlers ...func(mux *http.ServeMux, profiles map[string]dynamicProfileDescription)) {
	mux := new(http.ServeMux)
	profileDescriptions := map[string]dynamicProfileDescription{}
	for _, addHandler := range addHandlers {
		addHandler(mux, profileDescriptions)
	}
	idx := index(profileDescriptions)
	mux.HandleFunc("/", idx)
	mux.HandleFunc("/snapshots/", idx)
	server := &http.Server{
		Addr:              fmt.Sprintf("localhost:%d", wellknown.KgatewayAdminPort),
		Handler:           mux,
		ReadHeaderTimeout: 10 * time.Second,
	}
	slog.Info("admin server starting", "address", server.Addr)
	go func() {
		err := server.ListenAndServe()
		if err == http.ErrServerClosed {
			slog.Info("admin server closed")
		} else {
			slog.Warn("admin server closed with unexpected error", "error", err)
		}
	}()
	go func() {
		<-ctx.Done()
		if server != nil {
			err := server.Close()
			if err != nil {
				slog.Warn("admin server shutdown returned error", "error", err)
			}
		}
	}()
}

func index(profileDescriptions map[string]dynamicProfileDescription) func(w http.ResponseWriter, r *http.Request) {
	return func(w http.ResponseWriter, r *http.Request) {
		type profile struct {
			Name string
			Href string
			Desc string
		}
		var profiles []profile
		for href, descFunc := range profileDescriptions {
			profiles = append(profiles, profile{
				Name: href,
				Href: href,
				Desc: descFunc(),
			})
		}

		sort.Slice(profiles, func(i, j int) bool {
			return profiles[i].Name < profiles[j].Name
		})

		// Adding other profiles exposed from within this package
		var buf bytes.Buffer
		fmt.Fprintf(&buf, "<h1>Admin Server</h1>\n")
		for _, p := range profiles {
			fmt.Fprintf(&buf, "<h2><a href=\"%s\"}>%s</a></h2><p>%s</p>\n", p.Name, p.Name, p.Desc)
		}
		fmt.Fprintf(&buf, "<h2>About</h2>\n")
		fmt.Fprintf(&buf, "<pre>%s</pre>\n", version.String())
		w.Write(buf.Bytes())
	}
}

// addVersionHandler registers a /version endpoint that exposes build info
func addVersionHandler(path string, mux *http.ServeMux, profiles map[string]dynamicProfileDescription) {
	mux.HandleFunc(path, func(w http.ResponseWriter, r *http.Request) {
		payload := map[string]string{
			"string":  version.String(),
			"version": version.Version,
		}
		writeJSON(w, payload, r)
	})
	profiles[path] = func() string { return "Controller version and commit information" }
}
