import type { ErrorObject } from 'ajv';
import Ajv2020 from 'ajv/dist/2020';

import configSchema from '@/generated/schema.json';
import { deepEqual } from '@/mergeEdits';
import type { GatewayConfig } from '@/types';

let validatorPromise: Promise<ReturnType<Ajv2020['compile']>> | null = null;

export async function validateGatewayConfig(config: GatewayConfig) {
	const errors = await getGatewayConfigValidationErrors(config);
	if (errors.length === 0) return;

	const messages = errors.slice(0, 5).map(error => {
		const path = error.instancePath || '/';
		return `${path}: ${error.message ?? 'invalid value'}`;
	});
	throw new Error(`Configuration validation failed: ${messages.join('; ')}`);
}

export async function getGatewayConfigValidationErrors(
	config: GatewayConfig
): Promise<ErrorObject[]> {
	const validate = await getValidator();
	if (validate(config)) return [];
	return validate.errors?.slice() ?? [];
}

async function getValidator() {
	validatorPromise ??= Promise.resolve().then(() => {
		const schema = structuredClone(configSchema);
		sanitizeEmptyEnums(schema);
		const ajv = new Ajv2020({
			allErrors: true,
			validateFormats: false,
			strict: false
		});
		return ajv.compile(schema);
	});
	return validatorPromise;
}

let subschemaAjv: Ajv2020 | null = null;
const narrowedValidators = new Map<string, ReturnType<Ajv2020['compile']>>();

export type ConfigValueError = {
	// Dotted path within the value, such as `jwks.url` or `request.0.webhook`.
	path: string;
	message: string;
};

// Validates a value against the config schema node at `path`, such as
// ['$defs', 'LocalLLMVirtualModel']. Where the schema allows several shapes
// (anyOf/oneOf), errors come from the shape the value is closest to, so a
// single-issuer JWT is not told it is missing `providers`. `fields` are the
// paths a form shows, which pick the shape when the value alone does not.
export function configValueErrors(
	path: string[],
	value: unknown,
	fields: string[] = []
): ConfigValueError[] {
	if (!subschemaAjv) {
		const schema = structuredClone(configSchema);
		sanitizeEmptyEnums(schema);
		subschemaAjv = new Ajv2020({ allErrors: true, validateFormats: false, strict: false });
		subschemaAjv.addSchema(schema, 'config');
	}
	// Narrowing only picks which errors to report; validity comes from the
	// full schema, which a narrowed one can be stricter than.
	if (subschemaAjv.getSchema(`config#/${path.join('/')}`)?.(value)) return [];
	const narrowed = JSON.stringify(
		narrowSchema(
			configSchemaNode(path),
			value,
			fields.map(field => field.split('.'))
		)
	).replaceAll('"$ref":"#/', '"$ref":"config#/');
	let validate = narrowedValidators.get(narrowed);
	if (!validate) {
		validate = subschemaAjv.compile(JSON.parse(narrowed));
		narrowedValidators.set(narrowed, validate);
	}
	if (validate(value)) return [];
	const errors = validate.errors ?? [];
	const found = new Map<string, string>();
	for (const error of errors) {
		const base = error.instancePath
			.split('/')
			.slice(1)
			.map(part => part.replaceAll('~1', '/').replaceAll('~0', '~'));
		let at = base;
		let message = error.message ?? 'Invalid value';
		if (error.keyword === 'required') {
			at = [...base, String(error.params.missingProperty)];
			message = 'Required';
		} else if (error.keyword === 'additionalProperties') {
			at = [...base, String(error.params.additionalProperty)];
			message = 'Unknown field';
		} else if (['anyOf', 'oneOf', 'if'].includes(error.keyword)) {
			// A summary of errors reported on their own.
			if (
				errors.some(other => other !== error && other.instancePath.startsWith(error.instancePath))
			)
				continue;
		}
		const key = at.join('.');
		if (!found.has(key)) found.set(key, message.charAt(0).toUpperCase() + message.slice(1));
	}
	return [...found].map(([path, message]) => ({ path, message }));
}

export function configValueError(path: string[], value: unknown): string | null {
	const error = configValueErrors(path, value)[0];
	return error ? `${error.path || '/'}: ${error.message}` : null;
}

// Whether the field at `fieldPath` (dotted) within the schema node at
// `schemaPath` is required in any shape the schema allows.
export function schemaRequires(schemaPath: string[], fieldPath: string) {
	const parts = fieldPath.split('.');
	let nodes = [configSchemaNode(schemaPath)];
	for (const part of parts.slice(0, -1)) {
		nodes = nodes.flatMap(schemaVariants).flatMap(node => {
			const child = /^\d+$/.test(part)
				? node.items
				: (node.properties as Record<string, unknown> | undefined)?.[part];
			return child ? [child] : [];
		});
	}
	const last = parts.at(-1);
	return nodes
		.flatMap(schemaVariants)
		.some(node => Array.isArray(node.required) && node.required.includes(last));
}

// The schema with each anyOf/oneOf replaced by the branch `value` (or else
// the `fields` shown) is closest to, following the value down through
// properties and list items.
function narrowSchema(schema: unknown, value: unknown, fields: string[][]): unknown {
	if (!schema || typeof schema !== 'object' || Array.isArray(schema)) return schema;
	let node = resolveRef(schema);
	for (const keyword of ['anyOf', 'oneOf'] as const) {
		const branches = node[keyword];
		if (!Array.isArray(branches)) continue;
		const fits = branches.filter(branch => accepts(branch, value));
		if (!fits.length) continue;
		const best = fits.reduce((a, b) =>
			fitScore(b, value, fields) > fitScore(a, value, fields) ? b : a
		);
		const { [keyword]: _, ...rest } = node;
		const branch = narrowSchema(best, value, fields) as SchemaNode;
		node = {
			...rest,
			...branch,
			properties: { ...(rest.properties as object), ...(branch.properties as object) },
			required: [
				...(Array.isArray(rest.required) ? rest.required : []),
				...(Array.isArray(branch.required) ? branch.required : [])
			]
		};
	}
	if (isPlainRecord(value) && isPlainRecord(node.properties)) {
		const properties = node.properties;
		node = {
			...node,
			properties: Object.fromEntries(
				Object.entries(properties).map(([key, property]) => [
					key,
					key in value ? narrowSchema(property, value[key], fieldsUnder(fields, key)) : property
				])
			)
		};
	}
	if (Array.isArray(value) && value.length && node.items) {
		const items = node.items;
		node = {
			...node,
			prefixItems: value.map((item, index) =>
				narrowSchema(items, item, fieldsUnder(fields, String(index)))
			)
		};
	}
	return node;
}

// Whether `value` has the type (or constant) a schema branch expects.
function accepts(schema: unknown, value: unknown): boolean {
	const node = resolveRef(schema);
	if ('const' in node) return deepEqual(node.const, value);
	if (Array.isArray(node.enum)) return node.enum.some(item => deepEqual(item, value));
	const types = node.type === undefined ? [] : [node.type].flat();
	if (!types.length) {
		if (node.properties || node.required) return isPlainRecord(value);
		const branches = [node.anyOf, node.oneOf].find(Array.isArray);
		return branches ? branches.some(branch => accepts(branch, value)) : true;
	}
	return types.some(type =>
		type === 'null'
			? value === null
			: type === 'object'
				? isPlainRecord(value)
				: type === 'array'
					? Array.isArray(value)
					: type === 'integer'
						? Number.isInteger(value)
						: typeof value === type
	);
}

function resolveRef(schema: unknown): SchemaNode {
	if (!isPlainRecord(schema)) return {};
	if (typeof schema.$ref !== 'string' || !schema.$ref.startsWith('#/')) return schema;
	const { $ref, ...rest } = schema;
	return { ...resolveRef(configSchemaNode($ref.slice(2).split('/'))), ...rest };
}

// How well a branch fits the value, in order: how many of the value's fields
// it knows (less those it forbids), how many of the form's fields it has, and
// how few of its required fields are missing. Remaining ties go to the branch
// with more fields, the general shape over a special case.
function fitScore(schema: unknown, value: unknown, fields: string[][]) {
	if (!isPlainRecord(value)) return 0;
	const nodes = schemaVariants(schema);
	const known = new Set(nodes.flatMap(node => Object.keys((node.properties as object) ?? {})));
	const closed = nodes.some(node => node.additionalProperties === false);
	const fit = Object.keys(value).reduce(
		(score, key) => score + (known.has(key) ? 1 : closed ? -1 : 0),
		0
	);
	const shown = fields.filter(field => field.length && known.has(field[0])).length;
	const required = resolveRef(schema).required;
	const missing = Array.isArray(required) ? required.filter(key => !(key in value)).length : 0;
	return fit * 10000 + shown * 100 - missing + known.size / 10000;
}

function fieldsUnder(fields: string[][], key: string) {
	return fields.filter(field => field[0] === key).map(field => field.slice(1));
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
	return Boolean(value && typeof value === 'object' && !Array.isArray(value));
}

export function configSchemaNode(path: string[]) {
	return path.reduce<unknown>(
		(node, key) =>
			node && typeof node === 'object' ? (node as Record<string, unknown>)[key] : undefined,
		configSchema
	);
}

// Removes fields that equal their schema default, so a form does not write out
// values nobody chose. `path` is a config schema node path.
export function withoutSchemaDefaults<T>(value: T, path: string[]): T {
	return stripDefaults(value, configSchemaNode(path)) as T;
}

type SchemaNode = Record<string, unknown>;

function stripDefaults(value: unknown, schema: unknown): unknown {
	const variants = schemaVariants(schema);
	if (Array.isArray(value)) {
		const items = variants.find(node => node.items)?.items;
		return items ? value.map(item => stripDefaults(item, items)) : value;
	}
	if (!value || typeof value !== 'object') return value;
	const properties = Object.assign(
		{},
		...[...variants].reverse().map(node => (node.properties ?? {}) as Record<string, SchemaNode>)
	) as Record<string, SchemaNode>;
	const next: Record<string, unknown> = {};
	for (const [key, item] of Object.entries(value)) {
		const property = properties[key];
		if (!property) {
			next[key] = item;
			continue;
		}
		if ('default' in property && deepEqual(item, property.default)) continue;
		const stripped = stripDefaults(item, property);
		// Every field was a default, and so is leaving the object out.
		if (
			'default' in property &&
			stripped &&
			typeof stripped === 'object' &&
			!Array.isArray(stripped) &&
			!Object.keys(stripped).length
		)
			continue;
		next[key] = stripped;
	}
	return next;
}

// The node with its $ref resolved, plus every oneOf/anyOf/allOf branch.
function schemaVariants(schema: unknown): SchemaNode[] {
	if (!schema || typeof schema !== 'object') return [];
	const node = schema as SchemaNode;
	const ref = typeof node.$ref === 'string' ? node.$ref.replace('#/', '').split('/') : null;
	return [
		node,
		...(ref ? schemaVariants(configSchemaNode(ref)) : []),
		...[node.oneOf, node.anyOf, node.allOf].flatMap(branches =>
			Array.isArray(branches) ? branches.flatMap(schemaVariants) : []
		)
	];
}

function sanitizeEmptyEnums(value: unknown): unknown {
	if (!value || typeof value !== 'object') return value;
	if (Array.isArray(value)) {
		value.forEach(sanitizeEmptyEnums);
		return value;
	}

	const object = value as Record<string, unknown>;
	if (Array.isArray(object.enum) && object.enum.length === 0) {
		delete object.enum;
		delete object.type;
		object.not = {};
	}
	Object.values(object).forEach(sanitizeEmptyEnums);
	return object;
}
