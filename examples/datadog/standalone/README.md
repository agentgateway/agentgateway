# Datadog standalone observability

Configure an existing standalone agentgateway with HomeHub and the Datadog
Agent supplied by Docker Compose. The Agent
scrapes proxy metrics and receives OTLP/HTTP traces directly from agentgateway.
See the [overview](../README.md) for the manual verification and dashboard.

## Prerequisites

- A running [standalone agentgateway](https://agentgateway.dev/docs/standalone/latest/documentation/setup/).
  This example was tested with agentgateway v1.6.0.
- Docker with Compose v2, curl, and a Datadog ingestion API key and site.
- Available loopback ports `18080` for HomeHub and `4318` for OTLP ingestion.

The sample configuration assumes agentgateway runs on the Docker host and serves
metrics on port 15020. For a container or remote gateway, use backend addresses
reachable from it and update `openmetrics.yaml` with a metrics endpoint reachable
from the Datadog Agent.

## Start the Supporting Services

Run commands from this directory:

```sh
cd examples/datadog/standalone
```

Create an ignored `.env` file with your Datadog ingestion API key and site:

```dotenv
DD_API_KEY=replace-with-your-datadog-api-key
DD_SITE=us3.datadoghq.com
```

Use your organization's [Datadog site](https://docs.datadoghq.com/getting_started/site/).
For US1, use `datadoghq.com`. Enable Agent Observability in the organization to
inspect LLM traces. Keep the file private:

```sh
chmod 600 .env
docker compose --env-file versions.env --env-file .env up -d
```

[`versions.env`](versions.env) pins the Datadog Agent and HomeHub image. Their
published ports bind only to IPv4 loopback.

## Configure agentgateway

Merge [config.yaml](config.yaml) into your existing gateway configuration using
the [configuration update guide](https://agentgateway.dev/docs/standalone/latest/documentation/setup/update/).
It adds the HomeHub model and direct OTLP/HTTP export to Datadog. Adjust the
listener port and backend addresses to match your deployment.

## Verify

```sh
export GATEWAY_URL=http://127.0.0.1:3000
export METRICS_URL=http://127.0.0.1:15020/metrics
```

Follow the [manual verification](../README.md#verify-collection), then run
successive OpenMetrics checks:

```sh
docker compose --env-file versions.env --env-file .env \
  exec datadog agent check openmetrics --check-rate
```

The check should report metric samples tagged `component:proxy`. An OpenMetrics
service check proves reachability; verify actual metrics and LLM traces in
Datadog using the overview.

## Troubleshoot

```sh
docker compose --env-file versions.env --env-file .env ps
docker compose --env-file versions.env --env-file .env exec datadog agent status
curl --fail "${METRICS_URL}"
```

If telemetry is absent, check the API key, site, and Agent health. Send another
request between scrape intervals, allow time for intake, and search without
extra filters. Do not share `.env` or expanded Compose configuration containing
credentials. If supporting-service ports are occupied, update their mappings in
`compose.yaml` and the backend addresses in `config.yaml`.

## Cleanup

Remove the example HomeHub and tracing settings from your gateway configuration,
then stop the supporting services:

```sh
docker compose --env-file versions.env --env-file .env down
```
