## Tailscale Authentication

This example shows how to integrate with [Tailscale](https://tailscale.com/) for authentication.
This follows the approach laid out in the [Tailscale Authentication for NGINX](https://tailscale.com/blog/tailscale-auth-nginx) documentation.
However, unlike with NGINX, integration with Agentgateway does not require an intermediate component.

### Running the example

First, ensure Tailscale is running on the same machine as Agentgateway.

Then, run agentgateway:

```bash
cargo run -- -f examples/tailscale-auth/config.yaml
```

Next, we can send some example requests:

```bash
# Request not over Tailscale
$ curl localhost:3000
no match for IP:port
# Request over Tailscale
$ curl 100.x.x.x:3000
Hello world!
```

On the successful request, we can see the Tailscale attributes included in our logs:
```
request listener=default route=application http.method=GET http.path=/ http.version=HTTP/1.1 http.status=200 
    protocol=http duration=2ms tailscale.node="my.name.ts.net." tailscale.email="something@example.com"
```
