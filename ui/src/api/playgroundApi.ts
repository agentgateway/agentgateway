import { parseJsonOrText } from '@/api/base';

type StreamingToolCall = {
	index: number;
	id?: string;
	function: { name: string; arguments: string };
};

/**
 * Streams an OpenAI-compatible chat completion, invoking `onDelta` for each
 * content delta as it arrives, and returning an aggregated response shaped like
 * a non-streaming completion (message content, tool calls, usage) so callers
 * can reuse the same post-processing.
 *
 * Falls back to a single JSON body when the server does not stream
 * (non-SSE content type), still reporting its content through `onDelta`.
 */
export async function streamChatCompletion(args: {
	baseUrl: string;
	model: string;
	apiKey?: string;
	messages: Array<Record<string, unknown>>;
	tools?: unknown[];
	onDelta: (delta: string) => void;
}) {
	const url = `${args.baseUrl.replace(/\/$/, '')}/v1/chat/completions`;
	const headers: Record<string, string> = {
		'Content-Type': 'application/json'
	};
	if (args.apiKey) headers.Authorization = `Bearer ${args.apiKey}`;
	const response = await fetch(url, {
		method: 'POST',
		headers,
		body: JSON.stringify({
			model: args.model,
			messages: args.messages,
			...(args.tools?.length ? { tools: args.tools, tool_choice: 'auto' } : {}),
			stream: true,
			// Ask the gateway to include token usage in the final SSE event.
			stream_options: { include_usage: true }
		})
	});
	if (!response.ok) {
		const text = await response.text();
		throw new Error(text || `${response.status} ${response.statusText}`);
	}
	const contentType = response.headers.get('content-type') ?? '';
	if (!response.body || !contentType.includes('text/event-stream')) {
		const text = await response.text();
		const body = parseJsonOrText(text);
		const content = singleResponseContent(body);
		if (content) args.onDelta(content);
		return body;
	}

	let content = '';
	let finishReason: string | undefined;
	let usage: unknown;
	const toolCalls: StreamingToolCall[] = [];
	const handleData = (payload: string) => {
		if (payload === '[DONE]') return;
		let chunk: unknown;
		try {
			chunk = JSON.parse(payload);
		} catch {
			return;
		}
		const record = chunk as {
			choices?: Array<{
				delta?: {
					content?: unknown;
					tool_calls?: Array<{
						index?: number;
						id?: string;
						function?: { name?: string; arguments?: string };
					}>;
				};
				finish_reason?: string;
			}>;
			usage?: unknown;
		};
		if (record.usage) usage = record.usage;
		const choice = record.choices?.[0];
		if (!choice) return;
		if (choice.finish_reason) finishReason = choice.finish_reason;
		const deltaContent = choice.delta?.content;
		if (typeof deltaContent === 'string' && deltaContent) {
			content += deltaContent;
			args.onDelta(deltaContent);
		}
		for (const call of choice.delta?.tool_calls ?? []) {
			const index = call.index ?? toolCalls.length;
			let aggregated = toolCalls.find(entry => entry.index === index);
			if (!aggregated) {
				aggregated = { index, function: { name: '', arguments: '' } };
				toolCalls.push(aggregated);
			}
			if (call.id) aggregated.id = call.id;
			if (call.function?.name) aggregated.function.name += call.function.name;
			if (call.function?.arguments) aggregated.function.arguments += call.function.arguments;
		}
	};

	// Line-buffered SSE parsing: process each complete `data:` line as it
	// arrives. Tolerates chunks splitting a line mid-stream and CRLF endings.
	const reader = response.body.getReader();
	const decoder = new TextDecoder();
	let buffer = '';
	const consumeLines = (flush: boolean) => {
		let frameStart = buffer.indexOf('\n');
		while (frameStart !== -1) {
			const line = buffer.slice(0, frameStart).replace(/\r$/, '');
			buffer = buffer.slice(frameStart + 1);
			if (line.startsWith('data:')) {
				const payload = line.slice(5).trim();
				if (payload) handleData(payload);
			}
			frameStart = buffer.indexOf('\n');
		}
		if (flush && buffer.trim()) {
			const line = buffer.replace(/\r$/, '');
			if (line.startsWith('data:')) {
				const payload = line.slice(5).trim();
				if (payload) handleData(payload);
			}
			buffer = '';
		}
	};
	while (true) {
		const { done, value } = await reader.read();
		if (done) break;
		buffer += decoder.decode(value, { stream: true });
		consumeLines(false);
	}
	buffer += decoder.decode();
	consumeLines(true);

	return {
		id: 'chatcmpl-stream',
		choices: [
			{
				message: {
					role: 'assistant',
					content: content || null,
					...(toolCalls.length
						? {
								tool_calls: toolCalls.map(({ id, function: fn }) => ({
									id,
									type: 'function',
									function: fn
								}))
							}
						: {})
				},
				finish_reason: finishReason ?? null
			}
		],
		...(usage ? { usage } : {})
	};
}

function singleResponseContent(body: unknown) {
	if (!body || typeof body !== 'object') return '';
	const choices = (body as { choices?: unknown }).choices;
	if (!Array.isArray(choices)) return '';
	const content = (choices[0] as { message?: { content?: unknown } })?.message?.content;
	return typeof content === 'string' ? content : '';
}

export async function sendChatCompletion(args: {
	baseUrl: string;
	model: string;
	apiKey?: string;
	messages: Array<Record<string, unknown>>;
	tools?: unknown[];
	stream?: boolean;
}) {
	const url = `${args.baseUrl.replace(/\/$/, '')}/v1/chat/completions`;
	const headers: Record<string, string> = {
		'Content-Type': 'application/json'
	};
	if (args.apiKey) headers.Authorization = `Bearer ${args.apiKey}`;
	const response = await fetch(url, {
		method: 'POST',
		headers,
		body: JSON.stringify({
			model: args.model,
			messages: args.messages,
			...(args.tools?.length ? { tools: args.tools, tool_choice: 'auto' } : {}),
			stream: Boolean(args.stream)
		})
	});
	const text = await response.text();
	if (!response.ok) {
		throw new Error(text || `${response.status} ${response.statusText}`);
	}
	return parseJsonOrText(text);
}

export async function sendMcpJsonRpc(args: {
	baseUrl: string;
	sessionId?: string;
	bearerToken?: string;
	body: unknown;
}) {
	const headers: Record<string, string> = {
		Accept: 'application/json, text/event-stream',
		'Content-Type': 'application/json'
	};
	if (args.sessionId) headers['mcp-session-id'] = args.sessionId;
	if (args.bearerToken?.trim()) headers.Authorization = `Bearer ${args.bearerToken.trim()}`;
	let response: Response;
	try {
		response = await fetch(args.baseUrl, {
			method: 'POST',
			headers,
			body: JSON.stringify(args.body)
		});
	} catch (err) {
		throw new Error(playgroundFetchErrorMessage(args.baseUrl, err));
	}
	const text = await response.text();
	if (!response.ok) {
		throw new Error(
			text || `MCP gateway returned ${response.status} ${response.statusText} from ${args.baseUrl}`
		);
	}
	return {
		sessionId: response.headers.get('mcp-session-id'),
		body: parseMcpResponse(text, response.headers.get('content-type') ?? ''),
		status: response.status
	};
}

function playgroundFetchErrorMessage(url: string, err: unknown) {
	const browserMessage = err instanceof Error ? err.message : String(err);
	return `Could not reach ${url}: ${browserMessage}`;
}

function parseMcpResponse(text: string, contentType: string) {
	if (!text.trim()) return null;
	if (contentType.includes('text/event-stream')) {
		const events = text
			.split('\n\n')
			.map(event =>
				event
					.split('\n')
					.filter(line => line.startsWith('data:'))
					.map(line => line.slice(5).trim())
					.join('\n')
			)
			.filter(Boolean)
			.map(parseJsonOrText);
		return events.length === 1 ? events[0] : events;
	}
	return parseJsonOrText(text);
}
