//! Anthropic-specific streaming logic for Bedrock responses
//!
//! This module handles the conversion of Bedrock's EventStream responses
//! into Anthropic's native SSE format, including proper business logic
//! for content blocks, tool usage, and error handling.

use anyhow::{Result, anyhow};
use bytes::Bytes;
use serde_json;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::llm::anthropic_types::{self as anthropic, ContentDelta, StreamEvent};
use crate::llm::bedrock::types::{self as bedrock, ConverseStreamOutput};

/// Maximum size for tool JSON buffers to prevent memory exhaustion (64KB)
const MAX_TOOL_JSON_BUFFER_SIZE: usize = 64 * 1024;

/// Stream event processor that converts Bedrock events to Anthropic SSE format
pub struct BedrockStreamProcessor {
	/// Current message ID (generated since Bedrock doesn't provide one)
	message_id: String,

	/// Current model name
	model: String,

	/// Buffer for accumulating tool input JSON strings by content block index
	tool_json_buffers: HashMap<usize, String>,

	/// Track content block metadata for correlation
	content_block_metadata: HashMap<usize, ContentBlockMetadata>,

	/// Whether we've seen the first token (for timing metrics)
	seen_first_token: bool,

	/// Accumulated usage information
	current_usage: Option<anthropic::Usage>,
}

/// Metadata for tracking content blocks during streaming
#[derive(Debug, Clone)]
struct ContentBlockMetadata {
	pub block_type: ContentBlockType,
	#[allow(dead_code)]
	pub tool_use_id: Option<String>,
	#[allow(dead_code)]
	pub tool_name: Option<String>,
}

/// Types of content blocks we're tracking
#[derive(Debug, Clone, PartialEq)]
enum ContentBlockType {
	Text,
	ToolUse,
	Reasoning,
	#[allow(dead_code)]
	Citations,
}

impl BedrockStreamProcessor {
	/// Create a new stream processor
	pub fn new(message_id: String, model: String) -> Self {
		Self {
			message_id,
			model,
			tool_json_buffers: HashMap::new(),
			content_block_metadata: HashMap::new(),
			seen_first_token: false,
			current_usage: None,
		}
	}

	/// Process a Bedrock stream event and convert to Anthropic events
	/// Returns Vec because some Bedrock events may produce multiple Anthropic events
	pub fn process_event(&mut self, bedrock_event: ConverseStreamOutput) -> Result<Vec<StreamEvent>> {
		let mut events = Vec::new();

		match bedrock_event {
			ConverseStreamOutput::MessageStart(start_event) => {
				let event = self.handle_message_start(start_event)?;
				events.push(event);
			},

			ConverseStreamOutput::ContentBlockStart(start_event) => {
				let event = self.handle_content_block_start(start_event)?;
				events.push(event);
			},

			ConverseStreamOutput::ContentBlockDelta(delta_event) => {
				let _index = delta_event.content_block_index;

				if let Some(event) = self.handle_content_block_delta(delta_event)? {
					events.push(event);
				}
			},

			ConverseStreamOutput::ContentBlockStop(stop_event) => {
				if let Some(event) = self.handle_content_block_stop(stop_event)? {
					events.push(event);
				}
			},

			ConverseStreamOutput::MessageStop(stop_event) => {
				let event = self.handle_message_stop(stop_event)?;
				events.push(event);
			},

			ConverseStreamOutput::Metadata(metadata_event) => {
				// Update usage and potentially emit final events
				self.handle_metadata(metadata_event)?;
				// Metadata doesn't generate Anthropic events directly
			},

			// Error events - convert to Anthropic error format
			ConverseStreamOutput::InternalServerException(error_event) => {
				let error_event =
					self.handle_stream_error("internal_server_error", &error_event.message)?;
				events.push(error_event);
			},

			ConverseStreamOutput::ModelStreamErrorException(error_event) => {
				let error_event = self.handle_stream_error("model_error", &error_event.message)?;
				events.push(error_event);
			},

			ConverseStreamOutput::ServiceUnavailableException(error_event) => {
				let error_event = self.handle_stream_error("service_unavailable", &error_event.message)?;
				events.push(error_event);
			},

			ConverseStreamOutput::ThrottlingException(error_event) => {
				let error_event = self.handle_stream_error("rate_limit_error", &error_event.message)?;
				events.push(error_event);
			},

			ConverseStreamOutput::ValidationException(error_event) => {
				let error_event =
					self.handle_stream_error("invalid_request_error", &error_event.message)?;
				events.push(error_event);
			},
		}

		Ok(events)
	}

	/// Handle Bedrock MessageStart → Anthropic message_start
	fn handle_message_start(
		&mut self,
		_start_event: bedrock::MessageStartEvent,
	) -> Result<StreamEvent> {
		// Create initial message with empty content for message_start
		let message = anthropic::MessagesResponse {
			id: self.message_id.clone(),
			r#type: "message".to_string(),
			role: "assistant".to_string(),
			content: Vec::new(),
			model: self.model.clone(),
			stop_reason: None,
			stop_sequence: None,
			usage: anthropic::Usage {
				input_tokens: 0,
				output_tokens: 0,
				cache_creation_input_tokens: None,
				cache_read_input_tokens: None,
				cache_creation: None,
				server_tool_use: None,
				service_tier: None,
			},
			container: None,
		};

		Ok(StreamEvent::MessageStart { message })
	}

	/// Handle Bedrock ContentBlockStart → Anthropic content_block_start
	fn handle_content_block_start(
		&mut self,
		start_event: bedrock::ContentBlockStartEvent,
	) -> Result<StreamEvent> {
		let index = start_event.content_block_index;

		let (content_block, metadata) = match start_event.start {
			bedrock::ContentBlockStart::ToolUse(tool_start) => {
				let metadata = ContentBlockMetadata {
					block_type: ContentBlockType::ToolUse,
					tool_use_id: Some(tool_start.tool_use_id.clone()),
					tool_name: Some(tool_start.name.clone()),
				};

				let content_block =
					anthropic::ResponseContentBlock::ToolUse(anthropic::ResponseToolUseBlock {
						id: tool_start.tool_use_id,
						name: tool_start.name,
						input: serde_json::Value::Object(serde_json::Map::new()), // Empty initially
					});

				(content_block, metadata)
			},

			bedrock::ContentBlockStart::Text(_) => {
				let metadata = ContentBlockMetadata {
					block_type: ContentBlockType::Text,
					tool_use_id: None,
					tool_name: None,
				};

				let content_block = anthropic::ResponseContentBlock::Text(anthropic::ResponseTextBlock {
					text: String::new(),
					citations: None,
				});

				(content_block, metadata)
			},

			bedrock::ContentBlockStart::Reasoning(_) => {
				let metadata = ContentBlockMetadata {
					block_type: ContentBlockType::Reasoning,
					tool_use_id: None,
					tool_name: None,
				};

				let content_block =
					anthropic::ResponseContentBlock::Thinking(anthropic::ResponseThinkingBlock {
						thinking: String::new(),
						signature: String::new(),
					});

				(content_block, metadata)
			},
		};

		// Store metadata for delta processing
		self.content_block_metadata.insert(index, metadata);

		Ok(StreamEvent::ContentBlockStart {
			index,
			content_block,
		})
	}

	/// Handle Bedrock ContentBlockDelta → Anthropic content_block_delta
	fn handle_content_block_delta(
		&mut self,
		delta_event: bedrock::ContentBlockDeltaEvent,
	) -> Result<Option<StreamEvent>> {
		let index = delta_event.content_block_index;

		// Mark first token seen for timing
		if !self.seen_first_token {
			self.seen_first_token = true;
		}

		let delta = match delta_event.delta {
			bedrock::ContentBlockDelta::Text { text } => ContentDelta::TextDelta { text },

			bedrock::ContentBlockDelta::ToolUse {
				tool_use: tool_delta,
			} => {
				// Accumulate partial JSON for tool inputs with bounds checking
				let json_buffer = self
					.tool_json_buffers
					.entry(index)
					.or_insert_with(String::new);
				let _old_length = json_buffer.len();

				// Check if adding this delta would exceed the maximum buffer size
				if json_buffer.len() + tool_delta.input.len() > MAX_TOOL_JSON_BUFFER_SIZE {
					return Err(anyhow!(
						"Tool JSON buffer for content block {} exceeded maximum size of {} bytes",
						index,
						MAX_TOOL_JSON_BUFFER_SIZE
					));
				}

				json_buffer.push_str(&tool_delta.input);

				// Return the partial JSON as input_json_delta
				ContentDelta::InputJsonDelta {
					partial_json: tool_delta.input,
				}
			},

			bedrock::ContentBlockDelta::ReasoningContent(reasoning_delta) => {
				// Map reasoning content to thinking deltas
				match reasoning_delta {
					bedrock::ReasoningContentBlockDelta::Text(text) => {
						ContentDelta::ThinkingDelta { thinking: text }
					},
				}
			},

			bedrock::ContentBlockDelta::Citation(_citation_delta) => {
				// Citations are typically accumulated and attached to text blocks
				// For now, we'll skip them in the streaming interface
				return Ok(None);
			},
		};
		Ok(Some(StreamEvent::ContentBlockDelta { index, delta }))
	}

	/// Handle Bedrock ContentBlockStop → Anthropic content_block_stop
	fn handle_content_block_stop(
		&mut self,
		stop_event: bedrock::ContentBlockStopEvent,
	) -> Result<Option<StreamEvent>> {
		let index = stop_event.content_block_index;

		// If this was a tool use block, validate the accumulated JSON
		if let Some(metadata) = self.content_block_metadata.get(&index) {
			if metadata.block_type == ContentBlockType::ToolUse {
				if let Some(json_buffer) = self.tool_json_buffers.remove(&index) {
					// Try to parse the complete JSON to validate it
					match serde_json::from_str::<serde_json::Value>(&json_buffer) {
						Ok(_parsed) => {
							debug!("Successfully assembled tool input JSON for block {}", index);
						},
						Err(e) => {
							warn!("Invalid JSON assembled for tool block {}: {}", index, e);
							// Continue anyway - the client may be able to handle partial JSON
						},
					}
				}
			}
		}

		// Clean up metadata
		self.content_block_metadata.remove(&index);

		Ok(Some(StreamEvent::ContentBlockStop { index }))
	}

	/// Handle Bedrock MessageStop → Anthropic message_stop
	fn handle_message_stop(&mut self, stop_event: bedrock::MessageStopEvent) -> Result<StreamEvent> {
		// Convert stop reason
		let stop_reason = match stop_event.stop_reason {
			bedrock::StopReason::EndTurn => anthropic::StopReason::EndTurn,
			bedrock::StopReason::ToolUse => anthropic::StopReason::ToolUse,
			bedrock::StopReason::MaxTokens => anthropic::StopReason::MaxTokens,
			bedrock::StopReason::StopSequence => anthropic::StopReason::StopSequence,
			bedrock::StopReason::GuardrailIntervened => anthropic::StopReason::Refusal,
			bedrock::StopReason::ContentFiltered => anthropic::StopReason::Refusal,
		};

		let delta = anthropic::MessageDelta {
			stop_reason: Some(stop_reason),
			stop_sequence: None, // Bedrock doesn't provide matched sequence details
			usage: self.current_usage.clone(),
		};

		Ok(StreamEvent::MessageDelta { delta })
	}

	/// Handle Bedrock Metadata events
	fn handle_metadata(
		&mut self,
		metadata_event: bedrock::ConverseStreamMetadataEvent,
	) -> Result<()> {
		if let Some(bedrock_usage) = metadata_event.usage {
			self.current_usage = Some(anthropic::Usage {
				input_tokens: bedrock_usage.input_tokens as u32,
				output_tokens: bedrock_usage.output_tokens as u32,
				cache_creation_input_tokens: bedrock_usage.cache_write_input_tokens.map(|t| t as u32),
				cache_read_input_tokens: bedrock_usage.cache_read_input_tokens.map(|t| t as u32),
				cache_creation: None,
				server_tool_use: None,
				service_tier: None,
			});
		}

		Ok(())
	}

	/// Handle stream error events
	fn handle_stream_error(&self, error_type: &str, message: &str) -> Result<StreamEvent> {
		Ok(StreamEvent::Error {
			error: anthropic::ErrorResponse {
				error_type: error_type.to_string(),
				message: message.to_string(),
			},
		})
	}

	/// Finalize the stream and return the final message_stop event
	pub fn finalize(&mut self) -> Result<StreamEvent> {
		// Clean up any remaining buffers
		if !self.tool_json_buffers.is_empty() {
			warn!(
				"Stream ended with {} incomplete tool JSON buffers",
				self.tool_json_buffers.len()
			);
			self.tool_json_buffers.clear();
		}

		Ok(StreamEvent::MessageStop)
	}
}

/// Serialize Anthropic StreamEvent to SSE format
pub fn serialize_anthropic_event_to_sse(event: &StreamEvent) -> anyhow::Result<Bytes> {
	let event_type = match event {
		StreamEvent::MessageStart { .. } => "message_start",
		StreamEvent::ContentBlockStart { .. } => "content_block_start",
		StreamEvent::Ping => "ping",
		StreamEvent::ContentBlockDelta { .. } => "content_block_delta",
		StreamEvent::ContentBlockStop { .. } => "content_block_stop",
		StreamEvent::MessageDelta { .. } => "message_delta",
		StreamEvent::MessageStop => "message_stop",
		StreamEvent::Error { .. } => "error",
	};

	let json_data = serde_json::to_string(event)?;
	let sse_frame = format!("event: {}\ndata: {}\n\n", event_type, json_data);
	Ok(Bytes::from(sse_frame))
}

/// Transform AWS EventStream to Anthropic SSE format using proper architectural separation
/// This function creates the bridge between generic aws_sse parsing and Anthropic business logic
pub fn transform_bedrock_to_anthropic_sse(
	body: crate::http::Body,
	message_id: String,
	model: String,
) -> crate::http::Body {
	use futures_util::StreamExt;
	use http_body::Frame;
	use http_body_util::StreamBody;
	use tokio_stream::wrappers::ReceiverStream;
	use tracing::{Instrument, info_span};

	// Create channel for SSE frames with buffer to prevent backpressure
	let (sse_tx, sse_rx) = mpsc::channel::<Bytes>(256);

	// Convert receiver to stream and then to http::Body
	let receiver_stream = ReceiverStream::new(sse_rx)
		.map(|bytes| Ok::<Frame<Bytes>, std::convert::Infallible>(Frame::data(bytes)));
	let response_body = StreamBody::new(receiver_stream);
	let http_body = crate::http::Body::new(response_body);

	// Create span for the streaming task
	let processing_span = info_span!("bedrock_to_anthropic_transform",
		message_id = %message_id,
		model = %model,
		channel_buffer_size = 256
	);

	// Spawn task with proper instrumentation to preserve tracing context
	tokio::spawn(
		process_bedrock_to_anthropic_stream(body, sse_tx, message_id, model)
			.instrument(processing_span),
	);

	http_body
}

/// Process Bedrock stream and convert to Anthropic SSE format
async fn process_bedrock_to_anthropic_stream(
	body_stream: crate::http::Body,
	sse_tx: mpsc::Sender<Bytes>,
	message_id: String,
	model: String,
) {
	use aws_event_stream_parser::EventStreamCodec;
	use http_body_util::BodyExt;
	use tokio_util::codec::Decoder;
	use tracing::{error, info};

	let mut decoder = EventStreamCodec;
	let mut decode_buffer = bytes::BytesMut::new();
	let mut body_stream = body_stream;
	let mut processor = BedrockStreamProcessor::new(message_id, model);
	let mut events_processed = 0u32;
	let mut decode_errors = 0u32;

	// Helper to send SSE event with backpressure handling
	async fn send_sse_event(tx: &mpsc::Sender<Bytes>, event: &StreamEvent) -> bool {
		match serialize_anthropic_event_to_sse(event) {
			Ok(sse_bytes) => {
				// Use send().await for backpressure handling - waits instead of failing
				match tx.send(sse_bytes).await {
					Ok(()) => true,
					Err(_) => {
						error!("SSE channel closed, stopping stream processing");
						false
					},
				}
			},
			Err(e) => {
				error!("Failed to serialize Anthropic event to SSE: {}", e);
				false
			},
		}
	}

	// Process body stream
	while let Some(frame_result) = body_stream.frame().await {
		match frame_result {
			Ok(frame) => {
				if let Some(data) = frame.data_ref() {
					decode_buffer.extend_from_slice(data);

					// Decode as many complete messages as possible
					loop {
						match decoder.decode(&mut decode_buffer) {
							Ok(Some(message)) => {
								events_processed += 1;

								// Convert Bedrock event to Anthropic events using business logic
								match crate::llm::bedrock::types::ConverseStreamOutput::deserialize(message) {
									Ok(bedrock_event) => {
										// Process event through our stream processor
										match processor.process_event(bedrock_event) {
											Ok(anthropic_events) => {
												// Send each event as SSE
												for event in anthropic_events {
													if !send_sse_event(&sse_tx, &event).await {
														return; // Channel closed, stop processing
													}
												}
											},
											Err(e) => {
												error!("Failed to process Bedrock event: {}", e);
											},
										}
									},
									Err(e) => {
										error!("Failed to parse Bedrock event: {}", e);
									},
								}
							},
							Ok(None) => {
								// Need more data
								break;
							},
							Err(e) => {
								decode_errors += 1;
								error!("Failed to decode EventStream message: {}", e);
								break;
							},
						}
					}
				}
			},
			Err(e) => {
				error!("Error reading body frame: {}", e);
				break;
			},
		}
	}

	info!(
		"Completed Bedrock->Anthropic stream processing: {} events, {} errors",
		events_processed, decode_errors
	);
}
