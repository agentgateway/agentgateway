pub mod from_responses {
	use bytes::Bytes;

	use crate::AIError;

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		super::super::completions::translate_google_error(bytes)
	}
}

use indexmap::IndexMap;
use serde_json::Value;

use crate::conversion::completions::{google_error_type, parse_google_error};
use crate::conversion::value_text as text;
use crate::types::completions::typed::GoogleError;
use crate::types::gemini::decisions as google;
use crate::types::{decisions, systemone};
use crate::{AIError, InputFormat, json};

pub mod from_systemone {
	use super::*;

	#[derive(Debug)]
	pub struct State {
		model: String,
		questions: IndexMap<String, (String, QuestionState)>,
	}

	#[derive(Debug)]
	enum QuestionState {
		Noul,
		Choice(IndexMap<String, String>),
		Score(Vec<Value>),
	}

	pub fn translate(req: &systemone::Request) -> Result<(Vec<u8>, State), AIError> {
		if req
			.rest
			.get("assets")
			.is_some_and(|v| !v.is_null() && v != &serde_json::json!({}))
		{
			return Err(AIError::UnsupportedContent);
		}
		if let Some(fields) = req.rest.as_object()
			&& let Some(field) = fields
				.keys()
				.find(|field| !matches!(field.as_str(), "state" | "questions" | "assets"))
		{
			return Err(AIError::UnsupportedConversion(
				format!("unsupported SystemOne field {field:?}").into(),
			));
		}
		let req: systemone::typed::Request =
			json::convert(req).map_err(|err| AIError::RequestParsing(InputFormat::SystemOne, err))?;
		validate_question_count(req.questions.len(), 1000)?;
		let mut state = State {
			model: req.model.clone(),
			questions: IndexMap::new(),
		};
		let mut questions = IndexMap::new();
		for (i, (name, question)) in req.questions.into_iter().enumerate() {
			let (question_state, translated) = match question {
				systemone::typed::Question::Noul {
					instructions,
					criteria,
				} => (
					QuestionState::Noul,
					google::Question::Noul {
						instructions: text(instructions),
						criteria: criteria.map(|c| google::NoulCriteria {
							true_description: c.true_.map(text),
							false_description: c.false_.map(text),
						}),
					},
				),
				systemone::typed::Question::Choice {
					instructions,
					criteria,
				} => {
					validate_option_count(criteria.len(), "choice options")?;
					let mut values = IndexMap::new();
					let mut translated = IndexMap::new();
					for (i, (value, description)) in criteria.into_iter().enumerate() {
						let key = option_key(i);
						let description = description
							.map(text)
							.filter(|description| !description.trim().is_empty())
							.unwrap_or_else(|| value.clone());
						translated.insert(key.clone(), description);
						values.insert(key, value);
					}
					(
						QuestionState::Choice(values),
						google::Question::Choice {
							instructions: text(instructions),
							criteria: translated,
						},
					)
				},
				systemone::typed::Question::Score {
					instructions,
					criteria,
				} => {
					validate_option_count(criteria.len(), "score levels")?;
					let translated = criteria
						.iter()
						.map(|criterion| {
							let criterion = text(criterion.clone());
							validate_nonempty_criterion(&criterion)?;
							Ok(criterion)
						})
						.collect::<Result<_, AIError>>()?;
					(
						QuestionState::Score(criteria),
						google::Question::Score {
							instructions: text(instructions),
							criteria: translated,
						},
					)
				},
			};
			let key = format!("q_{i}");
			questions.insert(key.clone(), translated);
			state.questions.insert(key, (name, question_state));
		}
		let body = serde_json::to_vec(&google::Request {
			model: req.model,
			input: text(req.state),
			questions,
		})
		.map_err(AIError::RequestMarshal)?;
		Ok((body, state))
	}

	pub fn translate_response(bytes: &[u8], state: &State) -> Result<Vec<u8>, AIError> {
		let resp: google::Response = serde_json::from_slice(bytes).map_err(AIError::ResponseParsing)?;
		let mut answers = IndexMap::new();
		for (key, (name, question), answer) in take_answers(resp.answers, &state.questions)? {
			let answer = match (answer, question) {
				(google::Answer::Noul { noul }, QuestionState::Noul) => {
					validate_probability(noul, "noul")?;
					systemone::typed::Answer::Noul { noul }
				},
				(
					google::Answer::Choice {
						choice,
						probabilities,
						confidence,
					},
					QuestionState::Choice(values),
				) => {
					let (choice, probabilities) =
						checked_choice(values, &choice, &probabilities, confidence)?;
					let confidence =
						choice_confidence(&probabilities.iter().map(|(_, p)| *p).collect::<Vec<_>>());
					systemone::typed::Answer::Choice {
						choice,
						probabilities: probabilities.into_iter().collect(),
						confidence,
					}
				},
				(
					google::Answer::Score {
						score,
						probabilities,
						confidence,
					},
					QuestionState::Score(criteria),
				) => {
					let (_, probabilities) =
						checked_score(&score, &probabilities, confidence, criteria.len())?;
					let confidence = score_confidence(&probabilities);
					let legend = Some(
						criteria
							.iter()
							.enumerate()
							.map(|(i, c)| (i.to_string(), c.clone()))
							.collect(),
					);
					systemone::typed::Answer::Score {
						// Keep Google's number as sent so an integer score stays an integer.
						score,
						probabilities: probabilities
							.into_iter()
							.enumerate()
							.map(|(i, p)| (i.to_string(), p))
							.collect(),
						confidence,
						legend,
					}
				},
				_ => return Err(answer_type_mismatch(key)),
			};
			answers.insert(name.clone(), answer);
		}
		serde_json::to_vec(&systemone::typed::Response {
			model: state.model.clone(),
			answers,
			usage: Some(systemone::typed::Usage {
				input_tokens: resp.usage.total_input_tokens,
				output_tokens: resp.usage.total_output_tokens,
			}),
		})
		.map_err(AIError::ResponseMarshal)
	}
}

pub mod from_decisions {
	use async_openai::types::decisions as d;

	use super::*;

	#[derive(Debug)]
	pub struct State {
		model: String,
		questions: IndexMap<String, QuestionState>,
	}

	#[derive(Debug)]
	struct QuestionState {
		name: Option<String>,
		kind: QuestionKind,
	}

	#[derive(Debug)]
	enum QuestionKind {
		Predicate,
		Choice(IndexMap<String, d::ChoiceValue>),
		Score(Vec<String>),
	}

	pub fn translate(req: &decisions::Request) -> Result<(Vec<u8>, State), AIError> {
		let mut request = serde_json::to_value(req).map_err(AIError::RequestMarshal)?;
		normalize_decisions_input(&mut request)?;
		let req: d::DecisionRequest = serde_json::from_value(request)
			.map_err(|err| AIError::RequestParsing(InputFormat::Decisions, err))?;
		let d::DecisionRequest {
			model,
			input,
			questions: source_questions,
			// Google has no equivalent. It identifies the caller for OpenAI safety systems and
			// is not part of the evidence evaluated by the model.
			safety_identifier: _,
		} = req;
		validate_question_count(source_questions.len(), 400)?;
		let input = text(super::super::decisions_input_value(input)?);
		let mut state = State {
			model: model.clone(),
			questions: IndexMap::new(),
		};
		let mut questions = IndexMap::new();
		for (i, question) in source_questions.into_iter().enumerate() {
			let (name, kind, question) = match question {
				d::QuestionParam::Predicate(q) => (
					q.name,
					QuestionKind::Predicate,
					google::Question::Noul {
						instructions: q.instructions,
						criteria: None,
					},
				),
				d::QuestionParam::Choice(q) => {
					validate_option_count(q.choices.len(), "choice options")?;
					let mut values = IndexMap::new();
					let mut criteria = IndexMap::new();
					for (i, choice) in q.choices.into_iter().enumerate() {
						if values.values().any(|value| value == &choice.value) {
							return Err(AIError::UnsupportedConversion(
								"duplicate decision choice value".into(),
							));
						}
						let key = option_key(i);
						let description = choice_description(&choice.value, choice.description);
						criteria.insert(key.clone(), description);
						values.insert(key, choice.value);
					}
					(
						q.name,
						QuestionKind::Choice(values),
						google::Question::Choice {
							instructions: q.instructions,
							criteria,
						},
					)
				},
				d::QuestionParam::Score(q) => {
					validate_option_count(q.levels.len(), "score levels")?;
					let mut labels = Vec::new();
					let criteria = q
						.levels
						.into_iter()
						.map(|level| {
							labels.push(level.label.clone());
							let criterion = match level.description {
								Some(description) => {
									serde_json::json!({"label": level.label, "description": description}).to_string()
								},
								None => level.label,
							};
							validate_nonempty_criterion(&criterion)?;
							Ok(criterion)
						})
						.collect::<Result<_, AIError>>()?;
					(
						q.name,
						QuestionKind::Score(labels),
						google::Question::Score {
							instructions: q.instructions,
							criteria,
						},
					)
				},
			};
			let key = format!("q_{i}");
			questions.insert(key.clone(), question);
			state.questions.insert(key, QuestionState { name, kind });
		}
		let body = serde_json::to_vec(&google::Request {
			model,
			input,
			questions,
		})
		.map_err(AIError::RequestMarshal)?;
		Ok((body, state))
	}

	pub fn translate_response(bytes: &[u8], state: &State) -> Result<Vec<u8>, AIError> {
		let resp: google::Response = serde_json::from_slice(bytes).map_err(AIError::ResponseParsing)?;
		let mut answers = Vec::new();
		for (key, question, answer) in take_answers(resp.answers, &state.questions)? {
			let name = question.name.clone();
			let answer = match (answer, &question.kind) {
				(google::Answer::Noul { noul }, QuestionKind::Predicate) => {
					validate_probability(noul, "noul")?;
					d::AnswerResource::Predicate(d::PredicateAnswer {
						name,
						probability: noul,
					})
				},
				(
					google::Answer::Choice {
						choice,
						probabilities,
						confidence,
					},
					QuestionKind::Choice(values),
				) => {
					let (choice, probabilities) =
						checked_choice(values, &choice, &probabilities, confidence)?;
					d::AnswerResource::Choice(d::ChoiceAnswer {
						name,
						choice,
						probabilities: probabilities
							.into_iter()
							.map(|(value, probability)| d::ChoiceProbability { value, probability })
							.collect(),
						confidence,
					})
				},
				(
					google::Answer::Score {
						score,
						probabilities,
						confidence,
					},
					QuestionKind::Score(labels),
				) => {
					let (score, probabilities) =
						checked_score(&score, &probabilities, confidence, labels.len())?;
					d::AnswerResource::Score(d::ScoreAnswer {
						name,
						score,
						probabilities: labels
							.iter()
							.zip(probabilities)
							.enumerate()
							.map(|(index, (label, probability))| d::ScoreProbability {
								value: index as i64,
								label: label.clone(),
								probability,
							})
							.collect(),
						confidence,
					})
				},
				_ => return Err(answer_type_mismatch(key)),
			};
			answers.push(answer);
		}
		let input_tokens = resp.usage.total_input_tokens;
		let output_tokens = resp.usage.total_output_tokens;
		serde_json::to_vec(&d::DecisionResponse {
			model: state.model.clone(),
			answers,
			usage: d::DecisionUsage {
				input_tokens,
				output_tokens,
				total_tokens: input_tokens + output_tokens,
				input_tokens_details: Default::default(),
				output_tokens_details: Default::default(),
			},
		})
		.map_err(AIError::ResponseMarshal)
	}

	fn choice_description(value: &d::ChoiceValue, description: Option<String>) -> String {
		let description = description.filter(|description| !description.trim().is_empty());
		match (value, description) {
			(d::ChoiceValue::String(_), Some(description)) => description,
			(d::ChoiceValue::String(value), None) => format!("String value {value:?}."),
			(d::ChoiceValue::Boolean(value), Some(description)) => {
				format!("Boolean value {value}. {description}")
			},
			(d::ChoiceValue::Boolean(value), None) => format!("Boolean value {value}."),
		}
	}
}

pub fn translate_error_to_decisions(
	bytes: &bytes::Bytes,
	status: http::StatusCode,
) -> Result<bytes::Bytes, AIError> {
	let error = parse_google_error(bytes)?.error;
	let error_type = error_type(&error, status);
	let code = error_code(&error);
	let body = serde_json::json!({
		"error": {
			"type": error_type,
			"message": error.message,
			"code": code,
		}
	});
	serde_json::to_vec(&body)
		.map(bytes::Bytes::from)
		.map_err(AIError::ResponseMarshal)
}

pub fn translate_error_to_systemone(bytes: &bytes::Bytes) -> Result<bytes::Bytes, AIError> {
	let error = parse_google_error(bytes)?.error;
	let code = error_code(&error);
	let body = serde_json::json!({
		"detail": {
			"code": code,
			"message": error.message,
		}
	});
	serde_json::to_vec(&body)
		.map(bytes::Bytes::from)
		.map_err(AIError::ResponseMarshal)
}

fn error_code(error: &GoogleError) -> Option<String> {
	error
		.code
		.as_str()
		.map(str::to_string)
		.or_else(|| error.status.clone())
		.or_else(|| error.code.as_i64().map(|code| code.to_string()))
}

/// Uses Google's status when it names a known type, otherwise the HTTP status.
fn error_type(error: &GoogleError, status: http::StatusCode) -> &'static str {
	match google_error_type(error) {
		"api_error" => match status {
			http::StatusCode::BAD_REQUEST => "invalid_request_error",
			http::StatusCode::UNAUTHORIZED | http::StatusCode::FORBIDDEN => "authentication_error",
			http::StatusCode::NOT_FOUND => "not_found_error",
			http::StatusCode::TOO_MANY_REQUESTS => "rate_limit_error",
			_ => "api_error",
		},
		error_type => error_type,
	}
}

fn normalize_decisions_input(request: &mut Value) -> Result<(), AIError> {
	let request = request
		.as_object_mut()
		.ok_or_else(|| AIError::UnsupportedConversion("Decisions request must be an object".into()))?;
	if let Some(field) = request.keys().find(|field| {
		!matches!(
			field.as_str(),
			"model" | "input" | "questions" | "safety_identifier"
		)
	}) {
		return Err(AIError::UnsupportedConversion(
			format!("unsupported Decisions field {field:?}").into(),
		));
	}
	let Some(Value::Array(items)) = request.get_mut("input") else {
		return Ok(());
	};
	for item in items {
		let item = item.as_object_mut().ok_or(AIError::UnsupportedContent)?;
		if item
			.get("type")
			.and_then(Value::as_str)
			.is_some_and(|kind| kind != "message")
		{
			return Err(AIError::UnsupportedContent);
		}
		let user = item.get("role").and_then(Value::as_str).unwrap_or("user") == "user";
		let Some(Value::Array(parts)) = item.get_mut("content") else {
			continue;
		};
		for part in parts {
			let Some(kind) = part.get("type").and_then(Value::as_str) else {
				return Err(AIError::UnsupportedContent);
			};
			match kind {
				"input_text" => {},
				"input_json" if user => {
					let data = part
						.get("data")
						.ok_or_else(|| AIError::UnsupportedConversion("input_json is missing data".into()))?;
					let text = serde_json::to_string(data).map_err(AIError::RequestMarshal)?;
					*part = serde_json::json!({"type": "input_text", "text": text});
				},
				_ => return Err(AIError::UnsupportedContent),
			}
		}
	}
	Ok(())
}

fn option_key(index: usize) -> String {
	format!("o_{index:03}")
}

fn validate_question_count(count: usize, maximum: usize) -> Result<(), AIError> {
	if !(1..=maximum).contains(&count) {
		return Err(AIError::UnsupportedConversion(
			format!("Google Decisions conversion accepts 1 to {maximum} questions, got {count}").into(),
		));
	}
	Ok(())
}

fn validate_option_count(count: usize, field: &str) -> Result<(), AIError> {
	if !(2..=255).contains(&count) {
		return Err(AIError::UnsupportedConversion(
			format!("Google Decisions requires 2 to 255 {field}, got {count}").into(),
		));
	}
	Ok(())
}

fn validate_nonempty_criterion(criterion: &str) -> Result<(), AIError> {
	if criterion.trim().is_empty() {
		return Err(AIError::UnsupportedConversion(
			"Google Decisions score levels cannot be empty".into(),
		));
	}
	Ok(())
}

fn validate_probability(value: f64, field: &str) -> Result<(), AIError> {
	if !value.is_finite() || !(0.0..=1.0).contains(&value) {
		return Err(AIError::UnsupportedConversion(
			format!("invalid Google decision {field} probability {value}").into(),
		));
	}
	Ok(())
}

/// Pairs each requested question with its Google answer, in request order.
fn take_answers<Q>(
	mut answers: IndexMap<String, google::Answer>,
	questions: &IndexMap<String, Q>,
) -> Result<Vec<(&str, &Q, google::Answer)>, AIError> {
	let paired = questions
		.iter()
		.map(|(key, question)| {
			let answer = answers.shift_remove(key).ok_or_else(|| {
				AIError::UnsupportedConversion(format!("missing Google decision answer {key:?}").into())
			})?;
			Ok((key.as_str(), question, answer))
		})
		.collect::<Result<Vec<_>, AIError>>()?;
	if !answers.is_empty() {
		return Err(AIError::UnsupportedConversion(
			"unexpected Google decision answers".into(),
		));
	}
	Ok(paired)
}

fn answer_type_mismatch(key: &str) -> AIError {
	AIError::UnsupportedConversion(format!("Google decision answer type mismatch for {key:?}").into())
}

/// Checks a Google choice answer and maps its option keys back to the client's values.
fn checked_choice<V: Clone>(
	values: &IndexMap<String, V>,
	choice: &str,
	probabilities: &IndexMap<String, f64>,
	confidence: f64,
) -> Result<(V, Vec<(V, f64)>), AIError> {
	validate_probability(confidence, "confidence")?;
	validate_distribution(probabilities, values.keys())?;
	let choice = values.get(choice).cloned().ok_or_else(|| {
		AIError::UnsupportedConversion(format!("unknown Google decision choice {choice:?}").into())
	})?;
	let probabilities = values
		.iter()
		.map(|(key, value)| (value.clone(), probabilities[key]))
		.collect();
	Ok((choice, probabilities))
}

/// Checks a Google score answer and returns the score with probabilities in level order.
fn checked_score(
	score: &serde_json::Number,
	probabilities: &IndexMap<String, f64>,
	confidence: f64,
	levels: usize,
) -> Result<(f64, Vec<f64>), AIError> {
	validate_probability(confidence, "confidence")?;
	let score = score
		.as_f64()
		.filter(|score| score.is_finite())
		.ok_or_else(|| AIError::UnsupportedConversion("invalid Google decision score".into()))?;
	if !(0.0..=(levels - 1) as f64).contains(&score) {
		return Err(AIError::UnsupportedConversion(
			format!("Google decision score {score} is outside 0..{}", levels - 1).into(),
		));
	}
	let keys = (0..levels).map(|i| i.to_string()).collect::<Vec<_>>();
	validate_distribution(probabilities, &keys)?;
	Ok((score, keys.iter().map(|key| probabilities[key]).collect()))
}

fn validate_distribution<'a>(
	probabilities: &IndexMap<String, f64>,
	expected: impl IntoIterator<Item = &'a String>,
) -> Result<(), AIError> {
	let expected = expected.into_iter().collect::<Vec<_>>();
	if probabilities.len() != expected.len()
		|| expected.iter().any(|key| !probabilities.contains_key(*key))
	{
		return Err(AIError::UnsupportedConversion(
			"Google decision probabilities do not match the requested outcomes".into(),
		));
	}
	for probability in probabilities.values() {
		validate_probability(*probability, "outcome")?;
	}
	let sum = probabilities.values().sum::<f64>();
	if (sum - 1.0).abs() > 0.02 {
		return Err(AIError::UnsupportedConversion(
			format!("Google decision probabilities sum to {sum}, expected 1").into(),
		));
	}
	Ok(())
}

// TypeSafe documents these formulas; OpenAI does not document a confidence formula.
// Callers pass at least two probabilities, checked by validate_option_count on the request.
fn choice_confidence(probabilities: &[f64]) -> f64 {
	let uniform = 1.0 / probabilities.len() as f64;
	let top = probabilities.iter().copied().fold(0.0, f64::max);
	((top - uniform) / (1.0 - uniform)).clamp(0.0, 1.0)
}

/// `probabilities` are in level order.
fn score_confidence(probabilities: &[f64]) -> f64 {
	let levels = probabilities.len();
	let mut peak = 0;
	for i in 1..levels {
		if probabilities[i] > probabilities[peak] {
			peak = i;
		}
	}
	let spread: f64 = probabilities
		.iter()
		.enumerate()
		.map(|(i, p)| p * i.abs_diff(peak) as f64)
		.sum();
	let middle = (levels - 1) as f64 / 2.0;
	let uniform_spread = (0..levels).map(|i| (i as f64 - middle).abs()).sum::<f64>() / levels as f64;
	(1.0 - spread / uniform_spread).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
	use serde_json::json;

	use super::*;

	#[test]
	fn confidence_uses_typesafe_formulas() {
		assert!((choice_confidence(&[0.7, 0.3]) - 0.4).abs() < 1e-10);
		assert!((score_confidence(&[0.1, 0.7, 0.2]) - 0.55).abs() < 1e-10);
		assert_eq!(score_confidence(&[0.5, 0.0, 0.5]), 0.0);
		// Score probabilities follow level order, independent of the provider's map order.
		let score = IndexMap::from([("2".into(), 0.2), ("0".into(), 0.1), ("1".into(), 0.7)]);
		let (_, ordered) = checked_score(&1.into(), &score, 0.5, 3).unwrap();
		assert_eq!(ordered, [0.1, 0.7, 0.2]);
	}

	#[test]
	fn malformed_answers_are_rejected() {
		let req = serde_json::from_value(json!({"model": "gemini-decision-1", "state": "text", "questions": {"q": {"type": "noul", "instructions": "yes?"}}})).unwrap();
		let (_, state) = from_systemone::translate(&req).unwrap();
		let missing_usage = json!({"answers": {"q_0": {"type": "noul", "noul": 0.5}}});
		assert!(matches!(
			from_systemone::translate_response(&serde_json::to_vec(&missing_usage).unwrap(), &state),
			Err(AIError::ResponseParsing(_))
		));

		for response in [
			json!({"answers": {}, "usage": {"total_input_tokens": 1, "total_output_tokens": 0}}),
			json!({"answers": {"q_0": {"type": "score", "score": 1, "probabilities": {}, "confidence": 0}}, "usage": {"total_input_tokens": 1, "total_output_tokens": 0}}),
			json!({"answers": {"q_0": {"type": "noul", "noul": 0.5}, "extra": {"type": "noul", "noul": 0.5}}, "usage": {"total_input_tokens": 1, "total_output_tokens": 0}}),
		] {
			assert!(matches!(
				from_systemone::translate_response(&serde_json::to_vec(&response).unwrap(), &state),
				Err(AIError::UnsupportedConversion(_))
			));
		}
	}

	#[test]
	fn invalid_requests_are_rejected() {
		for request in [
			json!({"model": "gemini-decision-1", "state": "text", "questions": {}}),
			json!({"model": "gemini-decision-1", "state": "text", "questions": {"q": {"type": "choice", "instructions": "pick", "criteria": {"only": null}}}}),
			json!({"model": "gemini-decision-1", "state": "text", "questions": {"q": {"type": "score", "instructions": "rate", "criteria": ["only"]}}}),
			json!({"model": "gemini-decision-1", "state": "text", "questions": {"q": {"type": "noul", "instructions": "yes?"}}, "temperature": 0}),
		] {
			let request = serde_json::from_value(request).unwrap();
			assert!(matches!(
				from_systemone::translate(&request),
				Err(AIError::UnsupportedConversion(_))
			));
		}

		let request = serde_json::from_value(json!({
			"model": "gemini-decision-1",
			"input": "text",
			"questions": [{"type": "choice", "instructions": "pick", "choices": [{"value": "only"}]}]
		}))
		.unwrap();
		assert!(matches!(
			from_decisions::translate(&request),
			Err(AIError::UnsupportedConversion(_))
		));

		let request = serde_json::from_value(json!({
			"model": "gemini-decision-1",
			"input": "text",
			"questions": [{"type": "predicate", "instructions": "yes?"}],
			"temperature": 0
		}))
		.unwrap();
		assert!(matches!(
			from_decisions::translate(&request),
			Err(AIError::UnsupportedConversion(_))
		));
	}

	#[test]
	fn malformed_choice_and_score_answers_are_rejected() {
		let choice_request = serde_json::from_value(json!({
			"model": "gemini-decision-1", "state": "text",
			"questions": {"q": {"type": "choice", "instructions": "pick", "criteria": {"a": null, "b": null}}}
		})).unwrap();
		let (_, choice_state) = from_systemone::translate(&choice_request).unwrap();
		let sparse_distribution = json!({
			"answers": {"q_0": {"type": "choice", "choice": "o_000", "probabilities": {"o_000": 1.0}, "confidence": 1.0}},
			"usage": {"total_input_tokens": 1, "total_output_tokens": 0}
		});
		let error = from_systemone::translate_response(
			&serde_json::to_vec(&sparse_distribution).unwrap(),
			&choice_state,
		)
		.unwrap_err();
		assert!(error.to_string().contains("probabilities do not match"));

		let unknown_choice = json!({
			"answers": {"q_0": {"type": "choice", "choice": "unknown", "probabilities": {"o_000": 0.5, "o_001": 0.5}, "confidence": 1.0}},
			"usage": {"total_input_tokens": 1, "total_output_tokens": 0}
		});
		let error = from_systemone::translate_response(
			&serde_json::to_vec(&unknown_choice).unwrap(),
			&choice_state,
		)
		.unwrap_err();
		assert!(error.to_string().contains("unknown Google decision choice"));

		let score_request = serde_json::from_value(json!({
			"model": "gemini-decision-1", "input": "text",
			"questions": [{"type": "score", "instructions": "rate", "levels": [{"label": "low"}, {"label": "high"}]}]
		})).unwrap();
		let (_, score_state) = from_decisions::translate(&score_request).unwrap();
		let response = json!({
			"answers": {"q_0": {"type": "score", "score": 99, "probabilities": {"0": 0.5, "1": 0.5}, "confidence": 0.5}},
			"usage": {"total_input_tokens": 1, "total_output_tokens": 0}
		});
		assert!(
			from_decisions::translate_response(&serde_json::to_vec(&response).unwrap(), &score_state)
				.is_err()
		);
	}

	#[test]
	fn confidence_matches_each_client_contract() {
		let systemone_request = serde_json::from_value(json!({
			"model": "gemini-decision-1",
			"state": "text",
			"questions": {"q": {"type": "choice", "instructions": "pick", "criteria": {"a": null, "b": null}}}
		}))
		.unwrap();
		let (_, systemone_state) = from_systemone::translate(&systemone_request).unwrap();
		let decisions_request = serde_json::from_value(json!({
			"model": "gemini-decision-1",
			"input": "text",
			"questions": [{"type": "choice", "instructions": "pick", "choices": [{"value": "a"}, {"value": "b"}]}]
		}))
		.unwrap();
		let (_, decisions_state) = from_decisions::translate(&decisions_request).unwrap();
		let response = serde_json::to_vec(&json!({
			"answers": {"q_0": {"type": "choice", "choice": "o_000", "probabilities": {"o_000": 0.7, "o_001": 0.3}, "confidence": 0.9}},
			"usage": {"total_input_tokens": 1, "total_output_tokens": 0}
		}))
		.unwrap();

		let systemone: Value = serde_json::from_slice(
			&from_systemone::translate_response(&response, &systemone_state).unwrap(),
		)
		.unwrap();
		let decisions: Value = serde_json::from_slice(
			&from_decisions::translate_response(&response, &decisions_state).unwrap(),
		)
		.unwrap();
		assert!((systemone["answers"]["q"]["confidence"].as_f64().unwrap() - 0.4).abs() < 1e-10);
		assert_eq!(decisions["answers"][0]["confidence"], 0.9);
	}

	#[test]
	fn google_errors_are_translated_to_client_shapes() {
		let numeric = bytes::Bytes::from_static(
			br#"{"error":{"code":401,"message":"bad key","status":"UNAUTHENTICATED"}}"#,
		);
		let openai: Value = serde_json::from_slice(
			&translate_error_to_decisions(&numeric, http::StatusCode::UNAUTHORIZED).unwrap(),
		)
		.unwrap();
		assert_eq!(
			openai,
			json!({"error": {"type": "authentication_error", "message": "bad key", "code": "UNAUTHENTICATED"}})
		);

		let string = bytes::Bytes::from_static(br#"{"error":{"code":"safety","message":"blocked"}}"#);
		let systemone: Value =
			serde_json::from_slice(&translate_error_to_systemone(&string).unwrap()).unwrap();
		assert_eq!(
			systemone,
			json!({"detail": {"code": "safety", "message": "blocked"}})
		);
	}

	#[test]
	fn unsupported_media_is_rejected() {
		let req = serde_json::from_value(json!({"model": "gemini-decision-1", "state": "{image}", "assets": {"image": "data:image/png;base64,AA=="}, "questions": {}})).unwrap();
		assert!(matches!(
			from_systemone::translate(&req),
			Err(AIError::UnsupportedContent)
		));
		let req = serde_json::from_value(json!({"model": "gemini-decision-1", "input": [{"role": "user", "content": [{"type": "input_image", "image_url": "https://example.com/image.png"}]}], "questions": []})).unwrap();
		assert!(matches!(
			from_decisions::translate(&req),
			Err(AIError::UnsupportedContent)
		));
	}

	#[test]
	fn blank_choice_descriptions_use_the_value() {
		let req = serde_json::from_value(json!({
			"model": "gemini-decision-1",
			"state": "text",
			"questions": {"q": {"type": "choice", "instructions": "pick", "criteria": {
				"first": "", "second": "   "
			}}}
		}))
		.unwrap();
		let (body, _) = from_systemone::translate(&req).unwrap();
		let body: Value = serde_json::from_slice(&body).unwrap();
		assert_eq!(body["questions"]["q_0"]["criteria"]["o_000"], "first");
		assert_eq!(body["questions"]["q_0"]["criteria"]["o_001"], "second");

		let req = serde_json::from_value(json!({
			"model": "gemini-decision-1",
			"input": "text",
			"questions": [{"type": "choice", "instructions": "pick", "choices": [
				{"value": "first", "description": ""},
				{"value": true, "description": "   "}
			]}]
		}))
		.unwrap();
		let (body, _) = from_decisions::translate(&req).unwrap();
		let body: Value = serde_json::from_slice(&body).unwrap();
		assert_eq!(
			body["questions"]["q_0"]["criteria"]["o_000"],
			"String value \"first\"."
		);
		assert_eq!(
			body["questions"]["q_0"]["criteria"]["o_001"],
			"Boolean value true."
		);
	}
}
