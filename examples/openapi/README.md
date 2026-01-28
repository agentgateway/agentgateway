## OpenAPI Example

This example shows how to use the agentgateway to proxy a OpenAPI as a MCP service.

OpenAPI is the current backbone of the internet, and it's a great way to describe your API. However, it's not geared towards agentic use-cases. Using agentgateway, you can seamlessly integrate OpenAPI APIs into your agentic workflows in a secure and scalable way.

This example will show you how to proxy the [Swagger Petstore](https://petstore3.swagger.io) as the example MCP service.

### Running the example

Start agentgateway.

```bash
cargo run -- -f examples/openapi/config.yaml
```

In addition to the [basic](../basic) setup, we have added a new target of type `openapi`:

```yaml
name: openapi
openapi:
  schema:
    file: ./examples/openapi/openapi.json
  host: localhost:8080
```

This will expose each method in the openapi specification as MCP tools, and proxy them to the petstore application (on `localhost:8080`).

Start the Petstore OpenAPI 3 demo app.

```bash
docker compose up
```

Now that we have the gateway and the demo petstore running, we can use the [mcpinspector](https://github.com/modelcontextprotocol/inspector) to try it out.
```bash
npx @modelcontextprotocol/inspector
```

Once the inspector is running, it will present the port that it's running on, and then you can navigate to it in your browser.

![Inspector](./img/connect.png)

Once you're connected, you can navigate to the tools tab and see the available tools.

![Tools](./img/tools.png)

Let's try out one of the tools, like `placeOrder`.

![Petstore](./img/call.png)
