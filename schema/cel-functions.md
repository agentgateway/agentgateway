# CEL Functions

The table below lists the CEL functions available in agentgateway.
See the [CEL documentation](https://agentgateway.dev/docs/standalone/latest/reference/cel/) for more information.

## Functions

| Function           | Purpose                                                                                                                                                                                                                                                                          |
|--------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `json`             | Parse a string or bytes as JSON. Example: `json(request.body).some_field`.                                                                                                                                                                                                       |
| `toJson`           | Convert a CEL value into a JSON string. Example: `toJson({"hello": "world"})`.                                                                                                                                                                                                   |
| `with`             | CEL does not allow variable bindings. `with` allows doing this. Example: `json(request.body).with(b, b.field_a + b.field_b)`                                                                                                                                                     |
| `variables`        | `variables` exposes all of the variables available as a value. CEL otherwise does not allow accessing all variables without knowing them ahead of time. Warning: this automatically enables all fields to be captured.                                                           |
| `mapValues`        | `mapValues` applies a function to all values in a map. `map` in CEL only applies to map keys.                                                                                                                                                                                    |
| `merge`            | `merge` joins two maps. Example: `{"a":2,"k":"v"}.merge({"a":3})` results in `{"a":3,"k":"v"}`.                                                                                                                                                                                  |
| `flatten`          | Usable only for logging and tracing. `flatten` will flatten a list or struct into many fields. For example, defining `headers: 'flatten(request.headers)'` would log many keys like `headers.user-agent: "curl"`, etc.                                                           |
| `flattenRecursive` | Usable only for logging and tracing. Like `flatten` but recursively flattens multiple levels.                                                                                                                                                                                    |
| `base64.encode`    | Encodes a string to a base64 string. Example: `base64.encode("hello")`.                                                                                                                                                                                                          |
| `base64.decode`    | Decodes a string in base64 format. Example: `string(base64.decode("aGVsbG8K"))`. Warning: this returns `bytes`, not a `String`. Various parts of agentgateway will display bytes in base64 format, which may appear like the function does nothing if not converted to a string. |
| `random`           | Generates a number float from 0.0-1.0                                                                                                                                                                                                                                            |
| `default`          | Resolves to a default value if the expression cannot be resolved. For example `default(request.headers["missing-header"], "fallback")`                                                                                                                                           |
| `regexReplace`     | Replace the string matching the regular expression. Example: `"/id/1234/data".regexReplace("/id/[0-9]*/", "/id/{id}/")` would result in the string `/id/{id}/data`.                                                                                                              |
| `fail`             | Unconditionally fail an expression.                                                                                                                                                                                                                                              |
| `uuid`             | Randomly generate a UUIDv4                                                                                                                                                                                                                                                       |`

## Standard Functions

The following standard functions are available:

* `contains`, `size`, `has`, `map`, `filter`, `all`, `max`, `startsWith`, `endsWith`, `string`, `bytes`, `double`, `exists`, `exists_one`, `int`, `uint`, `matches`.
* Duration/time functions: `duration`, `timestamp`, `getFullYear`, `getMonth`, `getDayOfYear`, `getDayOfMonth`, `getDate`, `getDayOfWeek`, `getHours`, `getMinutes`, `getSeconds`, `getMilliseconds`.
* From the [strings extension](https://pkg.go.dev/github.com/google/cel-go/ext#Strings): `charAt`, `indexOf`, `join`, `lastIndexOf`, `lowerAscii`, `upperAscii`, `trim`, `replace`, `split`, `substring`, `stripPrefix`, `stripSuffix`.
* From the [Kubernetes IP extension](https://kubernetes.io/docs/reference/using-api/cel/#kubernetes-ip-address-library): `isIP("...")`, `ip("...")`, `ip("...").family()`, `ip("...").isUnspecified()`, `ip("...").isLoopback()`, `ip("...").isLinkLocalMulticast()`, `ip("...").isLinkLocalUnicast()`, `ip("...").isGlobalUnicast()`.
* From the [Kubernetes CIDR extension](https://kubernetes.io/docs/reference/using-api/cel/#kubernetes-cidr-library): `cidr("...").containsIP("...")`, `cidr("...").containsIP(ip("..."))`, `cidr("...").containsCIDR(cidr("..."))`, `cidr("...").ip()`, `cidr("...").masked()`, `cidr("...").prefixLength()`.
