# Agentgateway MCP

## Version Negotiation

Agentgateway handles traffic from a single downstream client to N upstream servers.
We call this "multiplexing" if N>1.

Version negotiation is how we handle the disparate protocol versions between the clients and servers, and Agentgateway itself.
This is particularly important for 2026-07-28+, which has a very different protocol than the other versions (which are all much more incremental differences).
We will call "Old" before 2026-07-28 and "New" after.

**Assumptions**:
* Clients supporting `2026-07-28` will also support older protocols for the foreseeable future.
* Servers supporting `2026-07-28` will also support older protocols for the foreseeable future.
* Users will have a mix of support for `2026-07-28` across clients and servers for the foreseeable future.

### New clients

A new client will send a request with `server/discover`.
We can respond with:
* DiscoverResponse with `supportedVersions` including `2026-07-28`: declaring we are OK with modern protocol.
* DiscoverResponse with `supportedVersions` not including `2026-07-28`: arguably a spec violation; declares we are not OK with the modern protocol.
  Clients will *probably* fallback to send `initialize`.
* Respond with Unsupported error. Clients will fallback to send `initialize`.

**Single server**: we forward the `server/discover`. We can forward it back as is.
If they support modern protocol, this will be a valid DiscoverResponse.
Otherwise it will be an error, which is what we want: the client will fallback to the older protocol.

> **Compatibility mode**: we may want to support a mode where we do not put the fallback onto the client.
> Instead, we do the downgrade for them as the proxy.
> This is plausible but pretty tricky.
> If the server uses sessions, we would have to wrap each request in a session (`init; send request; delete`).
> This matches how we do the older "stateless" style.
> If the server doesn't use sessions, we would be fine to just forward directly... maybe.
> Technically, the specification requires an initialize even with the older session-less style.
> We could send it once and cache it (including the boolean "are they stateful"), which is probably sufficient
> in practice but theoretically unsound (a server could conditionally use sessions on random requests, etc).
> For now, we do not implement this mode. If we do, we will likely do session wrapping

**Multiple servers**: we forward the `server/discover` to each. We need to merge the result.
We want to create the intersection of supported versions from all servers.
If a server returns an error, we also return that error: this tells the clients "we don't support `2026-07-28`" and they will retry
with `initialize`; `initialize` will do similar negotiation.

`subscriptions/listen` requires Streamable HTTP upstreams. Stdio and SSE use legacy protocol
negotiation, so modern clients fall back before a listen request is forwarded.

For multiplexed `subscriptions/listen`, Agentgateway waits for selected upstream ACKs before
emitting one downstream ACK. The ACK includes only accepted filters; resource URIs keep the
client's `service+` form. Resource subscriptions may span upstreams; each target receives only
the URIs it owns. Later notifications are limited to the filters accepted by their source
upstream.
Like other MCP fanout requests, setup waits for every selected upstream before the gateway can
construct its response.
Listen setup has no separate timeout; when configured, the route request-timeout policy bounds it
before the downstream response begins.

---

With this approach, we should fully support new clients. And, when the client _and_ server are new, we get the optimal
behavior: no fallback requests needed.
For older clients, we do end up with fallback requests (both from client to gateway, and gateway to server).
If clients with _only_ `2026-07-28` support become popular, we will need a compatibility mode.
However, this doesn't seem likely to happen in the short term.

### Old clients

For older clients, they should handle sessions or sessionless servers today.
This means we could return a session, or not, when dealing with new servers -- it's our choice entirely.

> A new server cannot use sessions.
> However, they may have session-like behavior; to use the new spec with a server like this implies modifying the tools
> exposed to take correlation IDs.
> This tool change is a change non-dependent on protocol version.
> Therefore, we do not need any special handling of "Session-ful servers on the new protocol"

The key decision here is whether we stick to an older protocol version for the server side hop, or attempt
to upgrade it for the client.
In both cases, the server is unlikely to use sessions, which means there isn't that much benefit to using the new protocol
aside from avoiding an `initialize` _if we have a [cached server response](#caching-server-responses) for a `server/discover`_
As such, I propose for now we keep the same old protocol.

**Single server**: an older client will send an initialize request. We forward it as is.
This assumes the server supports the old version as well.
When we send an initialize, we will not know if the server is 'old' or 'new'; for our intent it is 'old'.
As such, it will probably return a session and we should retain our existing session logic.

**Multiple servers**: same as above; this ends up being the same flow as before.

## Caching

`2026-07-28` introduces a variety of caching controls on `server/discover`, `tools/list`, and more.

### Caching server responses

A server can tell us a resource can be cached, and we can optionally cache it.
There are two scopes: `public` and `private`.

I believe that, unless further evidence of ecosystem trends, `private` is essentially identical to "do no cache":
we have no way to know what the _private scope_ of the request is, so we ought to assume the worst.

However, with `public` we are free to cache for the provided TTL.

A primary concern around skipping requests (by reading from a cache) is that we may be caching the payload
but not the HTTP headers. In particular, around authorization failures which may include `www-authenticate` and other
important headers.
However, if we are caching it implies we got a 200, so that aspect is out. Additionally, if the 200 headers
were relevant to the response (which seems atypical), it is assumed the server will disable caching.

As such, I believe it is safe to cache server responses.
However, this is a performance optimization not required for correctness.

### Configuring caching in agentgateway responses

Whether agentgateway can tell clients to cache is a different story.
Agentgateway may apply various policies to traffic, both at the MCP level but also at all other levels (ext auth, ext proc, etc).
We don't know if something is cacheable.

If we have authorization rules which take request attributes into account, we definitely do not know.
Returning `private` scope is also not sufficient, because the client will not have any idea what the true scoping is:
a users authorization rule could be `only allow request on odd minutes` as a silly example.

As such, I believe the best option for us is to strictly disable caching of proxied results.
This includes sanitizing responses we get to turn off caching.

It is possible in the future we can enable caching on an opt-in basis.
