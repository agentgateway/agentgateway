#!/usr/bin/env python3
import json
import base64
import hashlib
import hmac
import time
import secrets
import urllib.parse
import subprocess
import os
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

# RSA key components (generated with openssl)
PRIVATE_KEY_PEM = """-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC8V4ETh0yKdPbJ
dvxKfqTByeTL3b+VxUePnuYzwWFuvMFZL7oLsCyBZ/IUO9tsTtBA5XZt3Q5mVPg2
Ljke0BrPmcoL7CFtjelfjpiRp5/HkPXabHkXa7Z7F+5ybDmjrtjG+dm7Ygy4Di2+
rH1l9W0LcspzvJL7IZU6EaWJ9flIV3Kzom30ai7SxMDOUJiymzbPDCkIT142eXfN
bHgjCmNf4H5O6wi3+mWzCiSHVYWgnmJDVvmxtf9WNPy3ReNZln5eFPPib2b5ylne
BxDppTQ/p+qtQ4MCb4/9kExchDINnUxGxB05V2lC9u5tu89qCxlfnlHcdQtuZB1E
eKMqRB5tAgMBAAECggEAFiBUlGn+eryjeocdPBY1PmP82lt2hL6cudlx0246R1Nr
BpKGEZX0oI5I4QooLMj0i885Us5XwPtl/p1/DejmYXHAjBaFVdTIcDa1I1V1PrF/
xJWeQztfAIaO94fR3LIvmu6i3vH1qxDVXHNwtu/2i9QER0UF4nVvTddhYnwQeWhC
5QmL3IQFwhQ5xVI0i+KI+NCsKV10drDUoZGcu/zFia5SmdsjCQ7+Xen85yumNSmP
ATD7WMfEL0XM21/rbhF3cFOGGDPR9dWbO5Bgfa4ol0uzUE8Q8F+y7QK1tkM8gb4/
astNNUlQas2TE6XeMD47+LjfhrQOYxfeUtmukctp0QKBgQDrOSWLo+29eyUdm4ZF
lyFnrBlKB+5cxUCec1cLi68eDsVI3aS/UGmJC6qhj2Irwo+VJT4iKp9uZMIM9KCI
8veuIc82VCpKDZ2NzwW1+nc9swth85qAFqRXcRoW2Zg87T8ZmkRxjOKQILETgIzY
lmkmtTg/YUtCR6wq/NiR8/VkPQKBgQDM+kcfRrTGl4W1Hlvn48VkIFvVcvze5AQl
OTD8ldfslR2PZPbSp24I+IZHYzreFX5KjZwR7k8Zw/0JPZp+xLil/4rlDrnQpnbT
c3mNVuGcsVOOFiQHNDlvRW1Mh5Y0hKqZ4WJE64GI2y9GLN4KBD0Ey+++bC5t7uiC
wqZpjuLV8QKBgQCht5NRku2DROO6nE9PDt1/ijmExTkifNa1WTTyEiHeN2d5djCq
+1zjRKsWEh77WPMgJg+2q7kay5kCETlBjlGsXUA56Nl+Oigk87zIZR+PwsXDnRiO
kYKBP5ghN45L7QxhzMbbjnHBh0hW0R2EVryKSTMXmAuG0QHUOCupBKGkPQKBgFbs
i9ynj2HoP7te9HqSDNM5JbiO2s1qxJdEeZGjub2KPs7gcgtDFVaYjdkYK46ibrwO
8XBpLwIuKtAQX8QCiItcovogFIx3C00AWzuk7GgWiuhmW0Dy1KhrOL6LgRcka3R2
L8YqWPRAfvuzazW0NmwiT7jhB493EQLiqM962JcBAoGAN3UzRXyHCP4ViiQmpNFA
JfQI3oIkOH7Vx67/xtRCUhBDYCKfnPs2lnXv5Pj6AugBdgF5nv1Ss0VD82iiX9+g
4GOk+gaCXB9gSlNuBe5Z3WRjsRZAOHlKcoJyCP+hif90cs55h1AuqUuLczPCBtcI
eInnfGNXAr7Q3p5QaAWOmJk=
-----END PRIVATE KEY-----"""

# RSA public key components for JWK
RSA_N = "vFeBE4dMinT2yXb8Sn6kwcnky92_lcVHj57mM8FhbrzBWS-6C7AsgWfyFDvbbE7QQOV2bd0OZlT4Ni45HtAaz5nKC-whbY3pX46Ykaefx5D12mx5F2u2exfucmw5o67YxvnZu2IMuA4tvqx9ZfVtC3LKc7yS-yGVOhGlifX5SFdys6Jt9Gou0sTAzlCYsps2zwwpCE9eNnl3zWx4IwpjX-B-TusIt_plswokh1WFoJ5iQ1b5sbX_VjT8t0XjWZZ-XhTz4m9m-cpZ3gcQ6aU0P6fqrUODAm-P_ZBMXIQyDZ1MRsQdOVdpQvbubbvPagsZX55R3HULbmQdRHijKkQebQ"
RSA_E = "AQAB"

# JWK for RSA RS256
PUBLIC_KEY_JWK = {
    "kty": "RSA",
    "kid": "key-1",
    "use": "sig",
    "alg": "RS256",
    "n": RSA_N,
    "e": RSA_E
}

# In-memory storage
registered_clients = {}
authorization_codes = {}
tokens = {}

def generate_id(prefix="", length=32):
    """Generate a random ID with optional prefix"""
    chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-"
    return prefix + ''.join(secrets.choice(chars) for _ in range(length))

def base64url_encode(data):
    """Base64url encode data"""
    if isinstance(data, str):
        data = data.encode('utf-8')
    return base64.urlsafe_b64encode(data).rstrip(b'=').decode('ascii')

def create_jwt_with_openssl(payload):
    """Create a JWT using openssl command for RS256 signing"""
    # Header
    header = {
        "typ": "JWT",
        "alg": "RS256",
        "kid": "key-1"
    }

    # Encode header and payload
    encoded_header = base64url_encode(json.dumps(header, separators=(',', ':')))
    encoded_payload = base64url_encode(json.dumps(payload, separators=(',', ':')))

    # Create the signing input
    signing_input = f"{encoded_header}.{encoded_payload}"

    # Write private key to temp file
    with open('/tmp/jwt_private_key.pem', 'w') as f:
        f.write(PRIVATE_KEY_PEM)

    try:
        # Use openssl to sign
        process = subprocess.run([
            'openssl', 'dgst', '-sha256', '-sign', '/tmp/jwt_private_key.pem'
        ], input=signing_input.encode(), capture_output=True)

        if process.returncode != 0:
            raise Exception(f"OpenSSL signing failed: {process.stderr.decode()}")

        signature = base64url_encode(process.stdout)
        return f"{signing_input}.{signature}"

    finally:
        # Clean up temp file
        if os.path.exists('/tmp/jwt_private_key.pem'):
            os.remove('/tmp/jwt_private_key.pem')

class AuthServerHandler(BaseHTTPRequestHandler):
    def do_OPTIONS(self):
        """Handle CORS preflight requests"""
        self.send_response(200)
        self.send_header('Access-Control-Allow-Origin', '*')
        self.send_header('Access-Control-Allow-Methods', 'GET, POST, OPTIONS')
        self.send_header('Access-Control-Allow-Headers', 'Content-Type, Authorization')
        self.end_headers()

    def send_json_response(self, data, status_code=200):
        """Send a JSON response with CORS headers"""
        self.send_response(status_code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Access-Control-Allow-Origin', '*')
        self.end_headers()
        self.wfile.write(json.dumps(data, indent=2).encode('utf-8'))

    def send_redirect(self, location):
        """Send a redirect response"""
        self.send_response(302)
        self.send_header('Location', location)
        self.send_header('Access-Control-Allow-Origin', '*')
        self.end_headers()

    def send_html_response(self, html_content, status_code=200):
        """Send an HTML response"""
        self.send_response(status_code)
        self.send_header('Content-Type', 'text/html; charset=utf-8')
        self.send_header('Access-Control-Allow-Origin', '*')
        self.end_headers()
        self.wfile.write(html_content.encode('utf-8'))

    def get_request_body(self):
        """Get and parse request body"""
        content_length = int(self.headers.get('Content-Length', 0))
        if content_length > 0:
            body = self.rfile.read(content_length).decode('utf-8')
            if self.headers.get('Content-Type', '').startswith('application/x-www-form-urlencoded'):
                return dict(urllib.parse.parse_qsl(body))
            else:
                try:
                    return json.loads(body)
                except:
                    return dict(urllib.parse.parse_qsl(body))
        return {}

    def do_POST(self):
        """Handle POST requests"""
        path = urlparse(self.path).path

        if path == '/register':
            self.handle_register()
        elif path == '/token':
            self.handle_token()
        else:
            self.send_response(404)
            self.end_headers()

    def do_GET(self):
        """Handle GET requests"""
        parsed_url = urlparse(self.path)
        path = parsed_url.path
        query_params = parse_qs(parsed_url.query)

        if path == '/authorize':
            self.handle_authorize(query_params)
        elif path == '/.well-known/jwks.json':
            self.handle_jwks()
        elif path == '/.well-known/oauth-authorization-server':
            self.handle_discovery()
        else:
            self.send_response(404)
            self.end_headers()

    def handle_register(self):
        """Handle client registration"""
        try:
            body = self.get_request_body()
            client_id = generate_id('mcp_')
            client_secret = generate_id('secret_')

            registration = {
                "client_id": client_id,
                "client_secret": client_secret,
                "client_name": body.get("client_name", "MCP Test Client"),
                "client_description": body.get("client_description", "A test MCP client"),
                "client_logo_url": body.get("client_logo_url"),
                "client_uri": body.get("client_uri"),
                "developer_name": body.get("developer_name", "Test Developer"),
                "developer_email": body.get("developer_email", "test@example.com"),
                "redirect_uris": body.get("redirect_uris", ["http://localhost:6274/oauth/callback/debug"]),
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "client_secret_basic",
                "created_at": time.strftime("%Y-%m-%dT%H:%M:%S.%fZ"),
                "updated_at": time.strftime("%Y-%m-%dT%H:%M:%S.%fZ")
            }

            registered_clients[client_id] = registration
            self.send_json_response(registration)

        except Exception as e:
            self.send_json_response({"error": "invalid_request", "error_description": str(e)}, 400)

    def handle_authorize(self, query_params):
        """Handle authorization request"""
        try:
            # Extract parameters (query_params values are lists)
            response_type = query_params.get('response_type', [''])[0]
            client_id = query_params.get('client_id', [''])[0]
            code_challenge = query_params.get('code_challenge', [''])[0]
            code_challenge_method = query_params.get('code_challenge_method', [''])[0]
            redirect_uri = query_params.get('redirect_uri', [''])[0]
            resource = query_params.get('resource', [''])[0]
            scope = query_params.get('scope', [''])[0]

            if response_type != 'code':
                self.send_json_response({"error": "unsupported_response_type"}, 400)
                return

            # Allow the hardcoded client_id from the example
            if client_id not in registered_clients and client_id == 'mcp_6950e6b7db0e6115a5af3a790340ad87':
                registered_clients[client_id] = {
                    "client_id": client_id,
                    "redirect_uris": ["http://localhost:6274/oauth/callback/debug"]
                }

            if client_id not in registered_clients:
                self.send_json_response({"error": "invalid_client"}, 400)
                return

            # Generate authorization code
            code = generate_id('', 43)  # Match example length
            code_data = {
                "client_id": client_id,
                "redirect_uri": redirect_uri,
                "resource": resource,
                "scope": scope,
                "code_challenge": code_challenge,
                "code_challenge_method": code_challenge_method,
                "expires_at": time.time() + 600  # 10 minutes
            }

            authorization_codes[code] = code_data

            # Build callback URL
            callback_url = f"{redirect_uri}?code={code}"

            # Show authorization consent page with countdown
            self.show_authorization_page(client_id, callback_url)

        except Exception as e:
            self.send_json_response({"error": "server_error", "error_description": str(e)}, 500)

    def show_authorization_page(self, client_id, callback_url):
        """Show authorization consent page with countdown"""
        html_content = f"""
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>MCP Authorization</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            margin: 0;
            padding: 0;
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
        }}
        .container {{
            background: white;
            padding: 2rem;
            border-radius: 12px;
            box-shadow: 0 20px 40px rgba(0,0,0,0.1);
            text-align: center;
            max-width: 400px;
            width: 90%;
        }}
        .logo {{
            font-size: 2.5rem;
            margin-bottom: 1rem;
        }}
        h1 {{
            color: #333;
            margin-bottom: 0.5rem;
            font-size: 1.5rem;
        }}
        .subtitle {{
            color: #666;
            margin-bottom: 2rem;
            font-size: 1rem;
        }}
        .client-info {{
            background: #f8f9fa;
            padding: 1rem;
            border-radius: 8px;
            margin: 1rem 0;
            border-left: 4px solid #667eea;
        }}
        .countdown {{
            font-size: 3rem;
            font-weight: bold;
            color: #667eea;
            margin: 1.5rem 0;
            font-family: 'Courier New', monospace;
        }}
        .status {{
            color: #28a745;
            font-weight: 500;
            margin-top: 1rem;
        }}
        .spinner {{
            display: inline-block;
            width: 20px;
            height: 20px;
            border: 2px solid #f3f3f3;
            border-top: 2px solid #667eea;
            border-radius: 50%;
            animation: spin 1s linear infinite;
            margin-right: 8px;
        }}
        @keyframes spin {{
            0% {{ transform: rotate(0deg); }}
            100% {{ transform: rotate(360deg); }}
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="logo">🔐</div>
        <h1>Authorization Successful</h1>
        <p class="subtitle">MCP Authentication Server</p>

        <div class="client-info">
            <strong>Client ID:</strong><br>
            <code>{client_id}</code>
        </div>

        <div class="status">
            <div class="spinner"></div>
            Authorization granted! Redirecting in...
        </div>

        <div class="countdown" id="countdown">3</div>

        <p style="color: #666; font-size: 0.9rem;">
            You will be redirected automatically to complete the authentication flow.
        </p>
    </div>

    <script>
        let countdown = 3;
        const countdownElement = document.getElementById('countdown');

        const timer = setInterval(() => {{
            countdown--;
            countdownElement.textContent = countdown;

            if (countdown <= 0) {{
                clearInterval(timer);
                countdownElement.textContent = '0';
                window.location.href = '{callback_url}';
            }}
        }}, 1000);

        // Also allow manual redirect by clicking
        document.addEventListener('click', () => {{
            clearInterval(timer);
            window.location.href = '{callback_url}';
        }});
    </script>
</body>
</html>
        """
        self.send_html_response(html_content)

    def handle_token(self):
        """Handle token request"""
        try:
            body = self.get_request_body()
            grant_type = body.get('grant_type')

            if grant_type == 'authorization_code':
                code = body.get('code')
                redirect_uri = body.get('redirect_uri')
                client_id = body.get('client_id')

                # Check for client credentials in Authorization header (Basic auth)
                auth_header = self.headers.get('Authorization', '')
                if not client_id and auth_header.startswith('Basic '):
                    try:
                        # Decode Basic auth
                        encoded = auth_header.split(' ', 1)[1]
                        decoded = base64.b64decode(encoded).decode('utf-8')
                        client_id, _ = decoded.split(':', 1)
                    except Exception:
                        pass

                if code not in authorization_codes:
                    self.send_json_response({"error": "invalid_grant"}, 400)
                    return

                code_data = authorization_codes[code]

                if (time.time() > code_data['expires_at'] or
                    code_data['client_id'] != client_id or
                    code_data['redirect_uri'] != redirect_uri):
                    self.send_json_response({"error": "invalid_grant"}, 400)
                    return

                # Clean up authorization code
                del authorization_codes[code]

                # Create tokens
                now = int(time.time())
                access_token_id = generate_id('access_')
                refresh_token_id = generate_id('refresh_')

                access_token_payload = {
                    "aud": code_data.get('resource', 'http://localhost:3000/mcp'),
                    "client_id": client_id,
                    "exp": now + 3600,  # 1 hour
                    "iat": now,
                    "iss": "http://localhost:9000",
                    "jti": access_token_id,
                    "resource": code_data.get('resource', 'http://localhost:3000/mcp'),
                    "scope": code_data.get('scope', ''),
                    "sub": "9026451",
                    "type": "access"
                }

                refresh_token_payload = {
                    "aud": code_data.get('resource', 'http://localhost:3000/mcp'),
                    "client_id": client_id,
                    "exp": now + (30 * 24 * 3600),  # 30 days
                    "iat": now,
                    "iss": "http://localhost:9000",
                    "jti": refresh_token_id,
                    "resource": code_data.get('resource', 'http://localhost:3000/mcp'),
                    "scope": code_data.get('scope', ''),
                    "sub": "9026451",
                    "type": "refresh"
                }

                access_token = create_jwt_with_openssl(access_token_payload)
                refresh_token = create_jwt_with_openssl(refresh_token_payload)

                # Store tokens
                tokens[access_token_id] = {"token": access_token, "payload": access_token_payload}
                tokens[refresh_token_id] = {"token": refresh_token, "payload": refresh_token_payload}

                response = {
                    "access_token": access_token,
                    "refresh_token": refresh_token,
                    "token_type": "bearer",
                    "expires_in": 3600
                }

                self.send_json_response(response)

            elif grant_type == 'refresh_token':
                # Handle refresh token (simplified for demo)
                refresh_token = body.get('refresh_token')

                # In a real implementation, you'd verify the refresh token
                # For simplicity, we'll just issue a new access token
                now = int(time.time())
                new_access_token_id = generate_id('access_')

                new_access_token_payload = {
                    "aud": "http://localhost:3000/mcp",
                    "client_id": body.get('client_id', 'mcp_6950e6b7db0e6115a5af3a790340ad87'),
                    "exp": now + 3600,
                    "iat": now,
                    "iss": "http://localhost:9000",
                    "jti": new_access_token_id,
                    "resource": "http://localhost:3000/mcp",
                    "scope": "",
                    "sub": "9026451",
                    "type": "access"
                }

                new_access_token = create_jwt_with_openssl(new_access_token_payload)
                tokens[new_access_token_id] = {"token": new_access_token, "payload": new_access_token_payload}

                response = {
                    "access_token": new_access_token,
                    "refresh_token": refresh_token,
                    "token_type": "bearer",
                    "expires_in": 3600
                }

                self.send_json_response(response)

            else:
                self.send_json_response({"error": "unsupported_grant_type"}, 400)

        except Exception as e:
            self.send_json_response({"error": "server_error", "error_description": str(e)}, 500)

    def handle_jwks(self):
        """Handle JWKS request"""
        jwks = {
            "keys": [PUBLIC_KEY_JWK]
        }
        self.send_json_response(jwks)

    def handle_discovery(self):
        """Handle OAuth discovery endpoint"""
        discovery = {
            "issuer": "http://localhost:9000",
            "authorization_endpoint": "http://localhost:9000/authorize",
            "token_endpoint": "http://localhost:9000/token",
            "jwks_uri": "http://localhost:9000/.well-known/jwks.json",
            "registration_endpoint": "http://localhost:9000/register",
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
            "code_challenge_methods_supported": ["S256"]
        }
        self.send_json_response(discovery)

    def log_message(self, format, *args):
        """Override to provide cleaner logging"""
        print(f"[{self.address_string()}] {format % args}")

def main():
    port = 9000
    server = HTTPServer(('localhost', port), AuthServerHandler)

    print(f"MCP Authorization Server running on http://localhost:{port}")
    print("Using RSA RS256 for JWT signing with OpenSSL")
    print("Endpoints:")
    print("  POST /register - Client registration")
    print("  GET  /authorize - Authorization endpoint")
    print("  POST /token - Token endpoint")
    print("  GET  /.well-known/jwks.json - JWKS endpoint")
    print("  GET  /.well-known/oauth-authorization-server - Discovery endpoint")
    print("\nPress Ctrl+C to stop the server")

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down server...")
        server.shutdown()

if __name__ == '__main__':
    main()