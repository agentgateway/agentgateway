package admin

import (
	"net/http"
	"net/http/pprof"
)

func addPprofHandler(path string, mux *http.ServeMux, profiles map[string]dynamicProfileDescription) {
	mux.HandleFunc(path, pprof.Index)
	mux.HandleFunc(path+"cmdline", pprof.Cmdline)
	mux.HandleFunc(path+"profile", pprof.Profile)
	mux.HandleFunc(path+"symbol", pprof.Symbol)
	mux.HandleFunc(path+"trace", pprof.Trace)

	profiles[path] = func() string {
		return `PProf related things:<br/>
	<a href="` + path + `goroutine?debug=2">full goroutine stack dump</a>
	`
	}
}
