# Dashboards

`agentgateway-dashboard.py` generates the AgentGateway Grafana dashboard from the Grafana Foundation SDK.

By default it emits a `dashboard.grafana.app/v1beta1` `Dashboard` manifest. Use `--legacy` to emit the raw dashboard JSON used by the Helm chart's Grafana sidecar ConfigMap.

To update the Helm chart copy:

```bash
controller/install/dashboards/agentgateway-dashboard.py --legacy > controller/install/helm/agentgateway/files/agentgateway-dashboard.json
```
