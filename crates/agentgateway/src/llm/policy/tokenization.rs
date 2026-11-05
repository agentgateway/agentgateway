use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use http_body_util::BodyDataStream;
use serde_json::{Value, json};
use tokio_sse_codec::{Frame, SseDecoder, SseEncoder};
use tokio_util::codec::{Decoder, Encoder};

use super::pii;
use crate::llm::AIError;
use crate::llm::types::{RequestType, ResponseType};
use crate::*;

#[apply(schema!)]
pub struct Tokenization {
	/// Replace standard email addresses in Chat Completions text with request-scoped tokens and restore them in replies, including streams.
	#[serde(default)]
	pub email: bool,
}

pub struct EmailTokens {
	prefix: String,
	values: Vec<String>,
}

impl std::fmt::Debug for EmailTokens {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("EmailTokens")
			.field("count", &self.values.len())
			.finish_non_exhaustive()
	}
}

impl EmailTokens {
	pub fn tokenize(req: &mut impl RequestType) -> Result<Arc<Self>, AIError> {
		let mut tokens = Self {
			prefix: format!("<AGW_EMAIL_{}_", uuid::Uuid::new_v4().simple()),
			values: Vec::new(),
		};
		let mut indices = HashMap::<String, usize>::new();
		let mut overflow = false;
		req.visit_text_mut(&mut |_, text| {
			let mut output = String::new();
			let mut end = 0;
			for found in pii::recognizer(&pii::EMAIL, text) {
				let email = &text[found.start..found.end];
				let index = if let Some(index) = indices.get(email) {
					*index
				} else {
					if tokens.values.len() == 4096 {
						overflow = true;
						return;
					}
					let index = tokens.values.len();
					tokens.values.push(email.to_owned());
					indices.insert(email.to_owned(), index);
					index
				};
				output.push_str(&text[end..found.start]);
				output.push_str(&format!("{}{index}>", tokens.prefix));
				end = found.end;
			}
			if end != 0 {
				output.push_str(&text[end..]);
				*text = output;
			}
		});
		if overflow {
			return Err(AIError::RequestTooLarge);
		}
		Ok(Arc::new(tokens))
	}

	pub fn restore_response(&self, response: &mut dyn ResponseType) {
		response.visit_text_mut(&mut |context, text| {
			if !context.signed {
				*text = self.restore(text, &mut String::new(), true);
			}
		});
	}

	fn restore(&self, text: &str, pending: &mut String, finished: bool) -> String {
		let input = std::mem::take(pending) + text;
		let mut rest = input.as_str();
		let mut output = String::new();
		while let Some(start) = rest.find(&self.prefix) {
			output.push_str(&rest[..start]);
			rest = &rest[start..];
			let tail = &rest[self.prefix.len()..];
			let digits_end = tail.bytes().take_while(u8::is_ascii_digit).count();
			let suffix = &tail[digits_end..];
			if digits_end > 0 && suffix.starts_with('>') {
				let digits = &tail[..digits_end];
				let size = self.prefix.len() + digits_end + 1;
				let value = digits
					.parse::<usize>()
					.ok()
					.filter(|index| index.to_string() == digits)
					.and_then(|index| self.values.get(index));
				output.push_str(value.map(String::as_str).unwrap_or(&rest[..size]));
				rest = &rest[size..];
			} else if !finished && digits_end <= 20 && suffix.is_empty() {
				pending.push_str(rest);
				return output;
			} else {
				output.push_str(&rest[..self.prefix.len()]);
				rest = tail;
			}
		}
		let hold = if finished {
			0
		} else {
			(1..self.prefix.len())
				.rev()
				.find(|&n| rest.ends_with(&self.prefix[..n]))
				.unwrap_or(0)
		};
		output.push_str(&rest[..rest.len() - hold]);
		pending.push_str(&rest[rest.len() - hold..]);
		output
	}

	pub fn restore_stream(
		self: Arc<Self>,
		body: agent_http::RawBody,
		limit: usize,
	) -> agent_http::RawBody {
		if self.values.is_empty() {
			return body;
		}
		let state = SseRestorer {
			body: BodyDataStream::new(body),
			decoder: SseDecoder::<Bytes>::with_max_size(limit.max(8)),
			encoder: SseEncoder::new(),
			buffer: BytesMut::new(),
			tokens: StreamTokens {
				tokens: self,
				pending: BTreeMap::new(),
			},
			eof: false,
		};
		agent_http::RawBody::from_stream(futures_util::stream::try_unfold(
			state,
			move |mut state| async move {
				loop {
					let frame = if state.eof {
						state.decoder.decode_eof(&mut state.buffer)
					} else {
						state.decoder.decode(&mut state.buffer)
					}
					.map_err(|_| stream_error())?;
					if let Some(mut frame) = frame {
						if let Frame::Event(event) = &mut frame {
							if event.data.as_ref() == b"[DONE]" {
								state.tokens.check_finished()?;
							} else {
								let mut value: Value =
									serde_json::from_slice(&event.data).map_err(|_| stream_error())?;
								state.tokens.restore_event(&mut value)?;
								event.data = serde_json::to_vec(&value)
									.map_err(|_| stream_error())?
									.into();
							}
						}
						let mut encoded = BytesMut::new();
						state
							.encoder
							.encode(frame, &mut encoded)
							.map_err(|_| stream_error())?;
						return Ok(Some((encoded.freeze(), state)));
					}
					if state.eof {
						state.tokens.check_finished()?;
						return Ok(None);
					}
					match state.body.next().await {
						Some(Ok(bytes)) => state.buffer.extend_from_slice(&bytes),
						Some(Err(_)) => return Err(stream_error()),
						None => state.eof = true,
					}
				}
			},
		))
	}
}

struct SseRestorer {
	body: BodyDataStream<agent_http::RawBody>,
	decoder: SseDecoder<Bytes>,
	encoder: SseEncoder,
	buffer: BytesMut,
	tokens: StreamTokens,
	eof: bool,
}

fn stream_error() -> std::io::Error {
	std::io::Error::other("invalid or incomplete tokenized Chat Completions stream")
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Channel {
	Text(String),
	Tool(u64, String, String),
	Function,
}

struct StreamTokens {
	tokens: Arc<EmailTokens>,
	pending: BTreeMap<(u64, Channel), String>,
}

impl StreamTokens {
	fn check_finished(&self) -> Result<(), std::io::Error> {
		if self.pending.values().any(|value| !value.is_empty()) {
			return Err(stream_error());
		}
		Ok(())
	}

	fn restore_text(
		&mut self,
		choice: u64,
		channel: Channel,
		value: &mut Value,
	) -> Result<(), std::io::Error> {
		let Some(text) = value.as_str() else {
			return Ok(());
		};
		let key = (choice, channel);
		if self.pending.len() >= 1024 && !self.pending.contains_key(&key) {
			return Err(stream_error());
		}
		let pending = self.pending.entry(key).or_default();
		*value = self.tokens.restore(text, pending, false).into();
		Ok(())
	}

	fn restore_event(&mut self, event: &mut Value) -> Result<(), std::io::Error> {
		if event.get("error").is_some() {
			self.pending.clear();
			return Ok(());
		}
		let Some(choices) = event.get_mut("choices").and_then(Value::as_array_mut) else {
			return Ok(());
		};
		for choice in choices {
			let index = choice
				.get("index")
				.and_then(Value::as_u64)
				.ok_or_else(stream_error)?;
			let finished = choice.get("finish_reason").is_some_and(|v| !v.is_null());
			if choice.get("delta").is_none() {
				choice["delta"] = json!({});
			}
			let delta = &mut choice["delta"];
			if !delta.is_object() {
				return Err(stream_error());
			}
			for field in ["content", "reasoning_content", "reasoning", "refusal"] {
				if let Some(value) = delta.get_mut(field) {
					self.restore_text(index, Channel::Text(field.to_owned()), value)?;
				}
			}
			if let Some(calls) = delta.get_mut("tool_calls").and_then(Value::as_array_mut) {
				for call in calls {
					if !call.is_object() {
						return Err(stream_error());
					}
					let tool = call
						.get("index")
						.and_then(Value::as_u64)
						.ok_or_else(stream_error)?;
					for (kind, field) in [("function", "arguments"), ("custom", "input")] {
						if let Some(value) = call.get_mut(kind).and_then(|v| v.get_mut(field)) {
							self.restore_text(
								index,
								Channel::Tool(tool, kind.to_owned(), field.to_owned()),
								value,
							)?;
						}
					}
				}
			}
			if let Some(value) = delta
				.get_mut("function_call")
				.and_then(|v| v.get_mut("arguments"))
			{
				self.restore_text(index, Channel::Function, value)?;
			}
			if finished {
				let keys: Vec<_> = self
					.pending
					.keys()
					.filter(|(i, _)| *i == index)
					.cloned()
					.collect();
				for key in keys {
					let tail = self.pending.remove(&key).unwrap_or_default();
					if tail.is_empty() {
						continue;
					}
					let target = match key.1 {
						Channel::Text(field) => &mut delta[field],
						Channel::Function => {
							if delta.get("function_call").is_some_and(|v| !v.is_object()) {
								return Err(stream_error());
							}
							if delta.get("function_call").is_none() {
								delta["function_call"] = json!({});
							}
							&mut delta["function_call"]["arguments"]
						},
						Channel::Tool(tool, kind, field) => {
							if delta.get("tool_calls").is_none() {
								delta["tool_calls"] = json!([]);
							}
							let calls = delta["tool_calls"]
								.as_array_mut()
								.ok_or_else(stream_error)?;
							let pos = calls.iter().position(|c| c["index"].as_u64() == Some(tool));
							let pos = pos.unwrap_or_else(|| {
								calls.push(json!({"index":tool}));
								calls.len() - 1
							});
							if calls[pos].get(&kind).is_some_and(|v| !v.is_object()) {
								return Err(stream_error());
							}
							if calls[pos].get(&kind).is_none() {
								calls[pos][&kind] = json!({});
							}
							&mut calls[pos][kind][field]
						},
					};
					*target = (target.as_str().unwrap_or_default().to_owned() + &tail).into();
				}
			}
		}
		Ok(())
	}
}

#[cfg(test)]
#[path = "tokenization_tests.rs"]
mod tests;
