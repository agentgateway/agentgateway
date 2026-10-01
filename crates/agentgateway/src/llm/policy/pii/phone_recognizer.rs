use once_cell::sync::Lazy;
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
		static CANDIDATE_RE: Lazy<Regex> =
			Lazy::new(|| Regex::new(r"(?i)(^|[^0-9])([+()]?[0-9][0-9\t\p{Zs}().\-]{6,30})").unwrap());

		// Map region strings once.
		fn to_country(code: &str) -> Option<country::Id> {
			match code {
				"US" => Some(country::US),
				"CA" => Some(country::CA),
				"GB" => Some(country::GB),
				"DE" => Some(country::DE),
				"IL" => Some(country::IL),
				"IN" => Some(country::IN),
				"BR" => Some(country::BR),
				_ => None,
			}
		}

		static TOKEN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\S+").unwrap());

		let best_match = |span: &str, start: usize| -> Option<RecognizerResult> {
			let candidate = span.trim_end_matches(|c: char| !c.is_ascii_digit());
			let mut best: Option<RecognizerResult> = None;

			for &region in &self.regions {
				let Some(country) = to_country(region) else {
					continue;
				};
				if let Ok(num) = parse(Some(country), candidate) {
					if !num.is_valid() {
						continue;
					}

					// prefer longer matches
					let digit_count = candidate.chars().filter(|c| c.is_ascii_digit()).count();
					let score = 0.6_f32 + (digit_count.min(15) as f32) / 100.0;

					let res = RecognizerResult {
						entity_type: "PHONE_NUMBER".to_string(),
						matched: candidate.to_string(),
						start,
						end: start + candidate.len(),
						score,
					};

					best = match best {
						Some(prev) => {
							let prev_digits = prev.matched.chars().filter(|c| c.is_ascii_digit()).count();
							if digit_count > prev_digits || (digit_count == prev_digits && score > prev.score) {
								Some(res)
							} else {
								Some(prev)
							}
						},
						None => Some(res),
					};
				}
			}

			best
		};

		let mut results = Vec::new();

		for caps in CANDIDATE_RE.captures_iter(text) {
			let m = caps.get(2).unwrap();
			// one candidate may have multiple phonenumbers so take the longest valid match
			let tokens: Vec<_> = TOKEN_RE.find_iter(m.as_str()).collect();
			let mut i = 0;
			while i < tokens.len() {
				let found = (i..tokens.len()).rev().find_map(|j| {
					let span = &m.as_str()[tokens[i].start()..tokens[j].end()];
					best_match(span, m.start() + tokens[i].start()).map(|r| (j, r))
				});
				match found {
					Some((j, r)) => {
						results.push(r);
						i = j + 1;
					},
					None => i += 1,
				}
			}
		}

		results.sort_by_key(|r| (r.start, r.end, r.matched.clone()));
		results.dedup_by(|a, b| a.start == b.start && a.end == b.end && a.matched == b.matched);
		results
	}

	fn name(&self) -> &str {
		"PHONE_NUMBER"
	}
}
