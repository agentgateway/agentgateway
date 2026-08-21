#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNNER="${SCRIPT_DIR}/../../../run-benchmark.sh"

BENCHMARK_CLUSTER_PROVIDER=gke \
BENCHMARK_MODEL_STORAGE_PROFILE=high-throughput-shared \
BENCHMARK_WORKLOAD_STORAGE_PROFILE=shared \
bash -c '
  source "$1"
  resolve_storage_profiles
  [[ "$MODEL_STORAGE_STRATEGY" == filestore ]]
  [[ "$MODEL_STORAGE_CLASS" == premium-rwx ]]
  [[ "$MODEL_STORAGE_SIZE" == 2560Gi ]]
  [[ "$WORKLOAD_STORAGE_CLASS" == standard-rwx ]]
  [[ "$WORKLOAD_STORAGE_SIZE" == 100Gi ]]
' _ "${RUNNER}"

# Explicit low-level values override the profile mapping.
BENCHMARK_CLUSTER_PROVIDER=gke \
BENCHMARK_MODEL_STORAGE_PROFILE=high-throughput-shared \
BENCHMARK_MODEL_STORAGE_CLASS=custom-rwx \
BENCHMARK_MODEL_STORAGE_SIZE=3Ti \
bash -c '
  source "$1"
  resolve_storage_profiles
  [[ "$MODEL_STORAGE_CLASS" == custom-rwx ]]
  [[ "$MODEL_STORAGE_SIZE" == 3Ti ]]
' _ "${RUNNER}"

if BENCHMARK_CLUSTER_PROVIDER=kind \
    BENCHMARK_MODEL_STORAGE_PROFILE=high-throughput-shared \
    bash -c 'source "$1"; resolve_storage_profiles' _ "${RUNNER}"; then
  echo "non-GKE storage profile was unexpectedly accepted" >&2
  exit 1
fi

echo "storage profile tests passed"

BENCHMARK_TREATMENT=service \
BENCHMARK_CAMPAIGN_ID=test-campaign bash -c '
  source "$1"
  validate_configuration
  [[ "$TREATMENT_ID" == service ]]
  [[ "$RESULTS_DIR" == */results/llm-d-benchmark/test-campaign ]]
  [[ "$SPEC_DIR" == */.work/service/repetition-1 ]]
' _ "${RUNNER}"

BENCHMARK_TREATMENT=agentgateway-gateway \
BENCHMARK_CAMPAIGN_ID=test-campaign \
BENCHMARK_REPETITION=3 bash -c '
  source "$1"
  validate_configuration
  [[ "$BENCHMARK_GATEWAY_IMPLEMENTATION" == agentgateway ]]
  [[ "$BENCHMARK_ROUTER_MODE" == gateway ]]
  [[ "$SPEC_DIR" == */.work/agentgateway-gateway/repetition-3 ]]
' _ "${RUNNER}"

echo "campaign treatment tests passed"

ENDPOINT_TEST_DIR="$(mktemp -d)"
mkdir -p "${ENDPOINT_TEST_DIR}/.venv/bin"
ln -s "$(command -v python3)" "${ENDPOINT_TEST_DIR}/.venv/bin/python"
LLM_D_BENCHMARK_DIR="${ENDPOINT_TEST_DIR}" bash -c '
  source "$1"
  SCENARIO_NAME=test-standalone
  BENCHMARK_TREATMENT=agentgateway-standalone
  kubectl() {
    cat <<"JSON"
{"items":[
  {"kind":"Service","metadata":{"name":"model-router-epp"},"spec":{"clusterIP":"10.0.0.9","ports":[{"name":"http","port":80}]}},
  {"kind":"Service","metadata":{"name":"llm-d-benchmark-harness"},"spec":{"clusterIP":"10.0.0.10","ports":[{"name":"rsync","port":20873}]}}
]}
JSON
  }
  [[ "$(resolve_internal_endpoint)" == "http://10.0.0.9:80" ]]
' _ "${RUNNER}"
rm -r -- "${ENDPOINT_TEST_DIR:?}"

echo "standalone internal endpoint test passed"

BENCHMARK_CLUSTER_PROVIDER=gke \
BENCHMARK_CAMPAIGN_ID=test-campaign \
BENCHMARK_GKE_PROJECT=test-project \
BENCHMARK_GKE_LOCATION=us-central1-a bash -c '
  source "$1"
  SCENARIO_NAME=test-scenario
  TREATMENT_ID=service
  kubectl() {
    case " $* " in
      *" get namespace test-scenario -o jsonpath={.metadata.labels.app\\.kubernetes\\.io/managed-by} "*) printf agentgateway-benchmark ;;
      *" get namespace test-scenario -o jsonpath={.metadata.annotations.benchmark\\.agentgateway\\.dev/campaign-id} "*) printf test-campaign ;;
      *" get namespace test-scenario "*) return 0 ;;
      *" get pvc model-pvc "*) printf pv-model ;;
      *" get pvc workload-pvc "*) printf pv-workload ;;
      *" get pv pv-model -o jsonpath={.spec.claimRef.namespace} "*|*" get pv pv-workload -o jsonpath={.spec.claimRef.namespace} "*) printf test-scenario ;;
      *" get pv pv-model -o jsonpath={.spec.claimRef.name} "*) printf model-pvc ;;
      *" get pv pv-workload -o jsonpath={.spec.claimRef.name} "*) printf workload-pvc ;;
      *" get pv pv-model -o jsonpath={.spec.csi.driver} "*|*" get pv pv-workload -o jsonpath={.spec.csi.driver} "*) printf filestore.csi.storage.gke.io ;;
      *" get pv pv-model -o jsonpath={.spec.csi.volumeHandle} "*) printf modeInstance/us-central1-a/model-instance/vol1 ;;
      *" get pv pv-workload -o jsonpath={.spec.csi.volumeHandle} "*) printf modeInstance/us-central1-a/workload-instance/vol1 ;;
      *" delete pod access-to-harness-data-workload-pvc "*|*" delete pvc "*|*" wait --for=delete pv/"*) return 0 ;;
      *) echo "unexpected kubectl call: $*" >&2; return 1 ;;
    esac
  }
  gcloud() {
    echo "ERROR: NOT_FOUND" >&2
    return 1
  }
  cleanup_treatment_storage
' _ "${RUNNER}"

echo "treatment storage cleanup tests passed"

BENCHMARK_CLUSTER_PROVIDER=gke BENCHMARK_CAMPAIGN_ID=test-campaign bash -c '
  source "$1"
  kubectl() {
    case " $* " in
      *" create namespace "*) printf "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: agentgateway-benchmark-system\n" ;;
      *" apply -f - "*) return 0 ;;
      *" get configmap campaign-lock "*) printf test-campaign ;;
      *) return 1 ;;
    esac
  }
  acquire_campaign_lock
' _ "${RUNNER}"

if BENCHMARK_CLUSTER_PROVIDER=gke BENCHMARK_CAMPAIGN_ID=test-campaign bash -c '
  source "$1"
  kubectl() {
    case " $* " in
      *" create namespace "*) printf "apiVersion: v1\nkind: Namespace\nmetadata:\n  name: agentgateway-benchmark-system\n" ;;
      *" apply -f - "*) return 0 ;;
      *" get configmap campaign-lock "*) printf another-campaign ;;
      *) return 1 ;;
    esac
  }
  acquire_campaign_lock
' _ "${RUNNER}"; then
  echo "campaign lock unexpectedly allowed a different campaign" >&2
  exit 1
fi

echo "campaign lock tests passed"

BENCHMARK_CLUSTER_PROVIDER=kind bash -c '
  source "$1"
  kubectl() {
    echo "Kind no-op unexpectedly called kubectl: $*" >&2
    return 1
  }
  acquire_campaign_lock
' _ "${RUNNER}"

echo "Kind campaign lock no-op test passed"

BENCHMARK_CLUSTER_PROVIDER=kind bash -c '
  source "$1"
  SCENARIO_NAME=test-scenario
  kubectl() {
    echo "Kind no-op unexpectedly called kubectl: $*" >&2
    return 1
  }
  cleanup_treatment_storage
' _ "${RUNNER}"

BENCHMARK_CLUSTER_PROVIDER=gke bash -c '
  source "$1"
  SCENARIO_NAME=absent-scenario
  kubectl() {
    [[ " $* " == *" get namespace absent-scenario "* ]] || {
      echo "unexpected kubectl call: $*" >&2
      return 2
    }
    return 1
  }
  cleanup_treatment_storage
' _ "${RUNNER}"

echo "storage cleanup no-op tests passed"

BENCHMARK_ROUTING_POLICY=optimized-baseline bash -c '
  source "$1"
  resolve_workload
  [[ "$WORKLOAD" == upstream-optimized-baseline.yaml ]]
  [[ "$WORKLOAD_FILE_PATH" == */workloads/upstream-optimized-baseline.yaml.in ]]
  resolve_request_timeout
  [[ "$REQUEST_TIMEOUT" == 300 ]]
' _ "${RUNNER}"

BENCHMARK_ROUTING_POLICY=optimized-baseline \
BENCHMARK_WORKLOAD_VARIANT=deterministic bash -c '
  source "$1"
  resolve_workload
  [[ "$WORKLOAD" == deterministic-optimized-baseline.yaml ]]
  [[ "$WORKLOAD_FILE_PATH" == */workloads/deterministic-optimized-baseline.yaml.in ]]
  resolve_request_timeout
  [[ "$REQUEST_TIMEOUT" == 120 ]]
' _ "${RUNNER}"

BENCHMARK_ROUTING_POLICY=load-only bash -c '
  source "$1"
  resolve_workload
  [[ "$WORKLOAD" == upstream-load-only.yaml ]]
  [[ "$WORKLOAD_FILE_PATH" == */workloads/upstream-load-only.yaml.in ]]
' _ "${RUNNER}"

echo "routing workload tests passed"

if harness_error="$(BENCHMARK_TREATMENT=service \
    BENCHMARK_CAMPAIGN_ID=test-campaign BENCHMARK_HARNESS=guidellm \
    bash -c 'source "$1"; validate_configuration' _ "${RUNNER}" 2>&1)"; then
  echo "GuideLLM was unexpectedly accepted" >&2
  exit 1
fi
grep -q 'only inference-perf is implemented' <<<"${harness_error}"

echo "unsupported harness test passed"

BENCHMARK_RUNTIME_METRICS=false \
BENCHMARK_GKE_MONITORING=off \
BENCHMARK_METRICS_INTERVAL=5 bash -c '
  source "$1"
  [[ "$RUNTIME_METRICS_ENABLED" == false ]]
  [[ "$GKE_MONITORING_MODE" == off ]]
  [[ "$METRICS_INTERVAL" == 5 ]]
' _ "${RUNNER}"

echo "runtime monitoring configuration tests passed"

evidence_test_dir="$(mktemp -d)"
mkdir -p "${evidence_test_dir}/source" "${evidence_test_dir}/artifacts"
printf '{"requests":[{"id":1}]}\n' > \
  "${evidence_test_dir}/source/per_request_lifecycle_metrics.json"
printf '{}\n' > "${evidence_test_dir}/source/summary_lifecycle_metrics.json"
BENCHMARK_WORKLOAD_VARIANT=upstream bash -c '
  source "$1"
  SPEC_DIR="$2/spec"
  mkdir -p "$SPEC_DIR"
  printf "scenario: []\n" > "$SPEC_DIR/scenario.yaml"
  printf "config\n" > "$3/config.yaml"
  printf "metadata\n" > "$3/run_metadata.yaml"
  printf "stdout\n" > "$3/stdout.log"
  printf "stderr\n" > "$3/stderr.log"
  write_prism_supporting_artifacts service "$3/report.yaml" "$4"
  gzip -t "$4/per_request_lifecycle_metrics.json.gz"
' _ "${RUNNER}" "${evidence_test_dir}" \
  "${evidence_test_dir}/source" "${evidence_test_dir}/artifacts"
rm -rf "${evidence_test_dir}"

echo "compressed per-request evidence test passed"

if BENCHMARK_TREATMENT=service \
    BENCHMARK_CAMPAIGN_ID=test-campaign \
    BENCHMARK_GPU_RELEASE_POLICY=invalid \
    bash -c 'source "$1"; validate_configuration' _ "${RUNNER}"; then
  echo "invalid GPU release policy was unexpectedly accepted" >&2
  exit 1
fi

BENCHMARK_CLUSTER_PROVIDER=gke \
BENCHMARK_KUBE_CONTEXT=gke_test_us-central1-a_test-cluster bash -c '
  source "$1"
  resolve_gke_identity
  [[ "$GKE_PROJECT" == test ]]
  [[ "$GKE_LOCATION" == us-central1-a ]]
  [[ "$GKE_CLUSTER" == test-cluster ]]
' _ "${RUNNER}"

BENCHMARK_CLUSTER_PROVIDER=gke \
BENCHMARK_ACCELERATOR_TYPE=gpu \
BENCHMARK_GPU_RELEASE_POLICY=after-load bash -c '
  source "$1"
  SCENARIO_NAME=test-scenario
  kubectl() {
    case " $* " in
      *" get pods "*) printf "pod/harness\n" ;;
      *" exec "*) return 1 ;;
      *" logs "*) printf "LLMDBENCH_EVENT_V1 traffic_complete experiment_id=test final_stage_id=16\n" ;;
      *) return 1 ;;
    esac
  }
  traffic_complete_visible 16
  [[ "$TRAFFIC_COMPLETE_SOURCE" == llmdbench-event ]]
' _ "${RUNNER}"

gpu_release_test_dir="$(mktemp -d)"
BENCHMARK_CLUSTER_PROVIDER=gke \
BENCHMARK_ACCELERATOR_TYPE=gpu \
BENCHMARK_GPU_RELEASE_POLICY=after-load bash -c '
  source "$1"
  SPEC_DIR="$2"
  SCENARIO_NAME=test-scenario
  GKE_PROJECT=test
  GKE_LOCATION=us-central1-a
  GKE_CLUSTER=test-cluster
  GKE_GPU_NODEPOOL=gpu-h100
  REPLICAS=8
  TENSOR_PARALLELISM=2
  TRAFFIC_COMPLETE_SOURCE=llmdbench-event
  gpu_workload_targets() { printf "deployment/qwen\n"; }
  kubectl() {
    case " $* " in
      *" scale deployment/qwen "*) return 0 ;;
      *" get nodes "*) return 0 ;;
      *) echo "unexpected kubectl call: $*" >&2; return 1 ;;
    esac
  }
  gcloud() {
    case " $* " in
      *" container node-pools describe "*) printf "gpu-h100\n" ;;
      *" container clusters resize "*) return 0 ;;
      *) echo "unexpected gcloud call: $*" >&2; return 1 ;;
    esac
  }
  release_gke_gpu_pool
  grep -Fxq "policy: \"after-load\"" "$SPEC_DIR/gpu-release.yaml"
  grep -Fxq "accelerators_released: 16" "$SPEC_DIR/gpu-release.yaml"
' _ "${RUNNER}" "${gpu_release_test_dir}"
rm -rf "${gpu_release_test_dir}"

echo "GPU release policy tests passed"

workload_test_dir="$(mktemp -d)"
trap 'rm -rf "${workload_test_dir}"' EXIT
python3 "${SCRIPT_DIR}/../scripts/render-workload.py" \
  --input "${SCRIPT_DIR}/../workloads/upstream-optimized-baseline.yaml.in" \
  --output "${workload_test_dir}/scaled.yaml" \
  --rate-scale 0.5 \
  --request-timeout 120
grep -Fxq '    - rate: 7.5' "${workload_test_dir}/scaled.yaml"
grep -Fxq '    - rate: 1.5' "${workload_test_dir}/scaled.yaml"
grep -Fxq '    - rate: 30' "${workload_test_dir}/scaled.yaml"
grep -Fxq '  request_timeout: 120' "${workload_test_dir}/scaled.yaml"
grep -Fxq '  type: poisson' "${workload_test_dir}/scaled.yaml"
[[ "$(grep -c '    - rate:' "${workload_test_dir}/scaled.yaml")" == 17 ]]

grep -Fxq '  type: poisson' \
  "${SCRIPT_DIR}/../workloads/upstream-load-only.yaml.in"
grep -Fxq '    per_request: true' \
  "${SCRIPT_DIR}/../workloads/upstream-optimized-baseline.yaml.in"
grep -Fxq '  type: constant' \
  "${SCRIPT_DIR}/../workloads/deterministic-optimized-baseline.yaml.in"
grep -Fxq '  base_seed: 42' \
  "${SCRIPT_DIR}/../workloads/deterministic-optimized-baseline.yaml.in"

echo "workload rendering tests passed"
