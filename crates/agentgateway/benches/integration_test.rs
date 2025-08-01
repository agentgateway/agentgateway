//! Integration test for benchmark report generation
//! 
//! This test validates that the report generation system works correctly
//! with actual benchmark data and produces the expected outputs.

use std::path::Path;
use std::time::{Duration, SystemTime};

// Import the report generator types
mod report_generator;
use report_generator::*;

/// Integration test for report generation with mock benchmark data
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🧪 Running benchmark report generation integration test...");
    
    // Create mock benchmark results that match our actual benchmark structure
    let mock_results = create_mock_benchmark_results();
    
    // Test report generation
    let output_dir = Path::new("target/benchmark_reports");
    
    println!("📊 Generating comprehensive benchmark report...");
    generate_comprehensive_report(mock_results, output_dir)?;
    
    // Validate that files were created
    validate_generated_files(output_dir)?;
    
    println!("✅ Integration test completed successfully!");
    println!("📁 Reports generated in: {}", output_dir.display());
    
    Ok(())
}

/// Create mock benchmark results that mirror our actual benchmark data
fn create_mock_benchmark_results() -> Vec<BenchmarkResult> {
    let mut results = Vec::new();
    
    // Proxy benchmarks
    results.push(create_mock_result(
        "http_request_latency_1_connection",
        "proxy_benchmarks",
        "HTTP request latency with 1 concurrent connection",
        Duration::from_millis(1), // 1.1ms average from actual results
        8500.0, // requests per second
    ));
    
    results.push(create_mock_result(
        "payload_throughput_1MB",
        "proxy_benchmarks", 
        "Payload throughput with 1MB payloads",
        Duration::from_millis(2), // 1.9ms average from actual results
        520.0, // requests per second
    ));
    
    // Protocol benchmarks
    results.push(create_mock_result(
        "mcp_call_tool",
        "protocol_benchmarks",
        "MCP call_tool message processing",
        Duration::from_millis(1), // 1.3ms average from actual results
        750.0, // requests per second
    ));
    
    results.push(create_mock_result(
        "a2a_agent_discovery",
        "protocol_benchmarks",
        "A2A agent discovery protocol handling",
        Duration::from_millis(1), // 1.3ms average from actual results
        765.0, // requests per second
    ));
    
    // Component benchmarks
    results.push(create_mock_result(
        "jwt_validation_HS256",
        "component_benchmarks",
        "JWT validation with HS256 algorithm",
        Duration::from_micros(65), // 65µs average from actual results
        15384.0, // requests per second
    ));
    
    results.push(create_mock_result(
        "config_parsing_simple",
        "component_benchmarks",
        "Simple configuration parsing performance",
        Duration::from_micros(64), // 64µs average from actual results
        15625.0, // requests per second
    ));
    
    // Comparative benchmarks
    results.push(create_mock_result(
        "agentgateway_vs_baseline",
        "comparative_benchmarks",
        "AgentGateway vs baseline HTTP server comparison",
        Duration::from_millis(7), // 6.7ms average from actual results
        148.0, // requests per second
    ));
    
    // Stress benchmarks
    results.push(create_mock_result(
        "connection_limit_10000",
        "stress_benchmarks",
        "Connection limit stress test with 10,000 connections",
        Duration::from_millis(117), // 117ms average from actual results
        8.5, // requests per second
    ));
    
    results.push(create_mock_result(
        "memory_pressure_100_workers",
        "stress_benchmarks",
        "Memory pressure test with 100 concurrent workers",
        Duration::from_secs(6), // 6.1s average from actual results
        0.16, // requests per second
    ));
    
    results
}

/// Create a mock benchmark result with realistic data
fn create_mock_result(
    name: &str,
    category: &str,
    description: &str,
    mean_duration: Duration,
    requests_per_second: f64,
) -> BenchmarkResult {
    // Calculate percentiles based on mean (simplified model)
    let p50 = Duration::from_nanos((mean_duration.as_nanos() as f64 * 0.9) as u64);
    let p90 = Duration::from_nanos((mean_duration.as_nanos() as f64 * 1.2) as u64);
    let p95 = Duration::from_nanos((mean_duration.as_nanos() as f64 * 1.4) as u64);
    let p99 = Duration::from_nanos((mean_duration.as_nanos() as f64 * 2.0) as u64);
    let p99_9 = Duration::from_nanos((mean_duration.as_nanos() as f64 * 3.0) as u64);
    let min = Duration::from_nanos((mean_duration.as_nanos() as f64 * 0.7) as u64);
    let max = Duration::from_nanos((mean_duration.as_nanos() as f64 * 4.0) as u64);
    
    // Generate raw measurements (simplified)
    let sample_count = 100;
    let mut raw_measurements = Vec::with_capacity(sample_count);
    for i in 0..sample_count {
        let variation = 1.0 + (i as f64 / sample_count as f64 - 0.5) * 0.4; // ±20% variation
        let duration = Duration::from_nanos((mean_duration.as_nanos() as f64 * variation) as u64);
        raw_measurements.push(duration);
    }
    
    BenchmarkResult {
        name: name.to_string(),
        category: category.to_string(),
        description: description.to_string(),
        metrics: BenchmarkMetrics {
            latency_percentiles: LatencyPercentiles {
                p50,
                p90,
                p95,
                p99,
                p99_9,
                mean: mean_duration,
                min,
                max,
            },
            throughput: ThroughputMetrics {
                requests_per_second,
                bytes_per_second: requests_per_second * 1024.0, // Assume 1KB average
                connections_per_second: requests_per_second * 0.1, // 10% new connections
            },
            resource_usage: ResourceMetrics {
                memory_usage_mb: 50.0 + (requests_per_second / 100.0), // Scale with throughput
                cpu_usage_percent: 10.0 + (requests_per_second / 1000.0), // Scale with throughput
                file_descriptors: 100 + (requests_per_second as u64 / 10),
                network_connections: requests_per_second as u64 / 5,
            },
            error_rates: ErrorMetrics {
                total_requests: sample_count as u64,
                successful_requests: sample_count as u64, // Perfect success for mock data
                failed_requests: 0,
                error_rate_percent: 0.0,
                timeout_count: 0,
            },
            statistical_analysis: StatisticalAnalysis {
                sample_count,
                confidence_interval_95: (
                    Duration::from_nanos((mean_duration.as_nanos() as f64 * 0.95) as u64),
                    Duration::from_nanos((mean_duration.as_nanos() as f64 * 1.05) as u64),
                ),
                standard_deviation: Duration::from_nanos((mean_duration.as_nanos() as f64 * 0.1) as u64),
                coefficient_of_variation: 0.1, // 10% CV
                outliers_removed: 2,
                statistical_significance: true,
            },
        },
        environment: BenchmarkEnvironment {
            hardware: HardwareInfo::collect(),
            software: SoftwareInfo::collect(),
            configuration: ConfigurationInfo::collect(),
            timestamp: SystemTime::now(),
            benchmark_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        raw_measurements,
    }
}

/// Validate that all expected report files were generated
fn validate_generated_files(output_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let html_path = output_dir.join("benchmark_report.html");
    let json_path = output_dir.join("benchmark_results.json");
    let csv_path = output_dir.join("benchmark_data.csv");
    
    // Check that files exist
    if !html_path.exists() {
        return Err(format!("HTML report not generated: {}", html_path.display()).into());
    }
    
    if !json_path.exists() {
        return Err(format!("JSON report not generated: {}", json_path.display()).into());
    }
    
    if !csv_path.exists() {
        return Err(format!("CSV export not generated: {}", csv_path.display()).into());
    }
    
    // Validate file contents (basic checks)
    let html_content = std::fs::read_to_string(&html_path)?;
    if !html_content.contains("AgentGateway Performance Benchmark Report") {
        return Err("HTML report missing expected title".into());
    }
    
    let json_content = std::fs::read_to_string(&json_path)?;
    if !json_content.contains("performance_claims_validation") {
        return Err("JSON report missing expected structure".into());
    }
    
    let csv_content = std::fs::read_to_string(&csv_path)?;
    if !csv_content.contains("Name,Category,P50_Latency_ms") {
        return Err("CSV export missing expected headers".into());
    }
    
    // Validate file sizes (should not be empty)
    let html_size = std::fs::metadata(&html_path)?.len();
    let json_size = std::fs::metadata(&json_path)?.len();
    let csv_size = std::fs::metadata(&csv_path)?.len();
    
    if html_size < 1000 {
        return Err("HTML report suspiciously small".into());
    }
    
    if json_size < 500 {
        return Err("JSON report suspiciously small".into());
    }
    
    if csv_size < 200 {
        return Err("CSV export suspiciously small".into());
    }
    
    println!("📊 File validation results:");
    println!("  HTML Report: {} bytes", html_size);
    println!("  JSON Data: {} bytes", json_size);
    println!("  CSV Export: {} bytes", csv_size);
    
    Ok(())
}
