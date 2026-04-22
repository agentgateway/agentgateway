## Route Delegation Example

This example shows how to use route delegation to compose routes hierarchically.
A parent route can delegate to a **route group** instead of a backend, and the route group
contains child routes that handle more specific path prefixes.

This models the Kubernetes Gateway API pattern where a parent `HTTPRoute` delegates
to child `HTTPRoute` resources in different namespaces.

### Concepts

```
Request: /anything/team1/foo
         |
         v
   Parent Route (matches /anything/team1)
         |
         | delegates to routeGroup: team1-routes
         v
   Route Group "team1-routes"
         |
         | selects best matching child route
         v
   Child Route "child-foo" (matches /anything/team1/foo)
         |
         v
   Backend: 127.0.0.1:8080
```

* **Route delegation**: A route's backend can reference a `routeGroup` instead of a direct backend.
  When the parent route matches, the gateway looks up the route group and selects the best
  matching child route within it.
* **Route groups**: Defined at the top level under `routeGroups`, each group contains a set of
  child routes. Child routes use the same matching logic as regular routes (path, headers, etc).
* **Child route matching**: Each child route independently matches against the request. A child
  route must match a subset of the parent route's path to ever be reached.
* **Cycle detection**: Route groups can delegate to other route groups (multi-level delegation),
  but cycles are detected at runtime and return errors.
* **Policy composition**: Policies from each route along the delegation chain are combined, with
  later (child) routes taking precedence over earlier (parent) routes.

### Running the example

Start some upstream servers to receive traffic:

```bash
# Terminal 1 - team1/foo backend
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers()
        self.wfile.write(b'team1/foo backend (port 8080)\n')
HTTPServer(('', 8080), H).serve_forever()
"

# Terminal 2 - team1/bar backend
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers()
        self.wfile.write(b'team1/bar backend (port 8081)\n')
HTTPServer(('', 8081), H).serve_forever()
"

# Terminal 3 - team2 backend
python3 -c "
from http.server import HTTPServer, BaseHTTPRequestHandler
class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers()
        self.wfile.write(b'team2 backend (port 8082)\n')
HTTPServer(('', 8082), H).serve_forever()
"
```

Run the gateway:

```bash
cargo run -- -f examples/delegation/config.yaml
```

Test the routes:

```bash
# Matches parent-team1 -> delegates to team1-routes -> matches child-foo -> backend on port 8080
curl 127.0.0.1:3000/anything/team1/foo

# Matches parent-team1 -> delegates to team1-routes -> matches child-bar -> backend on port 8081
curl 127.0.0.1:3000/anything/team1/bar

# Matches team2-direct -> backend on port 8082
curl 127.0.0.1:3000/anything/team2

# No matching route
curl 127.0.0.1:3000/other
```