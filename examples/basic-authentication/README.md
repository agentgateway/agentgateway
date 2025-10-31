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
      backends:
      - http:
          hostname: httpbin.org
          port: 80
```

### Configuration Options

- `htpasswdFile`: Path to the htpasswd file containing user credentials (required)
- `realm`: Realm name shown in the browser authentication dialog (optional, default: "Restricted")

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
