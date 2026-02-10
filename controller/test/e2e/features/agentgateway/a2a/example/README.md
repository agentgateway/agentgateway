# A2A Test Server 

This is a simple example of a server that can be used to test A2A gateways. 

## Setup

### 1. Change the directory of this README.md and clone the repository.

```bash
cd $(git rev-parse --show-toplevel)/test/e2e/features/agentgateway/a2a/example
git clone https://github.com/EmilLindfors/a2a-rs.git
```

### 2. Copy the Dockerfile:

```bash
cp Dockerfile http_server_only.rs a2a-rs/
cd a2a-rs
```

### 3. Build the Docker image

```bash
export REPO="ghcr.io/kgateway-dev"
export IMAGE="test-a2a-server"
export IMAGE_VERSION="0.0.<version>"
docker build -t $REPO/$IMAGE:$IMAGE_VERSION .
```

### 4. Start the container

```shell
docker run -d --name test-local -p 9999:9999 $REPO/$IMAGE:$IMAGE_VERSION
```

## Test:

1. Simple card test:

```bash
curl -H "Authorization: Bearer secret-token" http://localhost:9999/agent-card
```

output

```json
{"name":"Example A2A Agent","description":"An example A2A agent using the a2a-protocol crate","url":"http://localhost:9999","provider":{"organization":"Example Organization","url":"https://example.org"},"version":"1.0.0","documentationUrl":"https://example.org/docs","capabilities":{"streaming":true,"pushNotifications":false,"stateTransitionHistory":false},"defaultInputModes":["text"],"defaultOutputModes":["text"],"skills":[{"id":"echo","name":"Echo Skill","description":"Echoes back the user's message","tags":["echo","respond"],"examples":["Echo: Hello World"],"inputModes":["text"],"outputModes":["text"]}]}
```

2. Full request call:

```bash
curl -X POST -H "Content-Type: application/json" -H "Authorization: Bearer secret-token" \
  -d '{"jsonrpc":"2.0","id":"2","method":"tasks/send","params":{"id":"task-123","message":{"kind":"message","messageId":"test-456","parts":[{"kind":"text","text":"Hello task"}],"role":"user"}}}' \
  http://localhost:9999/
  ```

  ```output
{"id":"2","jsonrpc":"2.0","result":{"contextId":"default","history":[{"kind":"message","messageId":"test-456","parts":[{"kind":"text","text":"Hello task"}],"role":"user"},{"contextId":"","kind":"message","messageId":"4f3226f5-d644-402a-b01d-d94793346ce1","parts":[{"kind":"text","text":"Echo: Hello task"}],"role":"agent","taskId":"task-123"},{"kind":"message","messageId":"test-456","parts":[{"kind":"text","text":"Hello task"}],"role":"user"},{"contextId":"","kind":"message","messageId":"f1110c19-a516-48e1-a75c-7a5a50b1e09d","parts":[{"kind":"text","text":"Echo: Hello task"}],"role":"agent","taskId":"task-123"},{"kind":"message","messageId":"test-456","parts":[{"kind":"text","text":"Hello task"}],"role":"user"},{"contextId":"","kind":"message","messageId":"a16e4347-c374-4326-8fd2-91c1ba249bdf","parts":[{"kind":"text","text":"Echo: Hello task"}],"role":"agent","taskId":"task-123"}],"id":"task-123","kind":"task","status":{"message":{"contextId":"","kind":"message","messageId":"a16e4347-c374-4326-8fd2-91c1ba249bdf","parts":[{"kind":"text","text":"Echo: Hello task"}],"role":"agent","taskId":"task-123"},"state":"working","timestamp":"2025-10-15T03:20:22.839844199Z"}}}
  ```

When you need to update the `kgateway-repo` push the container.

```bash
docker push $REPO/$IMAGE:$IMAGE_VERSION
```

3. Search locally in this repo and update the version reference to the container in the test yamls.