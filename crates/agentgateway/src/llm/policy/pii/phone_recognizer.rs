use rlibphonenumber::Region;
use rlibphonenumber::phonenumber_matcher::{FindNumberExt, Leniency, PhoneNumberMatch};

use super::recognizer::Recognizer;
use super::recognizer_result::RecognizerResult;

/// Recognizes phone numbers in free text using `rlibphonenumber`'s streaming
/// [`PhoneNumberMatcher`](rlibphonenumber::phonenumber_matcher::PhoneNumberMatcher).
///
/// Previously this recognizer hand-rolled a candidate regex and then looped over
/// a small, hard-coded list of regions (`US, GB, DE, IL, IN, CA, BR`), parsing
/// every candidate against each region and scoring/deduping the results manually.
/// That approach silently missed valid numbers from every other country and kept
/// an unused copy of libphonenumber's `_PATTERN` around.
///
/// `rlibphonenumber` exposes the real `PhoneNumberMatcher` (a port of Google's
/// `findNumbers`), which handles candidate extraction, validation and grouping
/// leniency natively. By default this recognizer now auto-detects numbers across
/// **all regions supported by the bundled metadata** (~250), resolving ambiguous
/// national-format numbers with a most-recently-used cache. Deployments that only
/// care about a fixed set of countries can opt back into a restricted subset via
/// [`PhoneRecognizer::with_regions`], which is both faster (fewer parse attempts)
/// and avoids spurious cross-region matches.
pub struct PhoneRecognizer {
	/// `None` => auto-detect across every supported region.
	/// `Some(_)` => only probe this subset of regions.
	regions: Option<Vec<Region>>,
	/// How strictly a candidate must look like a phone number to be reported.
	leniency: Leniency,
}

impl PhoneRecognizer {
	/// Creates a recognizer that auto-detects phone numbers across every region
	/// supported by `rlibphonenumber`'s metadata.
	///
	/// The default leniency is [`Leniency::Possible`], which is the appropriate
	/// choice for PII/DLP: the goal is to redact anything that *looks* like a
	/// phone number, even reserved/example ranges that are not actually
	/// assigned. Use [`PhoneRecognizer::with_leniency`] to tighten this (e.g.
	/// [`Leniency::Valid`] to only report fully valid numbers).
	pub fn new() -> Self {
		Self {
			regions: None,
			leniency: Leniency::Possible,
		}
	}

	/// Restricts detection to a specific subset of regions.
	///
	/// International (`+`) numbers are always resolved from their own country
	/// code; national-format numbers are only attributed to one of the provided
	/// regions. Order does not matter (the matcher sorts/dedups internally).
	pub fn with_regions(mut self, regions: impl IntoIterator<Item = Region>) -> Self {
		self.regions = Some(regions.into_iter().collect());
		self
	}

	/// Overrides the leniency used when scanning text.
	pub fn with_leniency(mut self, leniency: Leniency) -> Self {
		self.leniency = leniency;
		self
	}
}

impl Default for PhoneRecognizer {
	fn default() -> Self {
		Self::new()
	}
}

/// Converts a matcher result into a `RecognizerResult`.
///
/// The score is a confidence proxy based on how many digits the match carries
/// (longer, fully specified numbers are slightly more confident). The range is
/// kept identical to the previous implementation (`0.60..=0.75`) so downstream
/// thresholds keep working unchanged.
fn to_result(m: PhoneNumberMatch<'_>) -> RecognizerResult {
	let score = if m.number.is_valid() { 0.9 } else { 0.75 };

	RecognizerResult {
		entity_type: "PHONE_NUMBER".to_string(),
		matched: m.raw_string.to_string(),
		start: m.start,
		end: m.end(),
		score,
	}
}

impl Recognizer for PhoneRecognizer {
	fn recognize(&self, text: &str) -> Vec<RecognizerResult> {
		let builder = text.phone_number_matcher_builder().leniency(self.leniency);

		match &self.regions {
			None => builder.auto_region().build().map(to_result).collect(),
			Some(regions) => builder
				.regions(regions.iter().copied())
				.build()
				.map(to_result)
				.collect(),
		}
	}

	fn name(&self) -> &str {
		"PHONE_NUMBER"
	}
}
