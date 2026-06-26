# Agentgateway MCP

## Version Negotiation

Agentgatewaty handles traffic from a single downstream client to N upstream servers.
We call this "multiplexing" if N>1.

Version negotiation is how we handle the disparate protocol versions between the clients and servers, and Agentgateway itself.
This is particular important for 2026-07-28+, which has a very different protocol than the other versions (which are all much more incremental differences).
We will call "Old" before 2026-07-28 and "New" after.

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

---

With this approach, we should fully support new clients. And, when the client _and_ server are new, we get the optimal
behavior: no fallback requests needed.
For older clients, we do end up with fallback requests (both from client to gateway, and gateway to server).
If clients with _only_ `2026-07-28` support become popular, we will need a compatibility mode.
However, this doesn't seem likely to happen in the short term.

### Old clients

For older clients, they should handle sessions or sessionless servers today.
This means we could return a session, or not, when dealing with new servers -- its our choice entirely.

> A new server cannot use sessions.
> However, they may have session-like behavior; to use the new spec with a server like this implies modifying the tools
> exposed to take correlation IDs.
> This tool change is a change non-dependant on protocol version.
> Therefor, we do not need any special handling of "Session-ful servers on the new protocol"

**Single server**: an older client will send an initialize request