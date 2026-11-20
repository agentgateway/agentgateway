package after_resolution

import rego.v1

# input.registry holds only the definitions of this registry, not its dependencies.

owned_prefixes := ["agentgateway.", "agw."]

# Names agentgateway emitted before this registry existed. Remove a name when it is renamed.
legacy := {
	"gateway",
	"listener",
	"route",
	"route_rule",
	"endpoint",
	"protocol",
	"error",
	"reason",
	"duration",
	"connection.id",
	"request.id",
	"src.identity",
	"tls.sni",
	"trace.id",
	"span.id",
	"jwt.sub",
	"retry.attempt",
	"http.status",
	"grpc.status",
	"inferencepool.selected_endpoint",
	"a2a.method",
	"a2a.response.outcome",
	"a2a.response.error_code",
	"a2a.result.kind",
	"a2a.context.id",
	"mcp.target",
	"mcp.resource.type",
	"mcp.error.code",
	"mcp.error.message",
	"gen_ai.usage.reasoning_tokens",
	"gen_ai.usage.input_audio_tokens",
	"gen_ai.usage.output_audio_tokens",
	"gen_ai.usage.output_image_tokens",
	"gen_ai.usage.audio_seconds",
	"gen_ai.usage.cache_creation.input_tokens",
}

# Owned by Substrate; see crates/agentgateway/src/http/substrate/ateattr.rs.
external_prefixes := ["ate."]

owned(key) if {
	some prefix in owned_prefixes
	startswith(key, prefix)
}

external(key) if {
	some prefix in external_prefixes
	startswith(key, prefix)
}

deny contains finding if {
	some attr in input.registry.attributes
	not owned(attr.key)
	not external(attr.key)
	not attr.key in legacy
	finding := {
		"id": "agentgateway_owned_namespace",
		"context": {"attribute": attr.key},
		"message": sprintf("Attribute '%s' is defined by agentgateway but is not in the agentgateway. or agw. namespace. Reference the upstream attribute instead.", [attr.key]),
		"level": "violation",
	}
}

deny contains finding if {
	some span in input.registry.spans
	not "error.type" in {attr.key | some attr in span.attributes}
	finding := {
		"id": "agentgateway_error_type_on_spans",
		"context": {"span": span.type},
		"message": sprintf("Span '%s' does not reference error.type. Every agentgateway span can fail.", [span.type]),
		"level": "violation",
	}
}
