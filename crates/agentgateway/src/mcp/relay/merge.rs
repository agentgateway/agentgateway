//! Result merging for multiplexed MCP operations.
//!
//! The relay owns the downstream-facing aggregation contract. These helpers
//! merge list-style responses and synthesize a single initialize result while
//! preserving target-local capability snapshots.

use super::*;
use crate::mcp::mergestream::MergeFn;

pub(super) fn merge_meta(entries: impl IntoIterator<Item = (Strng, Option<Meta>)>) -> Option<Meta> {
	let mut items = entries
		.into_iter()
		.filter_map(|(server_name, meta)| meta.map(|m| (server_name, m)));

	let first = items.next()?;
	let Some(second) = items.next() else {
		return Some(first.1);
	};

	let mut per_upstream = Map::new();
	per_upstream.insert(first.0.to_string(), Value::Object(first.1.0));
	per_upstream.insert(second.0.to_string(), Value::Object(second.1.0));
	for (server_name, meta) in items {
		per_upstream.insert(server_name.to_string(), Value::Object(meta.0));
	}

	let mut root = Map::new();
	root.insert("upstreams".to_string(), Value::Object(per_upstream));
	Some(Meta(root))
}

impl Relay {
	pub fn merge_tools(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let target_router = self.target_router.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::with_capacity(streams.len());
			let mut tools = Vec::new();
			for (server_name, s) in streams {
				let ServerResult::ListToolsResult(ltr) = s else {
					continue;
				};
				let upstream_tools = ltr.tools;
				let meta = ltr.meta;
				meta_entries.push((server_name.clone(), meta));
				tools.reserve(upstream_tools.len());
				for mut t in upstream_tools {
					if !policies.validate(
						&rbac::ResourceType::Tool(rbac::ResourceId::new(
							server_name.as_str(),
							t.name.to_string(),
						)),
						&cel,
					) {
						continue;
					}
					t.name = target_router
						.resource_name(server_name.as_str(), t.name)
						.into_owned()
						.into();
					tools.push(t);
				}
			}
			let meta = merge_meta(meta_entries);
			Ok(
				ListToolsResult {
					tools,
					next_cursor: None,
					meta,
				}
				.into(),
			)
		})
	}

	pub fn merge_initialize(&self, pv: ProtocolVersion) -> Box<MergeFn> {
		let relay = self.clone();
		let multiplexing = self.is_multiplexing;
		Box::new(move |s| {
			for (name, result) in &s {
				if let ServerResult::InitializeResult(info) = result
					&& let Some(target_session) = relay.target_session(name.as_str())
				{
					target_session.set_info(info.clone());
				}
			}
			if !multiplexing {
				return match s.into_iter().next() {
					Some((_, ServerResult::InitializeResult(ir))) => Ok(ir.into()),
					_ => Ok(Self::get_info(pv, multiplexing).into()),
				};
			}

			let mut has_tools = false;
			let mut has_prompts = false;
			let mut has_tasks = false;
			let mut has_resources = false;
			let mut has_resource_subscribe = false;
			let mut has_resource_list_changed = false;
			let mut has_logging = false;
			let mut has_completions = false;
			let mut extensions = std::collections::BTreeMap::new();

			let lowest_version = s
				.into_iter()
				.flat_map(|(_, v)| match v {
					ServerResult::InitializeResult(r) => {
						has_tools |= r.capabilities.tools.is_some();
						has_prompts |= r.capabilities.prompts.is_some();
						has_tasks |= r.capabilities.tasks.is_some();
						if let Some(res) = &r.capabilities.resources {
							has_resources = true;
							has_resource_subscribe |= res.subscribe.unwrap_or_default();
							has_resource_list_changed |= res.list_changed.unwrap_or_default();
						}
						has_logging |= r.capabilities.logging.is_some();
						has_completions |= r.capabilities.completions.is_some();
						if let Some(ext) = &r.capabilities.extensions {
							extensions.extend(ext.clone());
						}
						Some(r.protocol_version)
					},
					_ => None,
				})
				.min_by(|a, b| {
					a.partial_cmp(b)
						.expect("ProtocolVersion ordering must be total")
				})
				.unwrap_or(pv);
			let mut capabilities = ServerCapabilities::default();
			capabilities.completions = has_completions.then_some(rmcp::model::JsonObject::default());
			capabilities.logging = has_logging.then_some(rmcp::model::JsonObject::default());
			capabilities.tasks = has_tasks.then_some(TasksCapability::default());
			capabilities.tools = has_tools.then_some(ToolsCapability::default());
			capabilities.prompts = has_prompts.then_some(PromptsCapability::default());
			capabilities.resources = has_resources.then_some(ResourcesCapability {
				subscribe: Some(has_resource_subscribe),
				list_changed: Some(has_resource_list_changed),
			});
			capabilities.extensions = if extensions.is_empty() {
				None
			} else {
				Some(extensions)
			};
			Ok(ServerInfo::new(capabilities)
				.with_protocol_version(lowest_version)
				.with_server_info(Implementation::from_build_env())
				.with_instructions(
					"This server is a gateway to a set of MCP servers. It is responsible for routing requests to the correct server and aggregating the results.",
				)
				.into())
		})
	}

	pub fn merge_prompts(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let target_router = self.target_router.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::with_capacity(streams.len());
			let mut prompts = Vec::new();
			for (server_name, s) in streams {
				let ServerResult::ListPromptsResult(lpr) = s else {
					continue;
				};
				let upstream_prompts = lpr.prompts;
				let meta = lpr.meta;
				meta_entries.push((server_name.clone(), meta));
				prompts.reserve(upstream_prompts.len());
				for mut p in upstream_prompts {
					if !policies.validate(
						&rbac::ResourceType::Prompt(rbac::ResourceId::new(
							server_name.as_str(),
							p.name.as_str(),
						)),
						&cel,
					) {
						continue;
					}
					let old_name = std::mem::take(&mut p.name);
					p.name = target_router
						.resource_name(server_name.as_str(), Cow::Owned(old_name))
						.into_owned();
					prompts.push(p);
				}
			}
			let meta = merge_meta(meta_entries);
			Ok(
				ListPromptsResult {
					prompts,
					next_cursor: None,
					meta,
				}
				.into(),
			)
		})
	}

	pub fn merge_resources(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let target_router = self.target_router.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::with_capacity(streams.len());
			let mut resources = Vec::new();
			for (server_name, s) in streams {
				let ServerResult::ListResourcesResult(lrr) = s else {
					continue;
				};
				let upstream_resources = lrr.resources;
				let meta = lrr.meta;
				meta_entries.push((server_name.clone(), meta));
				resources.reserve(upstream_resources.len());
				for mut r in upstream_resources {
					if !policies.validate(
						&rbac::ResourceType::Resource(rbac::ResourceId::new(
							server_name.as_str(),
							r.uri.as_str(),
						)),
						&cel,
					) {
						continue;
					}
					r.uri = target_router
						.wrap_resource_uri(server_name.as_str(), &r.uri)
						.into_owned();
					let old_name = std::mem::take(&mut r.name);
					r.name = target_router
						.resource_name(server_name.as_str(), Cow::Owned(old_name))
						.into_owned();
					resources.push(r);
				}
			}
			let meta = merge_meta(meta_entries);
			Ok(
				ListResourcesResult {
					resources,
					next_cursor: None,
					meta,
				}
				.into(),
			)
		})
	}

	pub fn merge_resource_templates(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let target_router = self.target_router.clone();
		Box::new(move |streams| {
			let mut meta_entries = Vec::with_capacity(streams.len());
			let mut resource_templates = Vec::new();
			for (server_name, s) in streams {
				let ServerResult::ListResourceTemplatesResult(lrr) = s else {
					continue;
				};
				let upstream_templates = lrr.resource_templates;
				let meta = lrr.meta;
				meta_entries.push((server_name.clone(), meta));
				resource_templates.reserve(upstream_templates.len());
				for mut rt in upstream_templates {
					if !policies.validate(
						&rbac::ResourceType::Resource(rbac::ResourceId::new(
							server_name.as_str(),
							rt.uri_template.as_str(),
						)),
						&cel,
					) {
						continue;
					}
					rt.uri_template = target_router
						.wrap_resource_uri(server_name.as_str(), &rt.uri_template)
						.into_owned();
					let old_name = std::mem::take(&mut rt.name);
					rt.name = target_router
						.resource_name(server_name.as_str(), Cow::Owned(old_name))
						.into_owned();
					resource_templates.push(rt);
				}
			}
			let meta = merge_meta(meta_entries);
			Ok(
				ListResourceTemplatesResult {
					resource_templates,
					next_cursor: None,
					meta,
				}
				.into(),
			)
		})
	}

	pub fn merge_tasks(&self, cel: CelExecWrapper) -> Box<MergeFn> {
		let policies = self.policies.clone();
		let target_router = self.target_router.clone();
		Box::new(move |streams| {
			let mut tasks = Vec::new();
			let mut upstream_result_count = 0usize;
			let mut single_next_cursor = None;
			let mut single_total = None;
			for (server_name, s) in streams {
				let ServerResult::ListTasksResult(ltr) = s else {
					continue;
				};
				upstream_result_count += 1;
				let rmcp::model::ListTasksResult {
					tasks: upstream_tasks,
					next_cursor,
					total,
					..
				} = ltr;
				if upstream_result_count == 1 {
					single_next_cursor = next_cursor;
					single_total = total;
				}
				tasks.reserve(upstream_tasks.len());
				for mut task in upstream_tasks {
					if !policies.validate(
						&rbac::ResourceType::Task(rbac::ResourceId::new(
							server_name.as_str(),
							task.task_id.as_str(),
						)),
						&cel,
					) {
						continue;
					}
					target_router.prefix_task_id(server_name.as_str(), &mut task.task_id);
					tasks.push(task);
				}
			}
			let (next_cursor, total) = if upstream_result_count == 1 {
				(single_next_cursor, single_total)
			} else {
				(None, None)
			};
			let mut out = ListTasksResult::new(tasks);
			out.next_cursor = next_cursor;
			out.total = total;
			Ok(out.into())
		})
	}

	pub fn merge_empty(&self) -> Box<MergeFn> {
		Box::new(move |_| Ok(rmcp::model::ServerResult::empty(())))
	}

	fn get_info(pv: ProtocolVersion, _multiplexing: bool) -> ServerInfo {
		let capabilities = ServerCapabilities::builder()
			.enable_tasks_with(TasksCapability::default())
			.enable_tools_with(ToolsCapability::default())
			.enable_prompts_with(PromptsCapability::default())
			.enable_resources_with(ResourcesCapability::default())
			.build();
		ServerInfo::new(capabilities)
			.with_protocol_version(pv)
			.with_server_info(Implementation::from_build_env())
			.with_instructions(
				"This server is a gateway to a set of MCP servers. It is responsible for routing requests to the correct server and aggregating the results.",
			)
	}
}
