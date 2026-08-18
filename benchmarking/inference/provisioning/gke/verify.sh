#!/usr/bin/env bash
# Verify that an existing GKE cluster satisfies the benchmark contract.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
source "${SCRIPT_DIR}/common.sh"

pool_target_size() {
  local pool="$1" url zone group target total=0
  local urls
  urls="$(pool_value "${pool}" 'instanceGroupUrls[]')"
  while IFS= read -r url; do
    [[ -n "${url}" ]] || continue
    zone="$(sed -E 's#^.*/zones/([^/]+)/.*#\1#' <<<"${url}")"
    group="${url##*/}"
    target="$(gcloud compute instance-groups managed describe "${group}" \
      --project "${GKE_PROJECT}" --zone "${zone}" --format='value(targetSize)')"
    is_nonnegative_integer "${target}" || die "could not determine target size for ${zone}/${group}"
    total=$((total + target))
  done <<<"${urls}"
  echo "${total}"
}

verify_cluster_config() {
  local network subnetwork workload_pool pd_csi filestore dns_cache
  local managed_prometheus monitoring_components
  network="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" --format='value(network)')"
  subnetwork="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" --format='value(subnetwork)')"
  workload_pool="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(workloadIdentityConfig.workloadPool)')"
  filestore="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(addonsConfig.gcpFilestoreCsiDriverConfig.enabled)')"
  pd_csi="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(addonsConfig.gcePersistentDiskCsiDriverConfig.enabled)')"
  dns_cache="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(addonsConfig.dnsCacheConfig.enabled)')"
  managed_prometheus="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(monitoringConfig.managedPrometheusConfig.enabled)')"
  monitoring_components="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(monitoringConfig.componentConfig.enableComponents)')"
  [[ "${network##*/}" == "${GKE_NETWORK}" ]] || \
    die "cluster network ${network} does not match ${GKE_NETWORK}"
  [[ "${subnetwork##*/}" == "${GKE_SUBNETWORK}" ]] || \
    die "cluster subnetwork ${subnetwork} does not match ${GKE_SUBNETWORK}"
  [[ "${workload_pool}" == "${GKE_PROJECT}.svc.id.goog" ]] || \
    die "Workload Identity is not configured for ${GKE_PROJECT}"
  [[ "${pd_csi}" == "True" || "${pd_csi}" == "true" ]] || \
    die "the GKE Persistent Disk CSI driver is not enabled"
  [[ "${filestore}" == "True" || "${filestore}" == "true" ]] || \
    die "the GKE Filestore CSI driver is not enabled"
  [[ "${dns_cache}" == "True" || "${dns_cache}" == "true" ]] || \
    die "NodeLocal DNS is not enabled"
  [[ "${managed_prometheus}" == "True" || "${managed_prometheus}" == "true" ]] || \
    die "Google Managed Service for Prometheus is not enabled"
  [[ ";${monitoring_components};" == *';SYSTEM_COMPONENTS;'* && \
     ";${monitoring_components};" == *';CADVISOR;'* && \
     ";${monitoring_components};" == *';KUBELET;'* ]] || \
    die "SYSTEM, CADVISOR, and KUBELET monitoring components are required"
}

wait_for_storage_classes() {
  local deadline=$((SECONDS + 300)) class
  local -a classes=(standard-rwx premium-rwx)
  while true; do
    local missing=0
    for class in "${classes[@]}"; do
      kubectl --context "${GKE_KUBE_CONTEXT}" get storageclass "${class}" >/dev/null 2>&1 || missing=1
    done
    (( missing != 0 )) || return 0
    (( SECONDS < deadline )) || die "timed out waiting for standard-rwx and premium-rwx storage classes"
    sleep 5
  done
}

write_provisioning_manifest() {
  local output="${BENCHMARK_GKE_PROVISIONING_MANIFEST:-${SCRIPT_DIR}/../../results/provisioning/gke/${GKE_CLUSTER}.yaml}"
  local output_dir tmp version gpu_current
  output_dir="$(dirname "${output}")"
  mkdir -p "${output_dir}"
  tmp="$(mktemp "${output_dir}/.${GKE_CLUSTER}.XXXXXX")"
  version="$(gcloud container clusters describe "${GKE_CLUSTER}" \
    --project "${GKE_PROJECT}" --location "${GKE_LOCATION}" \
    --format='value(currentMasterVersion)')"
  gpu_current="$(pool_target_size "${GKE_GPU_NODEPOOL}")"
  cat >"${tmp}" <<EOF
apiVersion: benchmark.agentgateway.dev/v1alpha1
kind: GKEProvisioningRecord
generatedAt: "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
project: ${GKE_PROJECT}
location: ${GKE_LOCATION}
cluster: ${GKE_CLUSTER}
kubernetesVersion: ${version}
network: ${GKE_NETWORK}
subnetwork: ${GKE_SUBNETWORK}
nodeServiceAccount: ${GKE_NODE_SERVICE_ACCOUNT}
systemNodePool:
  name: ${GKE_SYSTEM_NODEPOOL}
  machineType: ${GKE_SYSTEM_MACHINE_TYPE}
  nodes: ${GKE_SYSTEM_NODES}
harnessNodePool:
  name: ${GKE_HARNESS_NODEPOOL}
  machineType: ${GKE_HARNESS_MACHINE_TYPE}
  nodes: ${GKE_HARNESS_NODES}
gpuNodePool:
  name: ${GKE_GPU_NODEPOOL}
  machineType: ${GKE_GPU_MACHINE_TYPE}
  acceleratorType: ${GKE_GPU_ACCELERATOR_TYPE}
  acceleratorsPerNode: ${GKE_GPU_ACCELERATORS_PER_NODE}
  targetNodes: ${GKE_GPU_TARGET_NODES}
  currentNodes: ${gpu_current}
  spot: ${GKE_GPU_SPOT}
EOF
  mv "${tmp}" "${output}"
  log "wrote provisioning record to ${output}"
}

main() {
  export CLOUDSDK_CORE_DISABLE_PROMPTS=1
  preflight
  cluster_exists || die "cluster ${GKE_CLUSTER} does not exist"
  assert_managed_cluster
  verify_cluster_config
  verify_cpu_pool "${GKE_SYSTEM_NODEPOOL}" "${GKE_SYSTEM_MACHINE_TYPE}"
  verify_cpu_pool "${GKE_HARNESS_NODEPOOL}" "${GKE_HARNESS_MACHINE_TYPE}"
  verify_gpu_pool
  [[ "$(pool_target_size "${GKE_SYSTEM_NODEPOOL}")" == "${GKE_SYSTEM_NODES}" ]] || \
    die "system node pool is not at ${GKE_SYSTEM_NODES} node(s)"
  [[ "$(pool_target_size "${GKE_HARNESS_NODEPOOL}")" == "${GKE_HARNESS_NODES}" ]] || \
    die "harness node pool is not at ${GKE_HARNESS_NODES} node(s)"
  get_credentials
  kubectl --context "${GKE_KUBE_CONTEXT}" cluster-info >/dev/null || \
    die "cannot reach ${GKE_KUBE_CONTEXT}"
  wait_for_storage_classes
  write_provisioning_manifest
  print_config
  print_benchmark_exports
  log "GKE benchmark infrastructure is ready; GPU nodes are managed separately"
}

main "$@"
