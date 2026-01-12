use super::recognizer_result::RecognizerResult;

pub trait Recognizer {
	fn recognize(&self, text: &str) -> Vec<RecognizerResult>;
	fn name(&self) -> &str;
}
