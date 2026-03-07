use std::collections::HashMap;

use rmcp::model::{Task, TaskStatus};

#[derive(Debug, Default)]
pub(crate) struct TaskStore {
	next_id: u64,
	tasks: HashMap<String, TaskEntry>,
}

impl TaskStore {
	pub(crate) fn create_task(&mut self, result: serde_json::Value) -> Task {
		let task_id = format!("task-{}", self.next_id);
		self.next_id += 1;
		let created_at = "2026-01-01T00:00:00Z".to_string();
		let task = Task::new(
			task_id.clone(),
			TaskStatus::Working,
			created_at.clone(),
			created_at,
		)
		.with_status_message("queued")
		.with_poll_interval(10);
		self.tasks.insert(
			task_id,
			TaskEntry {
				task: task.clone(),
				result: Some(result),
			},
		);
		task
	}

	pub(crate) fn get_mut(&mut self, task_id: &str) -> Option<&mut TaskEntry> {
		self.tasks.get_mut(task_id)
	}

	pub(crate) fn iter_tasks(&self) -> impl Iterator<Item = &Task> {
		self.tasks.values().map(|entry| &entry.task)
	}
}

#[derive(Debug)]
pub(crate) struct TaskEntry {
	pub(crate) task: Task,
	pub(crate) result: Option<serde_json::Value>,
}
