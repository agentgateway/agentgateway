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

The variables available to users is auto-generated into a [JSON schema](../schema/cel.json) and [rendered to markdown](../schema/cel.md).
The (not auto-generated) CEL functions are documented in [cel_functions.rs](../schema/cel-functions.md).
