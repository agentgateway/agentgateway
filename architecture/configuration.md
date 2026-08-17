# Configuration

Agentgateway has three forms of configuration:

## Static Configuration

**Static configuration** is set exactly once early in the process lifecycle.
This is set by environment variables or a YAML/JSON configuration (typically a file, but can also be passed as inline bytes into the command line).
This information is really about global settings like logging configurations, ports to use, etc.
All routing, policies, backends, etc are not configurable here.

## Local Configuration

**Local configuration** is configured via a file (YAML/JSON) and can define the full feature set of agentgateway (backends, routes, policies, etc).
The local configuration uses a file watch to dynamically reload changes.
The local configuration will translate into a shared (with [XDS](#xds-configuration)) internal representation (IR) that is used by the proxy at runtime.

In some cases, the IR and the local configuration are identical. In other cases, there are trivial re-mappings to make the usage more ergonomic.
Others are more broad differences, allow things like fetching JWKS from URLs, or creating a backend + policy with a simple expression like `host: https://example.com`.

## XDS Configuration

**XDS configuration** allows the proxy to be configured by a remote control plane. 
We use the [XDS Transport Protocol](https://www.envoyproxy.io/docs/envoy/latest/api-docs/xds_protocol), but do not use the Envoy types (Listener, Cluster, etc), and instead use [purpose-built types](../crates/agentgateway/proto/resource.proto).
Like the local configuration, these map into the same shared IR.

Unlike the local configuration, the XDS translation will not do things like fetching from URLs/files, etc, and is optimized around being simple and efficient rather than easy for humans.

A critical design philosophy for the APIs is to maintain a nearly direct mapping of user facing APIs to XDS to IR.
This simplifies operations (its easy to understand the configuration when it closely maps to the APIs the human created), but also importantly performance.
Additionally, the control plane is greatly simplified as most of the translations are trivial mechanical operations rather than complex joins.

For example, consider a user facing API that allows a user to globally configure the TLS cipher suites.
In Envoy, the user changing this 1 field would fan out to every Cluster need to change; this is expensive.
In agentgateway, instead we would have a Policy that targets a Bind.
This means the user changing 1 field will only need to change 1 small protobuf message.

Another similar example is routes. In Envoy, listeners point to a list of routes.
Changing one route requires updating all of the routes **even with delta xDS**.
This route list can be multiple megabytes, so each time the user changes 1 small field (such as a weight on a route), multiple MBs need to be fanned out to each proxy.
In agentgateway, the cardinality of protobuf resources mirrors the user API.

* One `HTTPRoute` rule maps to one agentgateway `Route` (rather than a list of all routes)
* One `Pod` maps to one `Workload` (rather than a list of all endpoints for a service)

This is generally achieved by having resources point up to their parent, rather than the parent containing a list of children.
For example, a route references which listener its a part of.

For policies, similar philosophies are applied.
Policies are generally applied to a Gateway/Listener/HTTPRoute/HTTPRouteRule, with merging semantics (some types apply to Backend instead, though).
A naive approach would have the control plane flatten these down to the lowest type (HTTPRouteRule) which has the fanout problem.
In agentgateway, the control plane will instead send all of the policies as-is with a reference to where they apply.
The precedence/merging of policies is handled at runtime.
