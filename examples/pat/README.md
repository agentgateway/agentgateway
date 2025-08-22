# PAT Authentication Examples

This directory contains examples of different ways to configure Personal Access Token (PAT) authentication for routes.

## Available Approaches

1. **pat-ai-routes.yaml** - Automatically require PAT for AI/LLM endpoints
   - Uses `PAT_REQUIRE_FOR_AI_ROUTES=true`
   - Auto-detects AI routes by pattern matching

2. **pat-route-patterns.yaml** - Pattern-based PAT requirements
   - Uses `PAT_REQUIRED_ROUTE_PATTERNS` with wildcards
   - Flexible pattern matching for route names

3. **pat-explicit-routes.yaml** - Traditional per-route configuration
   - Explicitly set `pat: true` in route policies
   - Most control but requires config changes

## Environment Variables

```bash
# Enable PAT globally (required for all approaches)
export PAT_ENABLED=true

# Auto-require PAT for AI routes
export PAT_REQUIRE_FOR_AI_ROUTES=true

# Require PAT for specific route patterns
export PAT_REQUIRED_ROUTE_PATTERNS="admin*,internal/*,v2/api/*"

# PAT token configuration
export PAT_TOKEN_PREFIX=agpk  # Default prefix for tokens
export PAT_MAX_EXPIRY_DAYS=365  # Maximum token validity
```

## Testing

```bash
# Test without PAT (should fail for protected routes)
curl http://gateway/protected/endpoint
# Returns: 401 Unauthorized

# Test with PAT token
curl -H "Authorization: Bearer agpk_YOUR_TOKEN" http://gateway/protected/endpoint
# Returns: Success

# Test public endpoint (no PAT needed)
curl http://gateway/public/endpoint
# Returns: Success
```