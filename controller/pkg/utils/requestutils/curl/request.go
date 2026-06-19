package curl

import "time"

// requestConfig contains the set of options that can be used to configure a curl request
type requestConfig struct {
	method  string
	host    string
	port    int
	headers map[string][]string
	body    string
	path    string
	scheme  string
	timeout time.Duration
}
