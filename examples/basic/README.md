## Basic Example

This example shows how to use the agentgateway to proxy requests to the `everything` tool.

### Running the example

```bash
cargo run -- -f examples/basic/config.yaml
```

Let's look at the config to understand what's going on.

```yaml
binds:
- port: 3000
  listeners:
  - routes:
    - backends:
      - mcp: ...
```

We have a few concepts to understand here:
* `binds` represent each port our server listens on. In this case, we will listen on port 3000.
* `listeners` can contain groups of resources. For our simple case here, we just have 1 listener.
* `routes` can associate advanced routing functionality and traffic policies with traffic. In this case, we just match all traffic and do not apply any policies.
* `backends` contains where the traffic is finally sent to. In this case, we have 1 backend of type `mcp`.

For the MCP backend, we can connect to MCP servers over HTTP or Stdio.
Additionally, we can connect to *multiple* MCP servers, and expose them all as one aggregated server.
In this example, we will connect to one server over Stdio.

```yaml
targets:
- name: everything
  stdio:
    cmd: npx
    args: ["@modelcontextprotocol/server-everything"]
```

> [!TIP]
> If you don't have `npx`, you can also run with docker:
> ```yaml
> stdio:
>   cmd: docker
>   args: ["run", "--rm", "-i", "mcp/everything"]
> ```

When clients connect to the gateway, the `cmd` will be executed to serve the traffic.

Now that we have the gateway running, we can use the [mcpinspector](https://github.com/modelcontextprotocol/inspector) to try it out.
```bash
npx @modelcontextprotocol/inspector
```
Once the inspector is running, it will present the port that it's running on, and then you can navigate to it in your browser.

![Inspector](./img/connect.png)

Agentgatway supports both SSE (served under `/sse`) and streamable HTTP (served under `/mcp`).

Once you're connected, you can navigate to the tools tab and see the available tools.

![Tools](./img/tools.png)

Let's try out one of the tools, like `everything:echo`.

![Echo](./img/echo.png)

That worked! The gateway was able to proxy the request to the `everything` tool and return the response.