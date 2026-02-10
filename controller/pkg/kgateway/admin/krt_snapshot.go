package admin

import (
	"net/http"

	"istio.io/istio/pkg/kube/krt"
)

func addKrtSnapshotHandler(path string, mux *http.ServeMux, profiles map[string]dynamicProfileDescription, dbg *krt.DebugHandler) {
	mux.HandleFunc(path, func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, dbg, r)
	})
	profiles[path] = func() string { return "KRT Snapshot" }
}
