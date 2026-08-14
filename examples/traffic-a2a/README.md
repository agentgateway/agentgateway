## A2A Example

This example shows how to use the agentgateway to proxy A2A requests.

### Running the example

```bash
cargo run -- -f examples/traffic-a2a/config.yaml
```

Let's look at the config to understand what's going on. Like in the [basic](../mcp-basic) setup, we define a `bind` and `listener`.
This time, our backend will not be of type `mcp` and will just be a plain `host`.
The `a2a` policy indicates this traffic is A2A and will be processed accordingly

```yaml
policies:
  # Mark this route as a2a traffic
  a2a: {}
backends:
- host: localhost:9999
```

To test this, we will run a sample `a2a` agent and client.

First, run the server:
```bash
$ git clone https://github.com/a2aproject/a2a-samples
$ cd a2a-samples/samples/python
$ uv run agents/helloworld
```

In another terminal, run the client and send a few messages.

```bash
$ uv run hosts/cli --agent http://localhost:3000
```

Agentgateway will proxy the requests and do a few things.

First, we can directly send a request to agentgateway to the [agent card](http://localhost:3000/.well-known/agent.json) endpoint.
The agent will typically do this automatically, but we will use `curl` to manually look at the card.

```bash
$ curl http://localhost:3000/.well-known/agent.json | jq
{
  "description": "Just a hello world agent",
  "url": "http://localhost:3000",
}
```

You can see the `url` has been rewritten to point back to agentgateway, ensuring future requests do not bypass the gateway.

Additionally, as we send requests we can see A2A specific information in the logs:

```plain
2025-07-03T16:56:34.379262Z     info    request gateway=bind/3000 listener=listener0
    route=route0 endpoint=localhost:9999 src.addr=127.0.0.1:57408
    http.method=POST http.host=localhost http.path=/ http.version=HTTP/1.1 http.status=200
    a2a.method=message/stream a2a.response.outcome=success a2a.result.kind=task
    a2a.task.state=completed duration=2ms
```

If the upstream agent returns a JSON-RPC error with an HTTP 200 status, the log includes the A2A response outcome and error code:

```plain
2025-07-03T16:56:34.379262Z     info    request gateway=bind/3000 listener=listener0
    route=route0 endpoint=localhost:9999 src.addr=127.0.0.1:57408
    http.method=POST http.host=localhost http.path=/ http.version=HTTP/1.1 http.status=200
    a2a.method=message/stream a2a.response.outcome=error
    a2a.response.error_code=-32602 duration=2ms
```
