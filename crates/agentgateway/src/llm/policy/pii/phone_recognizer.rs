use std::cmp::Reverse;

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
			Lazy::new(|| Regex::new(r"(?i)(^|[^0-9])([+()]?[0-9][0-9\t\p{Zs}().\-+]{6,255})").unwrap());

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
			results.extend(split_numbers(caps.get(2).unwrap(), best_match));
		}

		results.sort_by_key(|r| (r.start, r.end, r.matched.clone()));
		results.dedup_by(|a, b| a.start == b.start && a.end == b.end && a.matched == b.matched);
		results
	}

	fn name(&self) -> &str {
		"PHONE_NUMBER"
	}
}

fn split_numbers(
	run: regex::Match,
	parse_span: impl Fn(&str, usize) -> Option<RecognizerResult>,
) -> Vec<RecognizerResult> {
	static WORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\S+").unwrap());
	const MAX_NUMBER_LEN: usize = 32;
	const MAX_NUMBER_WORDS: usize = 6;

	let words: Vec<_> = WORD_RE.find_iter(run.as_str()).collect();
	// maximize digits covered, prefer fewer matches (implying each match is longer)
	let mut score_from = vec![(0, Reverse(0)); words.len() + 1];
	let mut first_number_from: Vec<Option<(usize, RecognizerResult)>> = vec![None; words.len() + 1];
	for first in (0..words.len()).rev() {
		score_from[first] = score_from[first + 1];
		for last in first..words.len().min(first + MAX_NUMBER_WORDS) {
			let span = &run.as_str()[words[first].start()..words[last].end()];
			if span.len() > MAX_NUMBER_LEN {
				break;
			}
			// skip parsing spans that cannot be valid phone numbers (E.164 allows at most 15 digits)
			let span_digits = span.chars().filter(|c| c.is_ascii_digit()).count();
			if span_digits > 15 {
				break;
			}
			if span_digits < 7 {
				continue;
			}
			let Some(number) = parse_span(span, run.start() + words[first].start()) else {
				continue;
			};
			let (rest_digits, Reverse(rest_count)) = score_from[last + 1];
			let digits = number
				.matched
				.chars()
				.filter(|c| c.is_ascii_digit())
				.count();
			let score = (rest_digits + digits, Reverse(rest_count + 1));
			if score > score_from[first] {
				score_from[first] = score;
				first_number_from[first] = Some((last + 1, number));
			}
		}
	}

	let mut numbers = Vec::new();
	let mut i = 0;
	while i < words.len() {
		match first_number_from[i].take() {
			Some((next, number)) => {
				numbers.push(number);
				i = next;
			},
			None => i += 1,
		}
	}
	numbers
}
