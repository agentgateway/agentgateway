#[derive(Debug, Clone, PartialEq)]
pub struct RecognizerResult {
	pub entity_type: String,
	pub matched: String,
	pub start: usize,
	pub end: usize,
	pub score: f32,
}
