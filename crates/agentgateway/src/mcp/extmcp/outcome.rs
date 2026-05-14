use rmcp::model::ErrorData;

#[derive(Debug)]
pub enum Outcome {
	Pass,
	Mutated,
	Reject(ErrorData),
}
