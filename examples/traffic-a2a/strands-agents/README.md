## Strands Agent and AgentGateway

Example of [Strands Agent](https://strandsagents.com/) and A2A protocol using agentgateway.

## Set up the agentgateway

Create an agentgateway that proxies requests to the strands agent

1. Create a listener and target configuration for your agentgateway. In this example, the agentgateway is configured as follows:
   * **Listener**: An HTTP listener is configured for the A2A protocol and exposed on port 3000.
   * **Backend**: The agentgateway targets a backend on your localhost port 9999, which you create in a subsequent step.
   ```yaml
    binds:
    - port: 3000
    listeners:
    - routes:
        - policies:
            # Mark this route as a2a traffic
            a2a: {}
        backends:
        - host: localhost:9999
   ```

2. Create the agentgateway.
   ```sh
   agentgateway -f ../config.yaml
   ```

## Set up an Strands Agent

3. Run the Calculator Strands Agent.

   ```sh
   uv run .
   ```

## Verify the A2A connection

1. Clone the A2A sample repository.
   ```sh
   git clone https://github.com/a2aproject/a2a-samples.git
   ```

2. Navigate to the `samples/python/hosts/cli` directory.

   ```sh
   cd a2a-samples/samples/python/hosts/cli
   ```

3. Run the client and send several test messages to the Calculator agent.

    ```sh
    uv run . --agent http://localhost:3000
    ```

    Example output:

    ```
    ======= Agent Card ========
    {"capabilities":{"streaming":true},"defaultInputModes":["text"],"defaultOutputModes":["text"],"description":"A calculator agent that can perform basic arithmetic operations.","name":"Calculator Agent","protocolVersion":"0.2.6","skills":[{"description":"Calculator powered by SymPy for comprehensive mathematical operations...","id":"calculator","name":"calculator","tags":[]}],"url":"http://localhost:3000","version":"0.0.1"}
    =========  starting a new task ========

    What do you want to send to the agent? (:q or quit to exit):
    ```

    Type a sample message, such as `10 times 10`, and then send the message by pressing enter.

3. In another terminal tab, manually send a request to the [agent card endpoint](http://localhost:3000/.well-known/agent.json) through agentgateway.

   ```sh
   curl -s http://localhost:3000/.well-known/agent.json | jq
   ```

   Example output: Notice that the `url` field is rewritten to point to the agentgateway.

    ```json
    {
    "capabilities": {
        "streaming": true
    },
    "defaultInputModes": [
        "text"
    ],
    "defaultOutputModes": [
        "text"
    ],
    "description": "A calculator agent that can perform basic arithmetic operations.",
    "name": "Calculator Agent",
    "protocolVersion": "0.2.6",
    "skills": [
        {
        "description": "Calculator powered by SymPy for comprehensive mathematical operations.\n\nThis tool provides advanced mathematical functionality through multiple operation modes,\nincluding expression evaluation, equation solving, calculus operations (derivatives, integrals),\nlimits, series expansions, and matrix operations. Results are formatted with appropriate\nprecision and can be displayed in scientific notation when needed....",
        "id": "calculator",
        "name": "calculator",
        "tags": []
        }
    ],
    "url": "http://localhost:3000",
    "version": "0.0.1"
    }
    ```

4. In the tab where the agentgateway is running, verify that you see request logs from your client query to the Calculator agent, such as the following example.

   ```text
   2025-07-10T18:10:46.547567Z	info	request	gateway=bind/3000 listener=listener0 route=route0 endpoint=localhost:9999 src.addr=[::1]:59257 http.method=POST http.host=localhost http.path=/ http.version=HTTP/1.1 http.status=200 a2a.method=message/stream duration=3ms
   ```
