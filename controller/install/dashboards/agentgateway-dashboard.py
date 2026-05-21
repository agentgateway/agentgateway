#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.13"
# dependencies = ["grafana-foundation-sdk==1769699998!10.1.0"]
# ///

import argparse
from grafana_foundation_sdk.builders import common
from grafana_foundation_sdk.builders.dashboard import (
  Dashboard,
  DatasourceVariable,
  QueryVariable,
  Row,
)
from grafana_foundation_sdk.builders.prometheus import Dataquery as PrometheusQuery
from grafana_foundation_sdk.builders.timeseries import Panel as Timeseries
from grafana_foundation_sdk.cog.encoder import JSONEncoder
from grafana_foundation_sdk.models.common import (
  GraphGradientMode,
  LegendDisplayMode,
  VisibilityMode,
)
from grafana_foundation_sdk.models.dashboard import (
  DashboardCursorSync,
  DataSourceRef,
  VariableRefresh,
  VariableSort,
  VariableHide,
)
from grafana_foundation_sdk.models.resource import (
  Manifest as SDKManifest,
  Metadata,
)


def new_dashboard(name: str, uid: str) -> Dashboard:
  return (
    Dashboard(name)
    .uid(uid)
    .tooltip(DashboardCursorSync.CROSSHAIR)
    .refresh("15s")
    .time("now-30m", "now")
    .with_variable(DatasourceVariable("datasource").type("prometheus"))
  )


def prometheus() -> DataSourceRef:
  return DataSourceRef(type_val="prometheus", uid="$datasource")


def base_timeseries(title: str, desc: str = "") -> Timeseries:
  panel = (
    Timeseries()
    .title(title)
    .interval("5s")
    .legend(
      common.VizLegendOptions()
      .display_mode(LegendDisplayMode.TABLE)
      .calcs(["last", "max", "mean"])
      .show_legend(True)
      .sort_by("Last")
      .sort_desc(True)
    )
    .fill_opacity(10)
    .height(10)
    .show_points(VisibilityMode.NEVER)
    .gradient_mode(GraphGradientMode.OPACITY)
    .datasource(prometheus())
  )
  if desc:
    panel = panel.description(desc)
  return panel


def seconds_timeseries(title: str, desc: str = "") -> Timeseries:
  return base_timeseries(title, desc).unit("s")


def tps_timeseries(title: str, desc: str = "") -> Timeseries:
  return base_timeseries(title, desc).unit("tps")


def rps_timeseries(title: str, desc: str = "") -> Timeseries:
  return base_timeseries(title, desc).unit("reqps")


def bytes_timeseries(title: str, desc: str = "") -> Timeseries:
  return base_timeseries(title, desc).unit("bytes")


def query(expr: str, legend: str) -> PrometheusQuery:
  return raw_query(expr).legend_format(legend)


def raw_query(expr: str) -> PrometheusQuery:
  return PrometheusQuery().expr(expr)


def prom_sum(expr: str, by: list[str] | None = None) -> str:
  by = by or []
  if len(by) == 0:
    return f"sum ({expr})"
  return f"sum by ({','.join(by)}) ({expr})"


def irate(expr: str) -> str:
  return f"irate({expr}[$__rate_interval])"


def rate(expr: str) -> str:
  return f"rate({expr}[$__rate_interval])"


def quantile(quantile_value: str, expr: str) -> str:
  return f"histogram_quantile({quantile_value}, {expr})"


def labels(metric_name: str, label_values: dict[str, str]) -> str:
  prom_labels = []
  for key, value in label_values.items():
    if value.startswith("!~"):
      prom_labels.append(f'{key}!~"{value[2:]}"')
    elif value.startswith("~"):
      prom_labels.append(f'{key}=~"{value[1:]}"')
    elif value.startswith("!"):
      prom_labels.append(f'{key}!="{value[1:]}"')
    else:
      prom_labels.append(f'{key}="{value}"')

  return f"{metric_name}{{{','.join(prom_labels)}}}"


def filtered_sum(expr: str, by: list[str]) -> str:
  return prom_sum(
    rate(
      labels(
        expr,
        {"namespace": "~$namespace", "gateway": "~$gateway"},
      ),
    ),
    by=by,
  )


def pod_metric(metric: str) -> str:
  return labels(
    metric,
    {
      "namespace": "~$namespace",
      "gateway_networking_k8s_io_gateway_name": "~$gateway_name",
      "pod": "~$pod",
    },
  )


def filtered_histogram_quantile(
  quantile_value: str,
  metric: str,
  by: list[str],
) -> str:
  expr = quantile(quantile_value, filtered_sum(metric, by=["le", *by]))
  return f"({expr}) == ({expr})"


def filter_pods(expr: str) -> str:
  return (
    f"{expr} "
    f"* on(pod, namespace) group_left(gateway_networking_k8s_io_gateway_name) "
    f"agentgateway_build_info{{namespace=~\"$namespace\",gateway_networking_k8s_io_gateway_name=~\"$gateway_name\"}}"
  )


def add_targets(panel, targets: list[tuple[str, str]]):
  for expr, legend in targets:
    panel = panel.with_target(query(expr, legend))
  return panel


def build_dashboard() -> Dashboard:
  dpmem = bytes_timeseries("Memory").with_target(
    query(
      filter_pods(
        prom_sum(
          labels(
            "container_memory_working_set_bytes",
            {"image": "", "namespace": "~$namespace"},
          ),
          by=["pod", "namespace"],
        ),
      ),
      "{{namespace}}/{{pod}}",
    )
  )
  dpcpu = base_timeseries("CPU").with_target(
    query(
      prom_sum(
        filter_pods(
          irate(
            labels(
              "container_cpu_usage_seconds_total",
              {"image": "", "namespace": "~$namespace"},
            ),
          )
        ),
        by=["pod", "namespace"],
      ),
      "{{namespace}}/{{pod}}",
    )
  )
  requests = rps_timeseries("Requests (by Pod)").with_target(
    query(
      filtered_sum("agentgateway_requests_total", by=["pod", "namespace"]),
      "{{namespace}}/{{pod}}",
    )
  )
  requests_by_gateway = rps_timeseries("Requests (by Gateway)").with_target(
    query(
      filtered_sum("agentgateway_requests_total", by=["gateway"]),
      "{{gateway}}",
    )
  )
  request_by_status = rps_timeseries("Requests (by Status)").with_target(
    query(
      filtered_sum("agentgateway_requests_total", by=["gateway", "status"]),
      "{{gateway}}: {{status}}",
    )
  )
  request_by_reason = rps_timeseries("Requests (by Reason)").with_target(
    query(
      filtered_sum("agentgateway_requests_total", by=["gateway", "reason"]),
      "{{gateway}}: {{reason}}",
    )
  )

  def llm_median(name: str, q: str) -> Timeseries:
    return seconds_timeseries(name).with_target(
      query(
        quantile("0.5", filtered_sum(q, by=["le", "gateway", "gen_ai_request_model"])),
        "{{gateway}}: {{gen_ai_request_model}}",
      ),
    )

  llm_ttft = llm_median(
    "Time To First Token", "agentgateway_gen_ai_server_time_to_first_token_bucket"
  )
  llm_time = llm_median(
    "Request Time", "agentgateway_gen_ai_server_request_duration_bucket"
  )
  llm_tps = tps_timeseries("Tokens Per Second").with_target(
    query(
      "1 / "
      + quantile(
        "0.5",
        filtered_sum(
          "agentgateway_gen_ai_server_time_per_output_token_bucket",
          by=["le", "gateway", "gen_ai_request_model"],
        ),
      ),
      "{{gateway}}: {{gen_ai_request_model}}",
    ),
  )

  llm_tokens = base_timeseries("Token Consumption").with_target(
    query(
      filtered_sum(
        "agentgateway_gen_ai_client_token_usage_sum",
        by=["gen_ai_token_type", "gen_ai_request_model", "gateway"],
      ),
      "{{gateway}}: {{gen_ai_request_model}} ({{gen_ai_token_type}})",
    )
  )

  mcp_tools = rps_timeseries("MCP Calls (by method)").with_target(
    query(
      filtered_sum("agentgateway_mcp_requests_total", by=["gateway", "method"]),
      "{{gateway}}: {{method}}",
    )
  )
  mcp_list = rps_timeseries("Tool Calls (by tool)").with_target(
    query(
      prom_sum(
        rate(
          labels(
            "agentgateway_mcp_requests_total",
            {
              "namespace": "~$namespace",
              "gateway": "~$gateway",
              "method": "tools/call",
            },
          ),
        ),
        by=["gateway", "server", "resource"],
      ),
      "{{gateway}}: {{server}}/{{resource}}",
    )
  )

  latency_by_route = add_targets(
    seconds_timeseries("Latency by Route"),
    [
      (
        filtered_histogram_quantile(
          "0.50", "agentgateway_request_duration_seconds_bucket", ["gateway", "route"]
        ),
        "{{gateway}}: {{route}} p50",
      ),
      (
        filtered_histogram_quantile(
          "0.95", "agentgateway_request_duration_seconds_bucket", ["gateway", "route"]
        ),
        "{{gateway}}: {{route}} p95",
      ),
      (
        filtered_histogram_quantile(
          "0.99", "agentgateway_request_duration_seconds_bucket", ["gateway", "route"]
        ),
        "{{gateway}}: {{route}} p99",
      ),
    ],
  )

  xds_messages = base_timeseries("XDS Messages by Type").with_target(
    query(
      prom_sum(rate("agentgateway_xds_message_total"), by=["url"]),
      "{{url}}",
    )
  )
  xds_average_size = bytes_timeseries("XDS Average Message Size").with_target(
    query(
      "sum by (url) (rate(agentgateway_xds_message_bytes_total[$__rate_interval])) "
      "/ sum by (url) (rate(agentgateway_xds_message_total[$__rate_interval]))",
      "{{url}}",
    )
  )

  runtime_memory = add_targets(
    bytes_timeseries("Cgroup Memory"),
    [
      (pod_metric("agentgateway_cgroup_usage"), "{{namespace}}/{{pod}} usage"),
      (
        pod_metric("agentgateway_cgroup_working_set"),
        "{{namespace}}/{{pod}} working set",
      ),
      (pod_metric("agentgateway_cgroup_anon"), "{{namespace}}/{{pod}} anonymous"),
      (pod_metric("agentgateway_cgroup_file"), "{{namespace}}/{{pod}} file"),
      (pod_metric("agentgateway_cgroup_kernel"), "{{namespace}}/{{pod}} kernel"),
      (pod_metric("agentgateway_cgroup_sock"), "{{namespace}}/{{pod}} socket"),
      (pod_metric("agentgateway_cgroup_slab"), "{{namespace}}/{{pod}} slab"),
    ],
  )
  process_memory = add_targets(
    bytes_timeseries("Process Memory"),
    [
      (pod_metric("agentgateway_process_rss"), "{{namespace}}/{{pod}} rss"),
      (pod_metric("agentgateway_process_pss"), "{{namespace}}/{{pod}} pss"),
      (
        pod_metric("agentgateway_process_private_dirty"),
        "{{namespace}}/{{pod}} private dirty",
      ),
      (
        pod_metric("agentgateway_process_private_clean"),
        "{{namespace}}/{{pod}} private clean",
      ),
      (
        pod_metric("agentgateway_process_shared_clean"),
        "{{namespace}}/{{pod}} shared clean",
      ),
      (pod_metric("agentgateway_process_swap"), "{{namespace}}/{{pod}} swap"),
    ],
  )
  tokio_runtime = add_targets(
    base_timeseries("Tokio Runtime"),
    [
      (pod_metric("agentgateway_tokio_num_workers"), "{{namespace}}/{{pod}} workers"),
      (
        pod_metric("agentgateway_tokio_num_alive_tasks"),
        "{{namespace}}/{{pod}} alive tasks",
      ),
      (
        pod_metric("agentgateway_tokio_global_queue_depth"),
        "{{namespace}}/{{pod}} global queue depth",
      ),
    ],
  )
  build_info = base_timeseries("Build Versions").with_target(
    query(
      prom_sum(pod_metric("agentgateway_build_info"), by=["tag"]),
      "{{tag}}",
    )
  )

  return (
    new_dashboard("Agentgateway", "agentgateway")
    .with_variable(
      QueryVariable("namespace")
      .datasource(DataSourceRef(type_val="prometheus", uid="$datasource"))
      .label("Namespace")
      .query("label_values(agentgateway_build_info,namespace)")
      .refresh(VariableRefresh.ON_TIME_RANGE_CHANGED)
      .sort(VariableSort.ALPHABETICAL_ASC)
      .multi(True)
      .include_all(True)
    )
    .with_variable(
      QueryVariable("gateway_name")
      .datasource(DataSourceRef(type_val="prometheus", uid="$datasource"))
      .label("Gateway")
      .query(
        'label_values(agentgateway_build_info{namespace=~"$namespace"},gateway_networking_k8s_io_gateway_name)'
      )
      .refresh(VariableRefresh.ON_TIME_RANGE_CHANGED)
      .sort(VariableSort.ALPHABETICAL_ASC)
      .multi(True)
      .include_all(True)
    )
    .with_variable(
      QueryVariable("gateway")
      .datasource(DataSourceRef(type_val="prometheus", uid="$datasource"))
      .label("Gateway Full Name")
      .query(
        'query_result(label_join(agentgateway_build_info{namespace=~"$namespace",gateway_networking_k8s_io_gateway_name=~"$gateway_name"}, "gateway", "/", "namespace", "gateway_networking_k8s_io_gateway_name"))'
      )
      .regex('/.*gateway="([^"]+).*/')
      .refresh(VariableRefresh.ON_TIME_RANGE_CHANGED)
      .sort(VariableSort.ALPHABETICAL_ASC)
      .multi(True)
      .include_all(True)
      .hide(VariableHide.HIDE_VARIABLE)
    )
    .with_variable(
      QueryVariable("pod")
      .datasource(DataSourceRef(type_val="prometheus", uid="$datasource"))
      .label("Pod")
      .query(
        'query_result(agentgateway_build_info{namespace=~"$namespace",gateway_networking_k8s_io_gateway_name=~"$gateway_name"})'
      )
      .regex('.*pod="([^"]+)".*')
      .refresh(VariableRefresh.ON_TIME_RANGE_CHANGED)
      .sort(VariableSort.ALPHABETICAL_ASC)
      .multi(True)
      .include_all(True)
    )
    .with_row(Row("Overview"))
    .with_panel(dpmem)
    .with_panel(dpcpu)
    .with_row(Row("Requests"))
    .with_panel(requests)
    .with_panel(requests_by_gateway)
    .with_panel(request_by_status)
    .with_panel(request_by_reason)
    .with_row(
      Row("LLM")
      .collapsed(True)
      .with_panel(llm_tokens)
      .with_panel(llm_ttft)
      .with_panel(llm_time)
      .with_panel(llm_tps)
    )
    .with_row(Row("MCP").collapsed(True).with_panel(mcp_tools).with_panel(mcp_list))
    .with_row(Row("Latency").collapsed(True).with_panel(latency_by_route))
    .with_row(
      Row("XDS")
      .collapsed(True)
      .with_panel(xds_messages)
      .with_panel(xds_average_size)
    )
    .with_row(
      Row("Runtime")
      .collapsed(True)
      .with_panel(runtime_memory)
      .with_panel(process_memory)
      .with_panel(tokio_runtime)
      .with_panel(build_info)
    )
  )


class Manifest:
  @classmethod
  def dashboard(cls, dash: Dashboard) -> SDKManifest:
    if dash.uid is None:
      raise RuntimeError("dashboards must have a uid")

    return SDKManifest(
      api_version="dashboard.grafana.app/v1beta1",
      kind="Dashboard",
      metadata=Metadata(
        annotations={"generated-by": "python"},
        name=dash.uid,
      ),
      spec=dash,
    )


def encode_dashboard() -> str:
  dashboard = build_dashboard().build()
  encoder = JSONEncoder(sort_keys=True, indent=2)
  return encoder.encode(dashboard)


def encode_manifest() -> str:
  dashboard = build_dashboard().build()
  manifest = Manifest.dashboard(dashboard)
  encoder = JSONEncoder(sort_keys=True, indent=2)
  return encoder.encode(manifest)


def main() -> None:
  parser = argparse.ArgumentParser()
  parser.add_argument(
    "--legacy",
    action="store_true",
    help="emit the raw dashboard JSON instead of the dashboard.grafana.app manifest",
  )
  args = parser.parse_args()

  if args.legacy:
    print(encode_dashboard())
  else:
    print(encode_manifest())


if __name__ == "__main__":
  main()
