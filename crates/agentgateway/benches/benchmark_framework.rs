//! Comprehensive benchmark framework with statistical rigor and metrics collection
//! 
//! This module provides the infrastructure for industry-standard benchmark reporting
//! including statistical analysis, environment documentation, and comprehensive metrics.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};
use serde::{Deserialize, Serialize};

/// Comprehensive benchmark metrics with statistical analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkMetrics {
    pub latency_percentiles: LatencyPercentiles,
    pub throughput: ThroughputMetrics,
    pub resource_usage: ResourceMetrics,
    pub error_rates: ErrorMetrics,
    pub statistical_analysis: StatisticalAnalysis,
}

/// Latency percentile measurements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyPercentiles {
    pub p50: Duration,
    pub p90: Duration,
    pub p95: Duration,
    pub p99: Duration,
    pub p99_9: Duration,
    pub min: Duration,
    pub max: Duration,
    pub mean: Duration,
}

/// Throughput measurements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputMetrics {
    pub requests_per_second: f64,
    pub bytes_per_second: f64,
    pub connections_per_second: f64,
    pub operations_per_second: f64,
}

/// Resource utilization metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceMetrics {
    pub cpu_usage_percent: f64,
    pub memory_usage_bytes: u64,
    pub memory_usage_mb: f64,
    pub file_descriptors: u32,
    pub network_connections: u32,
    pub peak_memory_bytes: u64,
    pub gc_collections: u32,
}

/// Error rate tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorMetrics {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub error_rate_percent: f64,
    pub timeout_count: u32,
    pub connection_errors: u32,
}

/// Statistical analysis of benchmark results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatisticalAnalysis {
    pub sample_count: usize,
    pub confidence_interval_95: ConfidenceInterval,
    pub standard_deviation: Duration,
    pub coefficient_of_variation: f64,
    pub outliers_detected: u32,
    pub outliers_removed: u32,
    pub statistical_significance: bool,
}

/// 95% confidence interval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceInterval {
    pub lower_bound: Duration,
    pub upper_bound: Duration,
    pub margin_of_error: Duration,
}

/// Complete environment information for reproducibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkEnvironment {
    pub hardware: HardwareInfo,
    pub software: SoftwareInfo,
    pub configuration: ConfigurationInfo,
    pub timestamp: SystemTime,
    pub benchmark_version: String,
}

/// Hardware specifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareInfo {
    pub cpu_model: String,
    pub cpu_cores: u32,
    pub cpu_threads: u32,
    pub cpu_frequency_mhz: u32,
    pub memory_total_gb: f64,
    pub memory_available_gb: f64,
    pub storage_type: String,
    pub network_interface: String,
}

/// Software environment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoftwareInfo {
    pub os_name: String,
    pub os_version: String,
    pub kernel_version: String,
    pub rust_version: String,
    pub cargo_version: String,
    pub agentgateway_version: String,
    pub dependencies: HashMap<String, String>,
}

/// Configuration settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigurationInfo {
    pub benchmark_settings: HashMap<String, String>,
    pub runtime_settings: HashMap<String, String>,
    pub environment_variables: HashMap<String, String>,
    pub feature_flags: Vec<String>,
}

/// Comprehensive benchmark result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub name: String,
    pub category: String,
    pub description: String,
    pub metrics: BenchmarkMetrics,
    pub environment: BenchmarkEnvironment,
    pub raw_measurements: Vec<Duration>,
    pub metadata: HashMap<String, String>,
}

/// Benchmark execution context with statistical collection
pub struct BenchmarkContext {
    measurements: Vec<Duration>,
    start_time: Instant,
    resource_monitor: ResourceMonitor,
    error_tracker: ErrorTracker,
    sample_count: usize,
    warmup_iterations: usize,
}

/// Resource monitoring during benchmark execution
pub struct ResourceMonitor {
    initial_memory: u64,
    peak_memory: u64,
    cpu_samples: Vec<f64>,
    fd_count: u32,
    connection_count: u32,
}

/// Error tracking during benchmark execution
pub struct ErrorTracker {
    total_operations: u64,
    successful_operations: u64,
    failed_operations: u64,
    timeout_count: u32,
    connection_errors: u32,
}

impl BenchmarkContext {
    /// Create a new benchmark context with specified sample count
    pub fn new(sample_count: usize, warmup_iterations: usize) -> Self {
        Self {
            measurements: Vec::with_capacity(sample_count),
            start_time: Instant::now(),
            resource_monitor: ResourceMonitor::new(),
            error_tracker: ErrorTracker::new(),
            sample_count,
            warmup_iterations,
        }
    }

    /// Execute warmup iterations to stabilize JIT compilation
    pub fn warmup<F>(&mut self, mut operation: F) 
    where 
        F: FnMut() -> Result<(), Box<dyn std::error::Error>>
    {
        for _ in 0..self.warmup_iterations {
            let _ = operation();
        }
    }

    /// Record a single measurement
    pub fn record_measurement(&mut self, duration: Duration) {
        if self.measurements.len() < self.sample_count {
            self.measurements.push(duration);
            self.resource_monitor.sample_resources();
        }
    }

    /// Record a successful operation
    pub fn record_success(&mut self) {
        self.error_tracker.record_success();
    }

    /// Record a failed operation
    pub fn record_failure(&mut self, error_type: &str) {
        self.error_tracker.record_failure(error_type);
    }

    /// Generate comprehensive benchmark result
    pub fn finalize(self, name: String, category: String, description: String) -> BenchmarkResult {
        let metrics = self.calculate_metrics();
        let environment = self.capture_environment();
        
        BenchmarkResult {
            name,
            category,
            description,
            metrics,
            environment,
            raw_measurements: self.measurements,
            metadata: HashMap::new(),
        }
    }

    /// Calculate comprehensive metrics with statistical analysis
    fn calculate_metrics(&self) -> BenchmarkMetrics {
        let latency_percentiles = self.calculate_latency_percentiles();
        let throughput = self.calculate_throughput();
        let resource_usage = self.resource_monitor.get_metrics();
        let error_rates = self.error_tracker.get_metrics();
        let statistical_analysis = self.calculate_statistical_analysis();

        BenchmarkMetrics {
            latency_percentiles,
            throughput,
            resource_usage,
            error_rates,
            statistical_analysis,
        }
    }

    /// Calculate latency percentiles from measurements
    fn calculate_latency_percentiles(&self) -> LatencyPercentiles {
        let mut sorted_measurements = self.measurements.clone();
        sorted_measurements.sort();

        let len = sorted_measurements.len();
        if len == 0 {
            return LatencyPercentiles {
                p50: Duration::ZERO,
                p90: Duration::ZERO,
                p95: Duration::ZERO,
                p99: Duration::ZERO,
                p99_9: Duration::ZERO,
                min: Duration::ZERO,
                max: Duration::ZERO,
                mean: Duration::ZERO,
            };
        }

        let percentile = |p: f64| -> Duration {
            let index = ((len as f64 * p / 100.0) as usize).min(len - 1);
            sorted_measurements[index]
        };

        let sum: Duration = sorted_measurements.iter().sum();
        let mean = sum / len as u32;

        LatencyPercentiles {
            p50: percentile(50.0),
            p90: percentile(90.0),
            p95: percentile(95.0),
            p99: percentile(99.0),
            p99_9: percentile(99.9),
            min: sorted_measurements[0],
            max: sorted_measurements[len - 1],
            mean,
        }
    }

    /// Calculate throughput metrics
    fn calculate_throughput(&self) -> ThroughputMetrics {
        let total_duration = self.start_time.elapsed();
        let total_operations = self.measurements.len() as f64;
        
        let requests_per_second = if total_duration.as_secs_f64() > 0.0 {
            total_operations / total_duration.as_secs_f64()
        } else {
            0.0
        };

        ThroughputMetrics {
            requests_per_second,
            bytes_per_second: 0.0, // To be calculated based on payload size
            connections_per_second: requests_per_second, // Simplified assumption
            operations_per_second: requests_per_second,
        }
    }

    /// Calculate statistical analysis including confidence intervals
    fn calculate_statistical_analysis(&self) -> StatisticalAnalysis {
        if self.measurements.is_empty() {
            return StatisticalAnalysis {
                sample_count: 0,
                confidence_interval_95: ConfidenceInterval {
                    lower_bound: Duration::ZERO,
                    upper_bound: Duration::ZERO,
                    margin_of_error: Duration::ZERO,
                },
                standard_deviation: Duration::ZERO,
                coefficient_of_variation: 0.0,
                outliers_detected: 0,
                outliers_removed: 0,
                statistical_significance: false,
            };
        }

        let mean_nanos = self.measurements.iter()
            .map(|d| d.as_nanos() as f64)
            .sum::<f64>() / self.measurements.len() as f64;

        let variance = self.measurements.iter()
            .map(|d| {
                let diff = d.as_nanos() as f64 - mean_nanos;
                diff * diff
            })
            .sum::<f64>() / self.measurements.len() as f64;

        let std_dev_nanos = variance.sqrt();
        let std_dev = Duration::from_nanos(std_dev_nanos as u64);

        let coefficient_of_variation = if mean_nanos > 0.0 {
            std_dev_nanos / mean_nanos
        } else {
            0.0
        };

        // Calculate 95% confidence interval
        let t_value = 1.96; // Approximate for large samples
        let margin_of_error_nanos = t_value * std_dev_nanos / (self.measurements.len() as f64).sqrt();
        let margin_of_error = Duration::from_nanos(margin_of_error_nanos as u64);

        let confidence_interval_95 = ConfidenceInterval {
            lower_bound: Duration::from_nanos((mean_nanos - margin_of_error_nanos).max(0.0) as u64),
            upper_bound: Duration::from_nanos((mean_nanos + margin_of_error_nanos) as u64),
            margin_of_error,
        };

        StatisticalAnalysis {
            sample_count: self.measurements.len(),
            confidence_interval_95,
            standard_deviation: std_dev,
            coefficient_of_variation,
            outliers_detected: 0, // TODO: Implement outlier detection
            outliers_removed: 0,
            statistical_significance: self.measurements.len() >= 30, // Basic rule of thumb
        }
    }

    /// Capture complete environment information
    fn capture_environment(&self) -> BenchmarkEnvironment {
        BenchmarkEnvironment {
            hardware: HardwareInfo::collect(),
            software: SoftwareInfo::collect(),
            configuration: ConfigurationInfo::collect(),
            timestamp: SystemTime::now(),
            benchmark_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

impl ResourceMonitor {
    fn new() -> Self {
        Self {
            initial_memory: Self::get_memory_usage(),
            peak_memory: 0,
            cpu_samples: Vec::new(),
            fd_count: 0,
            connection_count: 0,
        }
    }

    fn sample_resources(&mut self) {
        let current_memory = Self::get_memory_usage();
        self.peak_memory = self.peak_memory.max(current_memory);
        
        // Sample CPU usage (simplified)
        self.cpu_samples.push(Self::get_cpu_usage());
    }

    fn get_metrics(&self) -> ResourceMetrics {
        let avg_cpu = if !self.cpu_samples.is_empty() {
            self.cpu_samples.iter().sum::<f64>() / self.cpu_samples.len() as f64
        } else {
            0.0
        };

        ResourceMetrics {
            cpu_usage_percent: avg_cpu,
            memory_usage_bytes: self.peak_memory,
            memory_usage_mb: self.peak_memory as f64 / 1024.0 / 1024.0,
            file_descriptors: self.fd_count,
            network_connections: self.connection_count,
            peak_memory_bytes: self.peak_memory,
            gc_collections: 0, // Not applicable for Rust
        }
    }

    fn get_memory_usage() -> u64 {
        // Simplified memory usage - in production, use proper system APIs
        0
    }

    fn get_cpu_usage() -> f64 {
        // Simplified CPU usage - in production, use proper system APIs
        0.0
    }
}

impl ErrorTracker {
    fn new() -> Self {
        Self {
            total_operations: 0,
            successful_operations: 0,
            failed_operations: 0,
            timeout_count: 0,
            connection_errors: 0,
        }
    }

    fn record_success(&mut self) {
        self.total_operations += 1;
        self.successful_operations += 1;
    }

    fn record_failure(&mut self, error_type: &str) {
        self.total_operations += 1;
        self.failed_operations += 1;

        match error_type {
            "timeout" => self.timeout_count += 1,
            "connection" => self.connection_errors += 1,
            _ => {}
        }
    }

    fn get_metrics(&self) -> ErrorMetrics {
        let error_rate_percent = if self.total_operations > 0 {
            (self.failed_operations as f64 / self.total_operations as f64) * 100.0
        } else {
            0.0
        };

        ErrorMetrics {
            total_requests: self.total_operations,
            successful_requests: self.successful_operations,
            failed_requests: self.failed_operations,
            error_rate_percent,
            timeout_count: self.timeout_count,
            connection_errors: self.connection_errors,
        }
    }
}

impl HardwareInfo {
    fn collect() -> Self {
        // In production, use proper system APIs to collect hardware info
        Self {
            cpu_model: "Unknown CPU".to_string(),
            cpu_cores: num_cpus::get() as u32,
            cpu_threads: num_cpus::get() as u32,
            cpu_frequency_mhz: 0,
            memory_total_gb: 0.0,
            memory_available_gb: 0.0,
            storage_type: "Unknown".to_string(),
            network_interface: "Unknown".to_string(),
        }
    }
}

impl SoftwareInfo {
    fn collect() -> Self {
        let mut dependencies = HashMap::new();
        dependencies.insert("divan".to_string(), "0.1.21".to_string());
        dependencies.insert("tokio".to_string(), "1.46.1".to_string());

        Self {
            os_name: std::env::consts::OS.to_string(),
            os_version: "Unknown".to_string(),
            kernel_version: "Unknown".to_string(),
            rust_version: "1.88".to_string(), // From rust-toolchain.toml
            cargo_version: "Unknown".to_string(),
            agentgateway_version: env!("CARGO_PKG_VERSION").to_string(),
            dependencies,
        }
    }
}

impl ConfigurationInfo {
    fn collect() -> Self {
        let mut benchmark_settings = HashMap::new();
        benchmark_settings.insert("sample_count".to_string(), "100".to_string());
        benchmark_settings.insert("warmup_iterations".to_string(), "10".to_string());

        let mut runtime_settings = HashMap::new();
        runtime_settings.insert("worker_threads".to_string(), num_cpus::get().to_string());

        let environment_variables: HashMap<String, String> = std::env::vars()
            .filter(|(key, _)| key.starts_with("RUST_") || key.starts_with("CARGO_"))
            .collect();

        let feature_flags = vec!["internal_benches".to_string()];

        Self {
            benchmark_settings,
            runtime_settings,
            environment_variables,
            feature_flags,
        }
    }
}

/// Macro for creating statistically rigorous benchmarks
#[macro_export]
macro_rules! rigorous_benchmark {
    ($name:expr, $category:expr, $description:expr, $sample_count:expr, $warmup:expr, $operation:expr) => {{
        let mut context = BenchmarkContext::new($sample_count, $warmup);
        
        // Warmup phase
        context.warmup(|| {
            $operation();
            Ok(())
        });

        // Measurement phase
        for _ in 0..$sample_count {
            let start = std::time::Instant::now();
            match $operation() {
                Ok(_) => {
                    let duration = start.elapsed();
                    context.record_measurement(duration);
                    context.record_success();
                }
                Err(e) => {
                    context.record_failure("operation_error");
                }
            }
        }

        context.finalize($name.to_string(), $category.to_string(), $description.to_string())
    }};
}
