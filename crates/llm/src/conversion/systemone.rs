pub mod from_decisions {
	use std::collections::{HashMap, HashSet};

	use async_openai::types::decisions as d;
	use indexmap::IndexMap;

	use crate::types::{decisions, systemone};
	use crate::{AIError, json};

	/// Request details SystemOne cannot represent, used to restore them on the response.
	/// Keyed by SystemOne question key.
	#[derive(Debug, Default)]
	pub struct State {
		questions: HashMap<String, QuestionState>,
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
		let req = json::convert::<_, d::DecisionRequest>(req)
			.map_err(|err| AIError::RequestParsing(crate::InputFormat::Decisions, err))?;
		let state = match req.input {
			d::DecisionInput::Text(text) => serde_json::Value::String(text),
			d::DecisionInput::Messages(items) => items
				.into_iter()
				.map(|d::DecisionInputItem::Message(m)| {
					let content = match m.content {
						d::DecisionInputContent::Text(text) => text,
						d::DecisionInputContent::Parts(parts) => parts
							.into_iter()
							.map(|part| match part {
								d::DecisionInputContentPart::InputText(t) => Ok(t.text),
								d::DecisionInputContentPart::InputImage(_) => Err(AIError::UnsupportedContent),
							})
							.collect::<Result<Vec<_>, _>>()?
							.join("\n"),
					};
					Ok(serde_json::json!({"role": m.role, "content": content}))
				})
				.collect::<Result<_, AIError>>()?,
		};
		let translated = req
			.questions
			.into_iter()
			.map(|q| {
				let mut qs = QuestionState::default();
				let question = match q {
					d::QuestionParam::Predicate(q) => {
						qs.name = q.name;
						systemone::Question::Noul {
							instructions: q.instructions.into(),
							criteria: None,
						}
					},
					d::QuestionParam::Choice(q) => {
						qs.name = q.name;
						let mut criteria = IndexMap::new();
						for c in q.choices {
							let value = match c.value {
								d::ChoiceValue::String(s) => s,
								d::ChoiceValue::Boolean(b) => {
									qs.boolean_values.insert(b.to_string());
									b.to_string()
								},
							};
							if criteria.insert(value.clone(), c.description).is_some() {
								return Err(AIError::UnsupportedConversion(
									format!("duplicate choice value {value:?}").into(),
								));
							}
						}
						systemone::Question::Choice {
							instructions: q.instructions.into(),
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
							instructions: q.instructions.into(),
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

	pub fn translate_response(bytes: &[u8], state: &State) -> Result<Vec<u8>, AIError> {
		let resp: systemone::Response =
			serde_json::from_slice(bytes).map_err(AIError::ResponseParsing)?;
		let answers = resp
			.answers
			.into_iter()
			.map(|(key, answer)| {
				let qs = state.questions.get(&key);
				let choice_value = |value: String| match value.parse() {
					Ok(b) if qs.is_some_and(|qs| qs.boolean_values.contains(&value)) => {
						d::ChoiceValue::Boolean(b)
					},
					_ => d::ChoiceValue::String(value),
				};
				let name = match qs {
					Some(qs) => qs.name.clone(),
					None => Some(key.clone()),
				};
				match answer {
					systemone::Answer::Noul { noul } => d::AnswerResource::Predicate(d::PredicateAnswer {
						name: name.clone(),
						probability: noul,
					}),
					systemone::Answer::Choice {
						choice,
						probabilities,
						confidence,
					} => d::AnswerResource::Choice(d::ChoiceAnswer {
						name: name.clone(),
						choice: choice_value(choice),
						probabilities: probabilities
							.into_iter()
							.map(|(value, probability)| d::ChoiceProbability {
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
					} => d::AnswerResource::Score(d::ScoreAnswer {
						name: name.clone(),
						score,
						probabilities: probabilities
							.into_iter()
							.map(|(level, probability)| {
								let value: usize = level.parse().unwrap_or_default();
								d::ScoreProbability {
									label: qs
										.and_then(|qs| qs.labels.get(value).cloned())
										.unwrap_or_default(),
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
			usage: d::DecisionUsage {
				input_tokens,
				input_tokens_details: Default::default(),
				output_tokens,
				output_tokens_details: Default::default(),
				total_tokens: input_tokens + output_tokens,
			},
		};
		serde_json::to_vec(&resp).map_err(AIError::ResponseMarshal)
	}
}
