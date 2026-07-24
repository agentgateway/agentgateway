import os
import json
import glob
import re
import pandas as pd
import matplotlib.pyplot as plt

plt.style.use('seaborn-v0_8-whitegrid' if 'seaborn-v0_8-whitegrid' in plt.style.available else 'default')
plt.rcParams.update({
    'font.family': 'sans-serif',
    'font.size': 11,
    'axes.labelsize': 12,
    'axes.titlesize': 14,
    'xtick.labelsize': 10,
    'ytick.labelsize': 10,
    'figure.titlesize': 16,
    'figure.dpi': 150,
    'grid.alpha': 0.4,
    'grid.linestyle': '--'
})

def parse_lifecycle_dict(data):
    load_summary = data.get('load_summary', {})
    target_qps = load_summary.get('target_request_rate', load_summary.get('requested_rate', 0.0))
    actual_qps = load_summary.get('actual_request_rate', load_summary.get('achieved_rate', 0.0))
    
    successes = data.get('successes', {})
    count = successes.get('count', 0)
    
    throughput = successes.get('throughput', {})
    req_throughput = throughput.get('requests_per_sec', 0.0)
    token_throughput = throughput.get('tokens_per_sec', throughput.get('total_tokens_per_sec', 0.0))
    
    latency = successes.get('latency', {})
    
    def get_stat(metric_dict, name, default=0.0):
        if not metric_dict:
            return default, default, default, default
        m = metric_dict.get(name, {})
        if isinstance(m, dict):
            return (
                m.get('mean', default),
                m.get('p50', m.get('median', default)),
                m.get('p90', default),
                m.get('p99', default)
            )
        return default, default, default, default

    ttft_mean, ttft_p50, ttft_p90, ttft_p99 = get_stat(latency, 'time_to_first_token')
    if ttft_mean == 0.0 or ttft_mean is None:
        ttft_mean, ttft_p50, ttft_p90, ttft_p99 = get_stat(latency, 'request_latency')

    tpot_mean, tpot_p50, tpot_p90, tpot_p99 = get_stat(latency, 'inter_token_latency')
    if tpot_mean == 0.0 or tpot_mean is None:
        tpot_mean, tpot_p50, tpot_p90, tpot_p99 = get_stat(latency, 'time_per_output_token')
        
    req_lat_mean, req_lat_p50, req_lat_p90, req_lat_p99 = get_stat(latency, 'request_latency')
    
    return {
        'target_qps': target_qps,
        'actual_qps': actual_qps,
        'success_count': count,
        'req_throughput': req_throughput,
        'token_throughput': token_throughput,
        'ttft_mean': ttft_mean,
        'ttft_p50': ttft_p50,
        'ttft_p90': ttft_p90,
        'ttft_p99': ttft_p99,
        'tpot_mean': tpot_mean,
        'tpot_p50': tpot_p50,
        'tpot_p90': tpot_p90,
        'tpot_p99': tpot_p99,
        'req_lat_mean': req_lat_mean,
        'req_lat_p50': req_lat_p50,
        'req_lat_p90': req_lat_p90,
        'req_lat_p99': req_lat_p99
    }

def parse_lifecycle_json(file_path):
    with open(file_path, 'r') as f:
        data = json.load(f)
    return parse_lifecycle_dict(data)

def load_results(label):
    script_dir = os.path.dirname(os.path.abspath(__file__))
    base_path = os.path.join(script_dir, "output", "default-run", label, "results", "json")
    
    records = []
    
    pattern = os.path.join(base_path, "**", "stage_*_lifecycle_metrics.json")
    files = glob.glob(pattern, recursive=True)
    if not files:
        pattern = os.path.join(base_path, "stage_*_lifecycle_metrics.json")
        files = glob.glob(pattern)
        
    if files:
        for f in files:
            try:
                record = parse_lifecycle_json(f)
                stage_num = int(os.path.basename(f).split('_')[1])
                record['stage'] = stage_num
                records.append(record)
            except Exception as e:
                print(f"warning: failed to parse {f}: {e}")
    else:
        # kubectl cp fails on completed pods, fall back to stdout logs
        fallback_file = os.path.join(base_path, "fallback_logs.json")
        if os.path.exists(fallback_file):
            try:
                with open(fallback_file, 'r') as f:
                    content = f.read()
                matches = re.finditer('=== START_STAGE_(\\d+) ===\\n(.*?)\\n=== END_STAGE_\\1 ===', content, re.DOTALL)
                for match in matches:
                    stage_num = int(match.group(1))
                    json_str = match.group(2)
                    try:
                        data = json.loads(json_str)
                        record = parse_lifecycle_dict(data)
                        record['stage'] = stage_num
                        records.append(record)
                    except Exception as json_err:
                        print(f"warning: failed to parse stage {stage_num} JSON in fallback: {json_err}")
            except Exception as e:
                print(f"warning: failed to read {fallback_file}: {e}")
                
    if not records:
        print(f"no results found in {base_path}")
        return pd.DataFrame()
        
    df = pd.DataFrame(records)
    df = df.sort_values(by='target_qps').reset_index(drop=True)
    return df

df_ag_prefill = load_results("agentgateway-bench-prefill")
df_ag_decode = load_results("agentgateway-bench-decode")
df_plain_prefill = load_results("plain-service-bench-prefill")
df_plain_decode = load_results("plain-service-bench-decode")

output_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "output")
os.makedirs(output_dir, exist_ok=True)

# Plot 1: TTFT (prefill-heavy)
if not df_ag_prefill.empty:
    fig, ax = plt.subplots(figsize=(10, 6))
    ax.plot(df_ag_prefill['target_qps'], df_ag_prefill['ttft_p50'], marker='o', color='#1f77b4', label='AgentGateway p50')
    ax.plot(df_ag_prefill['target_qps'], df_ag_prefill['ttft_p90'], marker='^', linestyle='--', color='#1f77b4', label='AgentGateway p90')
    if not df_plain_prefill.empty:
        ax.plot(df_plain_prefill['target_qps'], df_plain_prefill['ttft_p50'], marker='s', color='#2ca02c', label='Plain Service p50')
        ax.plot(df_plain_prefill['target_qps'], df_plain_prefill['ttft_p90'], marker='s', linestyle='--', color='#2ca02c', label='Plain Service p90')

    ax.set_xlabel('Target Load (QPS)')
    ax.set_ylabel('Time to First Token (seconds)')
    ax.set_title('TTFT Comparison (Prefill-Heavy Workload)')
    ax.set_xticks(df_ag_prefill['target_qps'])
    ax.legend()
    ax.grid(True)
    plt.tight_layout()
    plt.savefig(os.path.join(output_dir, 'ttft_comparison.png'), dpi=300)
    plt.close()
    print("saved ttft_comparison.png")

# Plot 2: request latency (decode-heavy)
if not df_ag_decode.empty:
    fig, ax = plt.subplots(figsize=(10, 6))
    ax.plot(df_ag_decode['target_qps'], df_ag_decode['req_lat_p50'], marker='o', color='#1f77b4', label='AgentGateway p50')
    ax.plot(df_ag_decode['target_qps'], df_ag_decode['req_lat_p90'], marker='^', linestyle='--', color='#1f77b4', label='AgentGateway p90')
    if not df_plain_decode.empty:
        ax.plot(df_plain_decode['target_qps'], df_plain_decode['req_lat_p50'], marker='s', color='#2ca02c', label='Plain Service p50')
        ax.plot(df_plain_decode['target_qps'], df_plain_decode['req_lat_p90'], marker='s', linestyle='--', color='#2ca02c', label='Plain Service p90')

    ax.set_xlabel('Target Load (QPS)')
    ax.set_ylabel('Request Latency (seconds)')
    ax.set_title('Request Latency Comparison (Decode-Heavy Workload)')
    ax.set_xticks(df_ag_decode['target_qps'])
    ax.legend()
    ax.grid(True)
    plt.tight_layout()
    plt.savefig(os.path.join(output_dir, 'latency_comparison.png'), dpi=300)
    plt.close()
    print("saved latency_comparison.png")

# Plot 3: throughput vs load
if not df_ag_decode.empty:
    fig, ax = plt.subplots(figsize=(10, 6))
    max_val = max(df_ag_decode['target_qps'].max(), df_ag_decode['req_throughput'].max())
    if not df_plain_decode.empty:
        max_val = max(max_val, df_plain_decode['req_throughput'].max())
    ax.plot([0, max_val], [0, max_val], linestyle=':', color='gray', label='Ideal Throughput')
    ax.plot(df_ag_decode['target_qps'], df_ag_decode['req_throughput'], marker='o', color='#1f77b4', label='AgentGateway')
    if not df_plain_decode.empty:
        ax.plot(df_plain_decode['target_qps'], df_plain_decode['req_throughput'], marker='s', color='#2ca02c', label='Plain Service')

    ax.set_xlabel('Target Load (QPS)')
    ax.set_ylabel('Actual Throughput (RPS)')
    ax.set_title('Throughput vs Target Load (Decode-Heavy)')
    ax.set_xticks(df_ag_decode['target_qps'])
    ax.legend()
    ax.grid(True)
    plt.tight_layout()
    plt.savefig(os.path.join(output_dir, 'throughput_comparison.png'), dpi=300)
    plt.close()
    print("saved throughput_comparison.png")
