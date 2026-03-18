use once_cell::sync::Lazy;

const DEFAULT_INSTANCE_IP: &str = "1.1.1.1";

fn read(name: &str) -> Option<String> {
	std::env::var(name).ok()
}

fn read_or_default(name: &str, default: &str) -> String {
	read(name).unwrap_or_else(|| default.to_string())
}

#[derive(Debug)]
pub struct Env {
	pub instance_ip: Option<String>,
	pub pod_name: String,
	pub pod_namespace: String,
	pub node_name: String,
	pub gateway: String,
	pub role: String,
	pub node_id: String,
}

impl Env {
	fn from_env() -> Self {
		let instance_ip = read("INSTANCE_IP");
		let pod_name = read_or_default("POD_NAME", "");
		let pod_namespace = read_or_default("NAMESPACE", "");
		let node_name = read_or_default("NODE_NAME", "");
		let gateway = read_or_default("GATEWAY", "");
		let role = format!("{pod_namespace}~{gateway}");
		let node_id = format!(
			"agentgateway~{}~{pod_name}.{pod_namespace}~{pod_namespace}.svc.cluster.local",
			instance_ip.as_deref().unwrap_or(DEFAULT_INSTANCE_IP)
		);

		Self {
			instance_ip,
			pod_name,
			pod_namespace,
			node_name,
			gateway,
			role,
			node_id,
		}
	}
}

pub static ENV: Lazy<Env> = Lazy::new(Env::from_env);
