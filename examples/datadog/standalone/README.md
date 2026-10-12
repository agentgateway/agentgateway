# Datadog standalone observability

Run agentgateway, HomeHub, and the Datadog Agent with Docker Compose. The Agent
scrapes proxy metrics and receives OTLP/HTTP traces directly from agentgateway.
See the [overview](../README.md) for the manual verification and dashboard.

## Start

Install Docker with Compose v2 and curl. Run commands from this directory:

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

[`versions.env`](versions.env) pins the gateway, Datadog Agent, and HomeHub image.
Only gateway and metrics ports are published, both on IPv4 loopback. HomeHub
and the OTLP receiver remain on the Compose network.

## Verify

```sh
export GATEWAY_URL=http://127.0.0.1:13000
export METRICS_URL=http://127.0.0.1:18520/metrics
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
credentials. If ports are occupied, change the host-side mappings in
`compose.yaml` and the corresponding URLs above.

## Cleanup

```sh
docker compose --env-file versions.env --env-file .env down
```
