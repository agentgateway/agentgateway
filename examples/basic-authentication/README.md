# Basic Authentication Example

This example demonstrates how to use Basic Authentication with agentgateway using htpasswd files.

## Overview

Basic Authentication provides a simple username/password authentication mechanism using htpasswd files. This is useful for protecting routes with standard HTTP Basic Auth.

## Setup

1. Create an htpasswd file with user credentials:

```bash
# Using htpasswd tool (from apache2-utils)
htpasswd -c .htpasswd testuser
# Enter password: test123

# Or manually create with bcrypt (recommended)
# Password: test123
echo 'testuser:$2y$05$H5iJbsJPn0dZVD6kM6tOQuVJxLw7KjKCGvWlhG1SxNxLNn6hZoKYy' > .htpasswd
```

2. Start agentgateway with the example configuration:

```bash
agentgateway --config examples/basic-authentication/config.yaml
```

3. Test the protected endpoint:

```bash
# Without credentials - should return 401
curl http://localhost:3000/

# With valid credentials - should succeed
curl -u testuser:test123 http://localhost:3000/

# With invalid credentials - should return 401
curl -u testuser:wrongpass http://localhost:3000/
```

## Configuration

The `config.yaml` shows how to configure basic authentication:

```yaml
binds:
- port: 3000
  listeners:
  - routes:
    - policies:
        basicAuth:
          htpasswdFile: ./examples/basic-authentication/.htpasswd
          realm: "Protected Area"
          mode: optional  # strict, optional (default), or permissive
      backends:
      - http:
          hostname: httpbin.org
          port: 80
```

### Configuration Options

- `htpasswdFile`: Path to the htpasswd file containing user credentials (required)
- `realm`: Realm name shown in the browser authentication dialog (optional, default: "Restricted")
- `mode`: Validation mode for authentication (optional, default: "optional")
  - `strict`: Requires valid credentials - rejects requests without credentials or with invalid credentials
  - `optional`: Validates credentials if provided - allows requests without credentials but validates any provided credentials
  - `permissive`: Never rejects requests - useful for logging/authorization in later steps, accepts invalid credentials

## Htpasswd File Format

The htpasswd file should contain one user per line in the format:

```
username:hashed_password
```

Supported password hashing algorithms:
- BCrypt (recommended): `$2y$` prefix
- Apache MD5: `$apr1$` prefix
- SHA1: `{SHA}` prefix
- Crypt: traditional Unix crypt

## Authentication Modes

Basic authentication supports three validation modes similar to JWT authentication:

### Strict Mode
Requires valid credentials for all requests. This is the most secure mode.

```yaml
basicAuth:
  htpasswdFile: ./.htpasswd
  mode: strict
```

**Behavior:**
- No credentials → 401 Unauthorized
- Invalid credentials → 401 Unauthorized  
- Valid credentials → Request proceeds

**Use cases:**
- Protecting sensitive endpoints
- APIs requiring authentication for all requests

### Optional Mode (Default)
Validates credentials if provided, but allows requests without credentials.

```yaml
basicAuth:
  htpasswdFile: ./.htpasswd
  mode: optional  # or omit mode field
```

**Behavior:**
- No credentials → Request proceeds
- Invalid credentials → 401 Unauthorized
- Valid credentials → Request proceeds

**Use cases:**
- Mixed public/private content
- APIs with optional authentication
- Gradual authentication rollout

### Permissive Mode
Never rejects requests. Useful for logging or authorization in later policy steps.

```yaml
basicAuth:
  htpasswdFile: ./.htpasswd
  mode: permissive
```

**Behavior:**
- No credentials → Request proceeds
- Invalid credentials → Request proceeds (with warning log)
- Valid credentials → Request proceeds

**Use cases:**
- Capturing user information for logging
- Using credentials in authorization rules
- A/B testing authentication flows

## Security Notes

1. Always use HTTPS in production to prevent credentials from being transmitted in cleartext
2. Use strong password hashing algorithms like BCrypt
3. Store htpasswd files securely with appropriate file permissions
4. Consider using more advanced authentication methods (JWT, OAuth) for production APIs

## Integration with Other Policies

Basic Authentication can be combined with other policies:

```yaml
policies:
  basicAuth:
    htpasswdFile: ./.htpasswd
  authorization:
    allow:
    - 'request.path.startsWith("/public")'
    - 'basicauth.user == "admin" && request.path.startsWith("/admin")'
  cors:
    allowOrigins: ["*"]
```

## Troubleshooting

- **401 Unauthorized**: Check that credentials are correct and user exists in htpasswd file
- **File not found**: Ensure htpasswd file path is correct relative to where agentgateway is running
- **Invalid password**: Verify password hash format is supported (BCrypt, MD5, etc.)
