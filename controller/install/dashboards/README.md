# Dashboards

`agentgateway-dashboard.py` generates the Agentgateway Grafana dashboards from the Grafana Foundation SDK. Edit this generator rather than the generated JSON files.

By default it emits a `dashboard.grafana.app/v1beta1` `Dashboard` manifest. Use `--legacy` to emit the raw dashboard JSON used by the Helm chart's Grafana sidecar ConfigMap.

The default dashboard uses Kubernetes namespace, gateway, and pod filters. Use `--standalone` for deployments without these labels.

To update the Helm chart copies:

```bash
controller/install/dashboards/agentgateway-dashboard.py --legacy > controller/install/helm/agentgateway/files/agentgateway-dashboard.json
controller/install/dashboards/agentgateway-dashboard.py --standalone --legacy > controller/install/helm/agentgateway-standalone/files/agentgateway-dashboard.json
```
