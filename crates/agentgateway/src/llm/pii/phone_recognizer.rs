use phonenumber::{country, parse};
use regex::Regex;

use super::recognizer::Recognizer;
use super::recognizer_result::RecognizerResult;

pub struct PhoneRecognizer {
	regions: Vec<&'static str>,
}

impl PhoneRecognizer {
	pub fn new() -> Self {
		// this is _PATTERN from libphonenumbers
		let _r: Regex = Regex::new(r#"(?:[(\[（［+＋][-x‐-―−ー－-／  \u{AD}\u{200B}\u{2060}　()（）［］.\[\]/~⁓∼～]{0,4}){0,2}\d{1,20}(?:[-x‐-―−ー－-／  \u{AD}\u{200B}\u{2060}　()（）［］.\[\]/~⁓∼～]{0,4}\d{1,20}){0,20}(?:;ext=(\d{1,20})|[  \t,]*(?:e?xt(?:ensi(?:ó?|ó))?n?|ｅ?ｘｔｎ?|доб|anexo)[:\.．]?[  \t,-]*(\d{1,20})#?|[  \t,]*(?:[xｘ#＃~～]|int|ｉｎｔ)[:\.．]?[  \t,-]*(\d{1,9})#?|[- ]+(\d{1,6})#)?"#).unwrap();

		// Default regions to check, can be extended
		let regions = vec!["US", "GB", "DE", "IL", "IN", "CA", "BR"];
		Self { regions }
	}
}

impl Recognizer for PhoneRecognizer {
	fn recognize(&self, text: &str) -> Vec<RecognizerResult> {
		let mut results = Vec::new();
		// For each region, try to find phone numbers
		for &region in &self.regions {
			// phonenumber::parse requires a country code, so we use the region
			let country = match region {
				"US" => country::US,
				"GB" => country::GB,
				"DE" => country::DE,
				"IL" => country::IL,
				"IN" => country::IN,
				"CA" => country::CA,
				"BR" => country::BR,
				_ => continue,
			};
			// Use a sliding window to try to parse phone numbers from all substrings
			// (phonenumber crate does not provide a matcher, so we use a heuristic)
			for start in 0..text.len() {
				for end in (start + 7)..=std::cmp::min(text.len(), start + 20) {
					// phone numbers are usually 7-20 chars
					// TODO: we currently match this for every substring basically
					let candidate = &text[start..end];
					if let Ok(number) = parse(Some(country), candidate)
						&& number.is_valid()
					{
						results.push(RecognizerResult {
							entity_type: "PHONE_NUMBER".to_string(),
							matched: candidate.to_string(),
							start,
							end,
							score: 0.7, // Higher score for library-validated
						});
					}
				}
			}
		}
		// Merge overlapping and adjacent matches
		if !results.is_empty() {
			results.sort_by_key(|r| (r.start, r.end));
			let mut merged = Vec::new();
			let mut current = results[0].clone();
			
			for result in results.into_iter().skip(1) {
				// Check if overlapping or adjacent (within 1 character)
				if result.start <= current.end + 1 {
					current.end = current.end.max(result.end);
					if current.start < text.len() && current.end <= text.len() {
						current.matched = text[current.start..current.end].to_string();
					}
				} else {
					merged.push(current);
					current = result;
				}
			}
			merged.push(current);
			merged
		} else {
			results
		}
	}
	fn name(&self) -> &str {
		"PHONE_NUMBER"
	}
}

struct PhoneNumberMatcher {
	patterns: Regex,
}
impl PhoneNumberMatcher {
	pub fn new() -> Self {
		// this is _PATTERN from libphonenumbers
		let r: Regex = Regex::new(r#"(?:[(\[（［+＋][-x‐-―−ー－-／  \u{AD}\u{200B}\u{2060}　()（）［］.\[\]/~⁓∼～]{0,4}){0,2}\d{1,20}(?:[-x‐-―−ー－-／  \u{AD}\u{200B}\u{2060}　()（）［］.\[\]/~⁓∼～]{0,4}\d{1,20}){0,20}(?:;ext=(\d{1,20})|[  \t,]*(?:e?xt(?:ensi(?:ó?|ó))?n?|ｅ?ｘｔｎ?|доб|anexo)[:\.．]?[  \t,-]*(\d{1,20})#?|[  \t,]*(?:[xｘ#＃~～]|int|ｉｎｔ)[:\.．]?[  \t,-]*(\d{1,9})#?|[- ]+(\d{1,6})#)?"#).unwrap();

		Self { patterns: r }
	}

	pub fn find<'a>(&self, text: &'a str) -> impl std::iter::Iterator<Item = &'a str> {
		let candidates = self.patterns.find_iter(text);

		candidates.map(|m| m.as_str())
	}
}
