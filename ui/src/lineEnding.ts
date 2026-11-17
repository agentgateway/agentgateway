export type LineEnding = 'LF' | 'CRLF';

/** Line ending of a file's text, so an editor can keep it when the text is saved verbatim. */
export function detectLineEnding(text: string): LineEnding {
	return text.includes('\r\n') ? 'CRLF' : 'LF';
}
