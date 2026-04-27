# CEL Functions

The table below lists the CEL functions available in agentgateway.
See the [CEL documentation](https://agentgateway.dev/docs/standalone/latest/reference/cel/) for more information.

## Body Variables

`request.body` and `response.body` expose body bytes to CEL. Capturing these values is bounded by the configured body buffer limit. The default limit is 2 MiB.

When a body variable is used by request-time or response-time expressions, agentgateway eagerly buffers the body so the expression can inspect it before forwarding continues. If the body exceeds the configured limit, buffering fails and the expression cannot read a complete body.

When a body variable is used only by logging, tracing, or other post request/response expressions, agentgateway does not eagerly buffer the body. Instead, it records bytes as the proxy stream is polled and makes the recorded bytes available to the log expression after the stream is done.

If the recorded log-only body exceeds the configured limit, `request.body` or `response.body` contains the truncated prefix up to that limit. The proxied stream itself is still forwarded in full and is not failed due to the logging capture limit. There is currently no CEL field that indicates truncation.

## Functions

| Function           | Purpose                                                                                                                                                                                                                                                                          |
|--------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `json`             | Parse a string or bytes as JSON. Example: `json(request.body).some_field`.                                                                                                                                                                                                       |
| `toJson`           | Convert a CEL value into a JSON string. Example: `toJson({"hello": "world"})`.                                                                                                                                                                                                   |
| `unvalidatedJwtPayload` | Parse the payload section of a JWT without verifying the signature. This splits the token, base64url-decodes the middle segment, and parses it as JSON. Example: `unvalidatedJwtPayload(request.headers.authorization.split(" ")[1]).sub`                          |
| `with`             | CEL does not allow variable bindings. `with` allows doing this. Example: `json(request.body).with(b, b.field_a + b.field_b)`                                                                                                                                                     |
| `variables`        | `variables` exposes all of the variables available as a value. CEL otherwise does not allow accessing all variables without knowing them ahead of time. Warning: this automatically enables all fields to be captured.                                                           |
| `mapValues`        | `mapValues` applies a function to all values in a map. `map` in CEL only applies to map keys.                                                                                                                                                                                    |
| `filterKeys`       | Returns a new map keeping only entries where the key matches the predicate (must evaluate to bool). Example: `{"a":1,"b":2}.filterKeys(k, k == "a")` results in `{"a":1}`. To remove keys, invert the predicate: `m.filterKeys(k, !k.startsWith("x_"))`.                        |
| `merge`            | `merge` joins two maps. Example: `{"a":2,"k":"v"}.merge({"a":3})` results in `{"a":3,"k":"v"}`.                                                                                                                                                                                  |
| `flatten`          | Usable only for logging and tracing. `flatten` will flatten a list or struct into many fields. For example, defining `headers: 'flatten(request.headers)'` would log many keys like `headers.user-agent: "curl"`, etc.                                                           |
| `flattenRecursive` | Usable only for logging and tracing. Like `flatten` but recursively flattens multiple levels.                                                                                                                                                                                    |
| `base64.encode`    | Encodes a string to a base64 string. Example: `base64.encode("hello")`.                                                                                                                                                                                                          |
| `base64.decode`    | Decodes a string in base64 format. Example: `string(base64.decode("aGVsbG8K"))`. Warning: this returns `bytes`, not a `String`. Various parts of agentgateway will display bytes in base64 format, which may appear like the function does nothing if not converted to a string. |
| `sha1.encode`      | Computes the SHA-1 digest of a string or bytes value and returns the lowercase hex string. Example: `sha1.encode("hello")`.                                                                                                                                                     |
| `sha256.encode`    | Computes the SHA-256 digest of a string or bytes value and returns the lowercase hex string. Example: `sha256.encode("hello")`.                                                                                                                                                 |
| `md5.encode`       | Computes the MD5 digest of a string or bytes value and returns the lowercase hex string. Example: `md5.encode("hello")`.                                                                                                                                                        |
| `random`           | Generates a number float from 0.0-1.0                                                                                                                                                                                                                                            |
| `default`          | Resolves to a default value if the expression cannot be resolved. For example `default(request.headers["missing-header"], "fallback")`                                                                                                                                           |
| `coalesce`         | Evaluates expressions from left to right and returns the first one that resolves successfully to a non-null value. `null` values are skipped while searching, but if every expression is either `null` or an error and at least one expression resolved to `null`, the result is `null`. Unlike `default`, it swallows any error from earlier expressions, not just missing keys or undeclared references. Example: `coalesce(request.headers["x-id"], json(request.body).id, "fallback")` |
| `regexReplace`     | Replace the string matching the regular expression. Example: `"/id/1234/data".regexReplace("/id/[0-9]*/", "/id/{id}/")` would result in the string `/id/{id}/data`.                                                                                                              |
| `fail`             | Unconditionally fail an expression.                                                                                                                                                                                                                                              |
| `uuid`             | Randomly generate a UUIDv4                                                                                                                                                                                                                                                       |

## Standard Functions

The following standard functions are available:

* `contains`, `size`, `has`, `map`, `filter`, `all`, `max`, `startsWith`, `endsWith`, `string`, `bytes`, `double`, `exists`, `exists_one`, `int`, `uint`, `matches`.
* Duration/time functions: `duration`, `timestamp`, `getFullYear`, `getMonth`, `getDayOfYear`, `getDayOfMonth`, `getDate`, `getDayOfWeek`, `getHours`, `getMinutes`, `getSeconds`, `getMilliseconds`.
* From the [strings extension](https://pkg.go.dev/github.com/google/cel-go/ext#Strings): `charAt`, `indexOf`, `join`, `lastIndexOf`, `lowerAscii`, `upperAscii`, `trim`, `replace`, `split`, `substring`, `stripPrefix`, `stripSuffix`.
* From the [Kubernetes IP extension](https://kubernetes.io/docs/reference/using-api/cel/#kubernetes-ip-address-library): `isIP("...")`, `ip("...")`, `ip("...").family()`, `ip("...").isUnspecified()`, `ip("...").isLoopback()`, `ip("...").isLinkLocalMulticast()`, `ip("...").isLinkLocalUnicast()`, `ip("...").isGlobalUnicast()`.
* From the [Kubernetes CIDR extension](https://kubernetes.io/docs/reference/using-api/cel/#kubernetes-cidr-library): `cidr("...").containsIP("...")`, `cidr("...").containsIP(ip("..."))`, `cidr("...").containsCIDR(cidr("..."))`, `cidr("...").ip()`, `cidr("...").masked()`, `cidr("...").prefixLength()`.

## Header Views

`request.headers` and `response.headers` expose a header-view object with chainable methods.

Available methods:

| Method       | Purpose                                                                                                                                                                                         |
|--------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| default      | A direct header lookup returns a string when there is one header entry, or a list of raw values when there are multiple entries. Example: `["a,b", "c"] -> ["a,b", "c"]`, while `["z"] -> "z"`. |
| `redacted()` | Replaces sensitive header values with `"<redacted>"`. Useful for usage within logs.                                                                                                             |
| `join()`     | Joins all header entries with `,`. Example: `["a,b", "c"] -> "a,b,c"`.                                                                                                                          |
| `raw()`      | Returns the raw header entries as a list. Example: `["a,b", "c"] -> ["a,b", "c"]`.                                                                                                              |
| `split()`    | Returns all header entries split on `,` as a list. Example: `["a,b", "c"] -> ["a", "b", "c"]`.                                                                                                  |
| `cookie(name)` | Parses the request `Cookie` header and returns the first cookie value for the given name. If the cookie is missing, evaluation fails with `NoSuchKey`.                                         |

Examples:

* `request.headers.redacted().authorization`
* `request.headers.join()["x-forwarded-for"]`
* `request.headers.raw()["set-cookie"]`
* `request.headers.redacted().split()["authorization"]`
* `request.headers.cookie("session")`

`redacted()` can be combined with any of the other methods. `join()`, `raw()`, and `split()` are mutually exclusive; if multiple are chained, the last one wins.

## Query Accessors

`request.pathAndQuery` and `request.uri` expose query-aware string values with chainable methods.

Available methods:

| Method | Purpose |
|--------|---------|
| `query(name)` | Returns a list of all values for the given query parameter. If the parameter is missing, evaluation fails with `NoSuchKey`. |
| `addQuery(name, value)` | Returns a new `pathAndQuery`/`uri` with the query parameter appended. The original value is unchanged. |
| `setQuery(name, value)` | Returns a new `pathAndQuery`/`uri` with all existing values for the key replaced by the provided value. The original value is unchanged. |

Examples:

* `request.pathAndQuery.query("foo") == ["bar", "baz"]`
* `request.uri.query("zap") == ["zip"]`
* `request.pathAndQuery.addQuery("foo", "qux") == "/api/test?foo=bar&foo=baz&foo=qux"`
* `request.uri.setQuery("foo", "qux") == "http://example.com/api/test?foo=qux"`

These values remain usable as strings for standard CEL string functions and comparisons.
