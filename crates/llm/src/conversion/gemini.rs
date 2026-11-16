use indexmap::IndexMap;
use serde_json::Value;

use crate::conversion::value_text as text;
use crate::types::gemini::{GoogleError, GoogleErrorResponse, decisions as google};
use crate::types::{decisions, systemone};
use crate::{AIError, InputFormat, json, logged_response_parsing};

pub mod from_responses {
	use bytes::Bytes;

	use crate::AIError;

	pub fn translate_error(bytes: &Bytes) -> Result<Bytes, AIError> {
		super::super::completions::translate_google_error(bytes)
	}
}

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
		let req: systemone::typed::Request =
			json::convert(req).map_err(|err| AIError::RequestParsing(InputFormat::SystemOne, err))?;
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
						let description = match description
							.map(text)
							.filter(|description| !description.trim().is_empty())
						{
							Some(description) => format!("{value}: {description}"),
							None => value.clone(),
						};
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
					let translated = criteria.iter().cloned().map(text).collect();
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
			input: google::Input::Text(text(req.state)),
			questions,
		})
		.map_err(AIError::RequestMarshal)?;
		Ok((body, state))
	}

	pub fn translate_response(bytes: &[u8], state: &State) -> Result<Vec<u8>, AIError> {
		let resp: google::Response = serde_json::from_slice(bytes).map_err(AIError::ResponseParsing)?;
		let mut answers = IndexMap::new();
		for ((name, question), answer) in take_answers(resp.answers, &state.questions) {
			let answer = match (answer, question) {
				(google::Answer::Noul { noul }, QuestionState::Noul) => {
					systemone::typed::Answer::Noul { noul }
				},
				(
					google::Answer::Choice {
						choice,
						probabilities,
						..
					},
					QuestionState::Choice(values),
				) => {
					let Some((choice, probabilities)) = map_choice(values, &choice, &probabilities) else {
						continue;
					};
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
						..
					},
					QuestionState::Score(criteria),
				) => {
					let probabilities = level_probabilities(&probabilities, criteria.len());
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
				_ => continue,
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
		let input = translate_input(input)?;
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
							match level.description {
								Some(description) => format!("{}: {description}", level.label),
								None => level.label,
							}
						})
						.collect();
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
		for (question, answer) in take_answers(resp.answers, &state.questions) {
			let name = question.name.clone();
			let answer = match (answer, &question.kind) {
				(google::Answer::Noul { noul }, QuestionKind::Predicate) => {
					d::AnswerResource::Predicate(d::PredicateAnswer {
						name,
						probability: noul,
					})
				},
				(
					google::Answer::Choice {
						choice,
						probabilities,
						..
					},
					QuestionKind::Choice(values),
				) => {
					let Some((choice, probabilities)) = map_choice(values, &choice, &probabilities) else {
						continue;
					};
					let confidence =
						choice_confidence(&probabilities.iter().map(|(_, p)| *p).collect::<Vec<_>>());
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
						..
					},
					QuestionKind::Score(labels),
				) => {
					let Some(score) = score.as_f64() else {
						continue;
					};
					let probabilities = level_probabilities(&probabilities, labels.len());
					let confidence = score_confidence(&probabilities);
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
				_ => continue,
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

	fn translate_input(input: d::DecisionInput) -> Result<google::Input, AIError> {
		match input {
			d::DecisionInput::Text(text) => Ok(google::Input::Text(text)),
			d::DecisionInput::Messages(items) => items
				.into_iter()
				.map(|d::DecisionInputItem::Message(message)| {
					if message.role != "user" {
						return Err(AIError::UnsupportedContent);
					}
					let content = match message.content {
						d::DecisionInputContent::Text(text) => vec![google::Content::Text { text }],
						d::DecisionInputContent::Parts(parts) => parts
							.into_iter()
							.map(|part| match part {
								d::DecisionInputContentPart::InputText(text) => {
									Ok(google::Content::Text { text: text.text })
								},
								d::DecisionInputContentPart::InputImage(image) => translate_image(image),
							})
							.collect::<Result<_, _>>()?,
					};
					Ok(google::Step::UserInput { content })
				})
				.collect::<Result<Vec<_>, _>>()
				.map(google::Input::Steps),
		}
	}

	fn translate_image(image: d::DecisionInputImage) -> Result<google::Content, AIError> {
		let resolution = match image.detail {
			None | Some(d::ImageDetail::Auto) => None,
			Some(d::ImageDetail::Low) => Some(google::MediaResolution::Low),
			// Gemini's closest documented resolution to OpenAI's unscaled original.
			Some(d::ImageDetail::High | d::ImageDetail::Original) => Some(google::MediaResolution::High),
		};
		let source =
			if image.image_url.starts_with("https://") || image.image_url.starts_with("http://") {
				google::ImageSource::Uri {
					uri: image.image_url,
				}
			} else {
				let Some((mime_type, data)) = super::super::completions::parse_data_url(&image.image_url)
				else {
					return Err(AIError::UnsupportedContent);
				};
				google::ImageSource::Data {
					data: data.to_string(),
					mime_type: mime_type.to_string(),
				}
			};
		Ok(google::Content::Image { source, resolution })
	}

	fn choice_description(value: &d::ChoiceValue, description: Option<String>) -> String {
		let value = match value {
			d::ChoiceValue::String(value) => value.clone(),
			d::ChoiceValue::Boolean(value) => value.to_string(),
		};
		match description.filter(|description| !description.trim().is_empty()) {
			Some(description) => format!("{value}: {description}"),
			None => value,
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

/// Parse a Google error response, handling both single object and array-wrapped formats.
/// Google's OpenAI-compatible endpoints consistently return `[{"error": {...}}]`
/// rather than `{"error": {...}}` when using the Vertex AI shim.
pub(crate) fn parse_google_error(bytes: &bytes::Bytes) -> Result<GoogleErrorResponse, AIError> {
	serde_json::from_slice::<GoogleErrorResponse>(bytes).or_else(|_| {
		serde_json::from_slice::<Vec<GoogleErrorResponse>>(bytes)
			.map_err(logged_response_parsing(bytes))?
			.into_iter()
			.next()
			.ok_or_else(|| {
				AIError::InvalidResponse(agent_core::strng::literal!(
					"error response missing error details"
				))
			})
	})
}

/// Maps a Google canonical error status to the compatible client error type.
/// Definitions and HTTP mappings: https://cloud.google.com/service-infrastructure/docs/service-management/reference/rest/v1/Code
pub(crate) fn google_error_type(status: Option<&str>) -> &'static str {
	match status {
		Some("INVALID_ARGUMENT") | Some("FAILED_PRECONDITION") => "invalid_request_error",
		Some("NOT_FOUND") => "not_found_error",
		Some("PERMISSION_DENIED") | Some("UNAUTHENTICATED") => "authentication_error",
		Some("RESOURCE_EXHAUSTED") => "rate_limit_error",
		_ => "api_error",
	}
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
	match google_error_type(error.status.as_deref()) {
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
				"input_text" | "input_image" => {},
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

/// The confidence formulas need at least two outcomes; Gemini enforces its own upper limits.
fn validate_option_count(count: usize, field: &str) -> Result<(), AIError> {
	if count < 2 {
		return Err(AIError::UnsupportedConversion(
			format!("Google Decisions requires at least 2 {field}, got {count}").into(),
		));
	}
	Ok(())
}

/// Pairs each requested question with its Google answer, in request order.
/// Unanswered questions and answers to unknown keys are left out.
fn take_answers<Q>(
	mut answers: IndexMap<String, google::Answer>,
	questions: &IndexMap<String, Q>,
) -> Vec<(&Q, google::Answer)> {
	questions
		.iter()
		.filter_map(|(key, question)| Some((question, answers.shift_remove(key)?)))
		.collect()
}

/// Maps a Google choice answer back to the client's values, or `None` if the chosen key is unknown.
/// Every requested option gets a probability; options Google left out get 0.
fn map_choice<V: Clone>(
	values: &IndexMap<String, V>,
	choice: &str,
	probabilities: &IndexMap<String, f64>,
) -> Option<(V, Vec<(V, f64)>)> {
	let choice = values.get(choice)?.clone();
	let probabilities = values
		.iter()
		.map(|(key, value)| {
			(
				value.clone(),
				probabilities.get(key).copied().unwrap_or(0.0),
			)
		})
		.collect();
	Some((choice, probabilities))
}

/// Returns score probabilities in level order; levels Google left out get 0.
fn level_probabilities(probabilities: &IndexMap<String, f64>, levels: usize) -> Vec<f64> {
	(0..levels)
		.map(|i| probabilities.get(&i.to_string()).copied().unwrap_or(0.0))
		.collect()
}

// TypeSafe's adapter calculates these formulas. OpenAI does not formally specify them,
// but its published choice and score examples produce the same values.
// Callers pass at least two probabilities, checked by validate_option_count on the request.
fn choice_confidence(probabilities: &[f64]) -> f64 {
	let uniform = 1.0 / probabilities.len() as f64;
	let top = probabilities.iter().copied().fold(0.0, f64::max);
	((top - uniform) / (1.0 - uniform)).clamp(0.0, 1.0)
}

/// `probabilities` are in level order.
///
/// Gemini returned the maximum probability in testing, so translated clients recalculate confidence.
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
#[path = "gemini_tests.rs"]
mod tests;
