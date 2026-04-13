# HTTP Connection Pool

This crate provides a connection pool for HTTP requests.
In the Rust/hyper ecosystem, typically [`hyper-util`](https://github.com/hyperium/hyper-util/) is used for a pooling client.

However, this client has a major limitation: there is only 1 HTTP2 connection per key.
This is the primary motivation for our own crate.

## Implementation

### Background

Generally, HTTP1 pooling is fairly simple.
Each connection is assigned to a single request at once.
Once a request is completed, it is returned to the idle pool.
When a request is made, the idle pool is checked for a connection; if there was none, a new connection is created.

However, with HTTP2 things are more challenging.
Each connection can handle multiple concurrent requests ("streams").
The number of streams is limited by the `max_concurrent_streams` setting.
This is negotiated during the HTTP/2 `SETTINGS` handshake, which introduces a number of problems.

For HTTPS, we negotiate the protocol using ALPN. As such, we don't know if it will be HTTP1 or HTTP2 until the TLS handshake is complete.
If it is HTTP2 (whether HTTPS or not), we still don't know the `max_concurrent_streams` once the connection is established.
This value is sent from the server in the initial `SETTINGS` frame; until this happens, the value is assumed to be infinite.
However, sending an unbounded number of streams is not a good idea -- once the `SETTINGS` frame does arrive, which will surely be less-than-infinite, our excessive streams will be closed.
Waiting for the `SETTINGS` frame is a plausible solution but one that would compromise performance, and not one I have seen in practice.

Fortunately, the specification *suggests* a minimum `max_concurrent_streams` value of 100. In practice,
any server with a value lower than this is likely to hit issues in production, so is unlikely (see below for more).

### High-level flow

At a high level, when a request is made we consult the pool which can return three possibilities:
1. Checkout an existing connection. This is the happy-path, where an idle (or active HTTP2 connection with spare capacity) is found.
2. Create a new connection. There was no suitable connection in the pool, so we need to create one.
3. Wait for a connection to become available. There are no idle connections, but there are some *in progress* ones that we will wait for. This is a valid path for HTTP/2.

### Creating new connections

For both case (2) and (3), the caller will wait to be notified.
The waiting is a FIFO queue; this means that in the "create new connection case", connections may be "mixed" across concurrent callers.
That is, while the caller is expected to start a connection establishment, it's not necessarily *their* connection; connections are considered fungible.

In progress connections have a concept of the `expected_capacity` that they can handle.
For HTTP/1.1 this is always 1, while for HTTP/2 this is 100.
If we don't know the protocol yet (due to ALPN), we classify it as `Auto` which also uses `100`.

This means that if we open 101 non-HTTP/1.1 requests, we should get 2 "create new connection" calls, and 99 "wait for connection" calls.
When the connection creation is complete, it is checked into the pool and distributed to waiters based on its actual capacity.

In the happy case, the expected capacity matches the actual capacity, and we fan it out to the 100 waiters.

In the unhappy case, we cache the actual value and fan out to the less-than-100 waiters (typically, this would be 1 for HTTP/1.1). 
This would leave 99 waiters waiting forever; these are instead returned an error that tells them to retry.
On the retry, they should see the cached expected capacity of `1` and open 99 new connections.

### HTTP/2 flow control

Unfortunately, Hyper doesn't expose great visibility/controls into the current/max stream count for a connection.
As such, we build our own on top of it. However, this is only an approximation.
For each request, we increment a counter (`H2Load`) which is then attached to the final `http::Response` body to be decremented when the stream is completed.
However, this is not a perfect solution, as the body could be discarded, and the exact state of the connection may not directly map to the object.

If we are wrong, its not fatal; if the estimated count is too high, we will under-utilize a connection which is fine.
If its too high, we will attempt to send too many streams on a single connection which will block until one frees up.
In typical cases this is acceptable, but may be pretty bad in long-lived request cases.


