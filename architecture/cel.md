# CEL

Agentgateway extensively uses [CEL](https://cel.dev/) throughout the project.
CEL is an expression language that can evaluate user-defined (at runtime) expressions based on incoming requests.

A simple example of an expression that could be used for MCP authorization: `jwt.sub == "test-user" && mcp.tool.name == "add"`.

While CEL is not as powerful as alternatives like Lua or WASM, it is pretty fast and good enough for many use cases.

Agentgateway currently uses CEL for:
* Defining attributes to include in logs/traces. For example `user_agent: 'request.headers["user-agent"]'`.
* Modifying HTTP headers and bodies.
* Authorization policies
* Selecting what aspects of a request to rate limit based on.

## Architecture

CEL allows evaluating expressions in a user-defined _context_ (here, users are agentgateway developers, not end-users).
The context includes custom variables and functions.
Agentgateway exposes a variety of variables based on the request context, as well as custom functions.

CEL expressions are used throughout the request processing pipeline, which means we may have an expression run before or after we have information available.
For example, a request header transformation runs before we have the `response` available, but logging fields runs after the `request` has been discarded.
For cases where the data is not yet available, these variables are just not available to the expression.
For fields that are _no longer available_, agentgateway will dynamically decide whether to keep the data around based on whether any expression depends on it.

This is done with the `ContextBuilder`.
During CEL policy parsing (which happens on configuration change, not each request), we extract which variables are referenced in the expression.
This handles _most_ cases, though its possible to have false negatives (for example, `request.body` is fine but `request["body"]` is not).
During request processing, `ContextBuilder.with_xxx()` is called to conditionally add fields into the context.
For example, if an expression requires `request.body` we will store a copy of the body in the context, but if no expression requires it the data will be ignored.
This ensures users only pay for what they use.
This is critical for `body`, which is super expensive, but also useful for `headers` which can be costly as well.

The variables available to users is auto-generated into a [JSON schema](../schema/cel.json) and [rendered to markdown](../schema/README.md#cel-context).
Additionally, custom functions are available:

| Function           | Purpose                                                                                                                                                                                                                                                                         |
|--------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `json`             | Parse a string or bytes as JSON. Example: `json(request.body).some_field`.                                                                                                                                                                                                      |
| `with`             | CEL does not allow variable bindings. `with` alows doing this. Example: `json(request.body).with(b, b.field_a + b.field_b)`                                                                                                                                                     |
| `variables`        | `variables` exposes all of the variables available as a value. CEL otherwise does not allow accessing all variables without knowing them ahead of time. Warning: this automatically enables all fields to be captured.                                                          |
| `mapValues`        | `mapValues` applies a function to all values in a map. `map` in CEL only applies to map keys.                                                                                                                                                                                   |
| `merge`            | `merge` joins two maps. Example: `{"a":2,"k":"v"}.merge({"a":3})` results in `{"a":3,"k":"v"}`.                                                                                                                                                                                 |
| `flatten`          | Usable only for logging and tracing. `flatten` will flatten a list or struct into many fields. For example, defining `headers: 'flatten(request.headers)'` would log many keys like `headers.user-agent: "curl"`, etc.                                                          |
| `flattenRecursive` | Usable only for logging and tracing. Like `flatten` but recursively flattens multiple levels.                                                                                                                                                                                   |
| `base64Encode`     | Encodes a string to a base64 string. Example: `base64Encode("hello")`.                                                                                                                                                                                                          |
| `base64Decode`     | Decodes a string in base64 format. Example: `string(base64Decode("aGVsbG8K"))`. Warning: this returns `bytes`, not a `String`. Various parts of agentgateway will display bytes in base64 format, which may appear like the function does nothing if not converted to a string. |
| `random`           | Generates a number float from 0.0-1.0                                                                                                                                                                                                                                           |
| `default`          | Resolves to a default value if the expression cannot be resolved. For example `default(request.headers["missing-header"], "fallback")`                                                                                                                                          |
| `regexReplace`     | Replace the string matching the regular expression. Example: `"/id/1234/data".regexReplace("/id/[0-9]*/", "/id/{id}/")` would result in the string `/id/{id}/data`.                                                                                                             |

Additionally, the following standard functions are available:
* `contains`, `size`, `has`, `map`, `filter`, `all`, `max`, `startsWith`, `endsWith`, `string`, `bytes`, `double`, `exists`, `exists_one`, `int`, `uint`, `matches`.
* Duration/time functions: `duration`, `timestamp`, `getFullYear`, `getMonth`, `getDayOfYear`, `getDayOfMonth`, `getDate`, `getDayOfWeek`, `getHours`, `getMinutes`, `getSeconds`, `getMilliseconds`.
* From the [strings extension](https://pkg.go.dev/github.com/google/cel-go/ext#Strings): `charAt`, `indexOf`, `join`, `lastIndexOf`, `lowerAscii`, `upperAscii`, `trim`, `replace`, `split`, `substring`.

