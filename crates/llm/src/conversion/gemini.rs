pub mod from_responses {
	use bytes::Bytes;

	use crate::AIError;

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		super::super::completions::translate_google_error(bytes)
	}
}
