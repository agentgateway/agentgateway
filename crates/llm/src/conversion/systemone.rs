pub mod from_decisions {
	use std::collections::HashSet;

	use async_openai::types::decisions as d;
	use indexmap::IndexMap;

	use crate::AIError;
	use crate::types::decisions;
	use crate::types::systemone::typed as systemone;

	/// Request details SystemOne cannot represent, used to restore them on the response.
	/// Keyed by SystemOne question key.
	#[derive(Debug, Default)]
	pub struct State {
		/// Keyed by SystemOne question key, in request order.
		questions: IndexMap<String, QuestionState>,
	}

	#[derive(Debug, Default)]
	struct QuestionState {
		name: Option<String>,
		/// Choice values that were booleans.
		boolean_values: HashSet<String>,
		/// Score level labels, by level.
		labels: Vec<String>,
	}

	pub fn translate(req: &decisions::Request) -> Result<(Vec<u8>, State), AIError> {
		let req = super::super::typed_decisions_request(req)?;
		let state = decisions_input_value(req.input)?;
		let translated = req
			.questions
			.into_iter()
			.map(|q| {
				let mut qs = QuestionState::default();
				let question = match q {
					d::QuestionParam::Predicate(q) => {
						qs.name = q.name;
						systemone::Question::Noul {
							instructions: super::super::decisions_text_instructions(q.instructions)?.into(),
							criteria: None,
						}
					},
					d::QuestionParam::Choice(q) => {
						qs.name = q.name;
						let mut criteria = IndexMap::new();
						for c in q.choices {
							let value = match c.value {
								d::ChoiceValueParam::Text(s) => s,
								d::ChoiceValueParam::Boolean(b) => {
									qs.boolean_values.insert(b.to_string());
									b.to_string()
								},
							};
							if criteria
								.insert(value.clone(), c.description.map(serde_json::Value::String))
								.is_some()
							{
								return Err(AIError::UnsupportedConversion(
									format!("duplicate choice value {value:?}").into(),
								));
							}
						}
						systemone::Question::Choice {
							instructions: super::super::decisions_text_instructions(q.instructions)?.into(),
							criteria,
						}
					},
					d::QuestionParam::Score(q) => {
						qs.name = q.name;
						let criteria = q
							.levels
							.into_iter()
							.map(|l| {
								qs.labels.push(l.label.clone());
								match l.description {
									Some(description) => {
										serde_json::json!({"label": l.label, "description": description})
									},
									None => serde_json::Value::String(l.label),
								}
							})
							.collect();
						systemone::Question::Score {
							instructions: super::super::decisions_text_instructions(q.instructions)?.into(),
							criteria,
						}
					},
				};
				Ok((qs, question))
			})
			.collect::<Result<Vec<_>, AIError>>()?;
		// Unnamed questions are keyed by index, avoiding explicit names.
		let names: HashSet<String> = translated
			.iter()
			.filter_map(|(qs, _)| qs.name.clone())
			.collect();
		let mut response_state = State::default();
		let mut questions = IndexMap::new();
		for (i, (qs, question)) in translated.into_iter().enumerate() {
			let key = match &qs.name {
				Some(name) => name.clone(),
				None => {
					let mut key = i.to_string();
					while names.contains(&key) || questions.contains_key(&key) {
						key.push('_');
					}
					key
				},
			};
			if questions.insert(key.clone(), question).is_some() {
				return Err(AIError::UnsupportedConversion(
					format!("duplicate question name {key:?}").into(),
				));
			}
			response_state.questions.insert(key, qs);
		}
		let req = systemone::Request {
			model: req.model,
			state,
			questions,
		};
		let body = serde_json::to_vec(&req).map_err(AIError::RequestMarshal)?;
		Ok((body, response_state))
	}

	/// SystemOne state takes any JSON, so messages keep their roles and tool items stay objects.
	fn decisions_input_value(input: d::DecisionInput) -> Result<serde_json::Value, AIError> {
		use serde_json::json;

		use super::super::{
			assistant_parts, instruction_parts, tool_name, tool_output_parts, user_parts,
		};

		let items = match input {
			d::DecisionInput::Text(text) => return Ok(serde_json::Value::String(text)),
			d::DecisionInput::Items(items) => items,
		};
		items
			.into_iter()
			.map(|item| {
				Ok(match item {
					d::DecisionInputItem::Message(d::DecisionInputMessage::User(message)) => {
						json!({"role": "user", "content": parts_text(user_parts(message.content)?)?})
					},
					d::DecisionInputItem::Message(d::DecisionInputMessage::Developer(message)) => {
						json!({"role": "developer", "content": parts_text(instruction_parts(message.content))?})
					},
					d::DecisionInputItem::Message(d::DecisionInputMessage::System(message)) => {
						json!({"role": "system", "content": parts_text(instruction_parts(message.content))?})
					},
					d::DecisionInputItem::Message(d::DecisionInputMessage::Assistant(message)) => {
						json!({"role": "assistant", "content": parts_text(assistant_parts(message.content))?})
					},
					d::DecisionInputItem::FunctionCall(call) => json!({
						"type": "function_call",
						"call_id": call.call_id,
						"name": tool_name(call.namespace, call.name),
						"arguments": call.arguments,
					}),
					d::DecisionInputItem::FunctionCallOutput(output) => json!({
						"type": "function_call_output",
						"call_id": output.call_id,
						"output": parts_text(tool_output_parts(output.output))?,
					}),
					d::DecisionInputItem::CustomToolCall(call) => json!({
						"type": "custom_tool_call",
						"call_id": call.call_id,
						"name": tool_name(call.namespace, call.name),
						"input": call.input,
					}),
					d::DecisionInputItem::CustomToolCallOutput(output) => json!({
						"type": "custom_tool_call_output",
						"call_id": output.call_id,
						"output": parts_text(tool_output_parts(output.output))?,
					}),
				})
			})
			.collect()
	}

	/// SystemOne state takes no images here.
	fn parts_text(parts: Vec<super::super::HistoryPart>) -> Result<String, AIError> {
		parts
			.into_iter()
			.map(|part| match part {
				super::super::HistoryPart::Text(text) => Ok(text),
				super::super::HistoryPart::Image(_) => Err(AIError::UnsupportedContent),
			})
			.collect::<Result<Vec<_>, _>>()
			.map(|texts| texts.join("\n"))
	}

	pub fn translate_response(bytes: &[u8], state: &State) -> Result<Vec<u8>, AIError> {
		let resp: systemone::Response =
			serde_json::from_slice(bytes).map_err(AIError::ResponseParsing)?;
		let mut source = resp.answers;
		let answers = state
			.questions
			.iter()
			.map(|(key, qs)| {
				let name = qs.name.clone();
				// SystemOne answers every question. A missing answer is reported as a refusal so the
				// answers after it keep their positions.
				let Some(answer) = source.shift_remove(key) else {
					return d::AnswerResource::Refusal(d::AnswerResourceRefusal { name });
				};
				let choice_value = |value: String| match value.parse() {
					Ok(b) if qs.boolean_values.contains(&value) => d::ChoiceValueResource::Boolean(b),
					_ => d::ChoiceValueResource::Text(value),
				};
				match answer {
					systemone::Answer::Noul { noul } => {
						d::AnswerResource::Predicate(d::AnswerResourcePredicate {
							name,
							probability: noul,
						})
					},
					systemone::Answer::Choice {
						choice,
						probabilities,
						confidence,
					} => d::AnswerResource::Choice(d::AnswerResourceChoice {
						name,
						choice: choice_value(choice),
						probabilities: probabilities
							.into_iter()
							.map(|(value, probability)| d::ChoiceProbabilityResource {
								value: choice_value(value),
								probability,
							})
							.collect(),
						confidence,
					}),
					systemone::Answer::Score {
						score,
						probabilities,
						confidence,
						..
					} => d::AnswerResource::Score(d::AnswerResourceScore {
						name,
						score: score.as_f64().unwrap_or_default(),
						probabilities: probabilities
							.into_iter()
							.map(|(level, probability)| {
								let value: usize = level.parse().unwrap_or_default();
								d::ScoreProbabilityResource {
									label: qs.labels.get(value).cloned().unwrap_or_default(),
									value: value as _,
									probability,
								}
							})
							.collect(),
						confidence,
					}),
				}
			})
			.collect();
		let (input_tokens, output_tokens) = resp
			.usage
			.map_or((0, 0), |u| (u.input_tokens, u.output_tokens));
		let resp = d::DecisionResponse {
			model: resp.model,
			answers,
			usage: d::ResponseUsage {
				input_tokens,
				input_tokens_details: d::InputTokenDetails {
					cache_write_tokens: Some(0),
					..Default::default()
				},
				output_tokens,
				output_tokens_details: Default::default(),
				total_tokens: input_tokens + output_tokens,
			},
		};
		serde_json::to_vec(&resp).map_err(AIError::ResponseMarshal)
	}
}

pub mod from_systemone {
	use std::collections::HashMap;

	use async_openai::types::decisions as d;
	use indexmap::IndexMap;

	use crate::conversion::value_text as text;
	use crate::types::systemone::{Request, typed as systemone};
	use crate::{AIError, json};

	/// Request details Decisions cannot represent, used to restore them on the response.
	#[derive(Debug, Default)]
	pub struct State {
		/// Score criteria, by question key, to rebuild the legend.
		legends: HashMap<String, Vec<serde_json::Value>>,
	}

	pub fn translate(req: &Request) -> Result<(Vec<u8>, State), AIError> {
		let req = json::convert::<_, systemone::Request>(req)
			.map_err(|err| AIError::RequestParsing(crate::InputFormat::SystemOne, err))?;
		let input = d::DecisionInput::Text(text(req.state));
		let mut state = State::default();
		let questions = req
			.questions
			.into_iter()
			.map(|(name, q)| match q {
				systemone::Question::Noul {
					instructions,
					criteria,
				} => {
					let mut instructions = text(instructions);
					if let Some(c) = criteria {
						instructions.push('\n');
						for (k, v) in [("true", c.true_), ("false", c.false_)] {
							if let Some(v) = v {
								instructions.push_str(&format!("\n{k}: {}", text(v)));
							}
						}
					}
					d::QuestionParam::Predicate(d::QuestionParamPredicate {
						name: Some(name),
						instructions: d::QuestionParamInstructions::Text(instructions),
					})
				},
				systemone::Question::Choice {
					instructions,
					criteria,
				} => d::QuestionParam::Choice(d::QuestionParamChoice {
					name: Some(name),
					instructions: d::QuestionParamInstructions::Text(text(instructions)),
					choices: criteria
						.into_iter()
						.map(|(value, description)| d::ChoiceOptionParam {
							value: d::ChoiceValueParam::Text(value),
							description: description.map(text),
						})
						.collect(),
				}),
				systemone::Question::Score {
					instructions,
					criteria,
				} => {
					state.legends.insert(name.clone(), criteria.clone());
					d::QuestionParam::Score(d::QuestionParamScore {
						name: Some(name),
						instructions: d::QuestionParamInstructions::Text(text(instructions)),
						levels: criteria
							.into_iter()
							.map(|c| match c {
								serde_json::Value::Object(mut o)
									if o.get("label").is_some_and(|l| l.is_string()) =>
								{
									let label = o.remove("label").map(text).unwrap_or_default();
									let description = match o.len() {
										0 => None,
										1 if o.contains_key("description") => o.remove("description").map(text),
										_ => Some(serde_json::Value::Object(o).to_string()),
									};
									d::ScoreLevelParam { label, description }
								},
								c => d::ScoreLevelParam {
									label: text(c),
									description: None,
								},
							})
							.collect(),
					})
				},
			})
			.collect();
		let req = d::DecisionRequest {
			model: req.model,
			input,
			questions,
			safety_identifier: None,
			prompt_cache_key: None,
			prompt_cache_options: None,
		};
		let body = serde_json::to_vec(&req).map_err(AIError::RequestMarshal)?;
		Ok((body, state))
	}

	pub fn translate_response(bytes: &[u8], state: &State) -> Result<Vec<u8>, AIError> {
		let resp: d::DecisionResponse =
			serde_json::from_slice(bytes).map_err(AIError::ResponseParsing)?;
		let choice_value = |v: d::ChoiceValueResource| match v {
			d::ChoiceValueResource::Text(s) => s,
			d::ChoiceValueResource::Boolean(b) => b.to_string(),
		};
		let mut answers = IndexMap::new();
		// Questions are always sent named, so every answer maps back to its key.
		for answer in resp.answers {
			let (name, answer) = match answer {
				d::AnswerResource::Predicate(a) => (
					a.name,
					systemone::Answer::Noul {
						noul: a.probability,
					},
				),
				d::AnswerResource::Choice(a) => (
					a.name,
					systemone::Answer::Choice {
						choice: choice_value(a.choice),
						probabilities: a
							.probabilities
							.into_iter()
							.map(|p| (choice_value(p.value), p.probability))
							.collect(),
						confidence: a.confidence,
					},
				),
				d::AnswerResource::Score(a) => {
					let criteria = a.name.as_ref().and_then(|n| state.legends.get(n));
					let legend = a
						.probabilities
						.iter()
						.map(|p| {
							let level = usize::try_from(p.value)
								.ok()
								.and_then(|i| criteria.and_then(|c| c.get(i)).cloned())
								.unwrap_or_else(|| serde_json::Value::String(p.label.clone()));
							(p.value.to_string(), level)
						})
						.collect();
					(
						a.name,
						systemone::Answer::Score {
							score: if a.score.fract() == 0.0 {
								(a.score as i64).into()
							} else {
								serde_json::Number::from_f64(a.score).unwrap_or_else(|| 0.into())
							},
							legend: Some(legend),
							probabilities: a
								.probabilities
								.into_iter()
								.map(|p| (p.value.to_string(), p.probability))
								.collect(),
							confidence: a.confidence,
						},
					)
				},
				d::AnswerResource::Refusal(a) => {
					return Err(AIError::UnsupportedConversion(
						format!(
							"decisions refusal for question {:?} has no SystemOne equivalent",
							a.name.unwrap_or_default()
						)
						.into(),
					));
				},
			};
			if let Some(name) = name {
				answers.insert(name, answer);
			}
		}
		let resp = systemone::Response {
			model: resp.model,
			answers,
			usage: Some(systemone::Usage {
				input_tokens: resp.usage.input_tokens,
				output_tokens: resp.usage.output_tokens,
			}),
		};
		serde_json::to_vec(&resp).map_err(AIError::ResponseMarshal)
	}
}
