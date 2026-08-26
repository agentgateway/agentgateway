# Microsoft Intune client verification

These example scripts let Microsoft Intune verify that managed Codex and
Claude Desktop clients use an approved agentgateway address. Each script can
check either client or both clients without returning configuration contents,
tokens, or provider credentials to Intune.

The scripts check:

- whether the selected client is installed;
- whether its effective managed configuration contains the expected
  agentgateway address; and
- whether the managed device can reach that address and receive an HTTP
  response.

On macOS, the verifier resolves the signed-in console user and checks both
user-specific and device-level managed preferences for Codex and Claude
Desktop. It then falls back to the effective user preference domain. This
supports an Intune preference profile assigned through either management
scope, including a per-user Claude Desktop profile under `/Library/Managed
Preferences/USERNAME/com.anthropic.claudefordesktop.plist`.

Any HTTP response proves DNS, transport, and listener reachability. A `401` or
`403` response is therefore a successful connectivity check when agentgateway
requires authentication. The scripts do not send an LLM request and do not
prove that an interactive client request used agentgateway. Complete that
final check from the client and correlate it with the agentgateway access log.

## Configure the scripts

Before uploading a script to Intune, set the configuration values at the top
to match the managed client configuration exactly. The macOS assignments use
the `${VARIABLE:-default}` form so that an administrator can override values
during local testing. Intune does not provide custom environment variables to
these scripts, so edit the defaults in the uploaded copy.

| Setting | Required value |
| --- | --- |
| `EXPECTED_CODEX_BASE_URL` or `ExpectedCodexBaseUrl` | Exact Codex `base_url`, including `/v1`, such as `https://llm.example.com/v1`. |
| `EXPECTED_CODEX_ENV_KEY` or `ExpectedCodexEnvKey` | Name from the managed TOML `env_key`, such as `AGENTGATEWAY_API_KEY`. The scripts check the name, not the secret value. |
| `EXPECTED_CLAUDE_GATEWAY_URL` or `ExpectedClaudeGatewayUrl` | Exact Claude Desktop `inferenceGatewayBaseUrl`. Include a prefix such as `/claude` only when it is part of the managed URL. |
| `EXPECTED_CLAUDE_CREDENTIAL_KIND` or `ExpectedClaudeCredentialKind` | `static` for Gateway API key authentication or `interactive` for Entra ID. Other values fail verification. |
| `EXPECTED_CLAUDE_OIDC_AUTH_FLOW` or `ExpectedClaudeOidcAuthFlow` | For `interactive` only, `browser` or `broker`. Leave empty for `static`. |
| `EXPECTED_CLAUDE_OIDC_ISSUER` or `ExpectedClaudeOidcIssuer` | For `interactive` only, the exact issuer, such as `https://login.microsoftonline.com/TENANT_ID/v2.0`. Leave empty for `static`. |
| `EXPECTED_CLAUDE_OIDC_CLIENT_ID` or `ExpectedClaudeOidcClientId` | For `interactive` only, the Entra Application (client) ID. Leave empty for `static`. |

For a static-key Claude Desktop profile, edit only the Gateway URL when the
default credential kind is already `static`, and leave the OIDC values empty.
For Entra ID, set the credential kind to `interactive` and populate all three
OIDC values. The scripts require an ID token and reject a leftover static key.

In the operational verification scripts, enable only the clients that Intune
requires on the target group. Keep the installation check enabled when the
approved package uses one of the paths in the script. Otherwise, add the
organization's package path or disable this check and use the Intune
managed-app report. Keep the network check enabled unless another endpoint
control performs it.

On macOS, keep the default verification log or set the
`AGENTGATEWAY_INTUNE_LOG_FILE` environment variable to another absolute `.log`
path during local testing. Because Intune does not provide a custom
environment-variable field for platform scripts, edit the `LOG_FILE` default
before upload when the organization requires a different managed path.

Do not add a gateway client key, LLM provider key, bearer token, or another
secret to either script.

## Deploy on macOS

Use
[`verify-agentgateway-clients-macos.sh`](verification/verify-agentgateway-clients-macos.sh)
as an Intune macOS shell script:

1. Go to **Devices > By platform > macOS > Manage devices > Scripts > Add**.
2. Upload the script.
3. Set **Run script as signed-in user** to **Yes**. Codex managed preferences
   and the effective Claude Desktop preferences are evaluated in the user's
   context.
4. Select a frequency appropriate for the pilot and assign the script to the
   pilot group.
5. Review **Device status** or **User status**. Exit code `0` reports success;
   a nonzero exit code reports failure. The script prints only individual
   check results.

The Mac must be managed by Intune, run macOS 12 or later, and have the
Microsoft Intune management agent. See [Use shell scripts on macOS devices in
Intune](https://learn.microsoft.com/en-us/intune/device-management/tools/run-shell-scripts-macos).

## Deploy on Windows

Use
[`Verify-AgentgatewayClientsWindows.ps1`](verification/Verify-AgentgatewayClientsWindows.ps1)
as the detection script in an Intune Remediations package:

1. Go to **Devices > Manage devices > Scripts and remediations** and create a
   script package.
2. Upload the script as the detection script. A remediation script is optional
   when another policy already restores the managed configuration.
3. Set **Run this script using the logged-on credentials** to **Yes** and
   **Run script in 64-bit PowerShell** to **Yes**.
4. Assign a schedule and the pilot group, then monitor **Device status**.

The Windows verifier returns exit code `1` when any enabled check fails, which
causes the package to report an issue. Remediations have additional enrollment,
edition, and licensing requirements. See [Use Remediations to detect and fix
support issues](https://learn.microsoft.com/en-us/intune/device-management/tools/deploy-remediations).

For a one-time Windows check, upload the same file as a [Windows platform
script](https://learn.microsoft.com/en-us/intune/device-management/tools/run-powershell-scripts-windows).

## Add custom compliance reporting

The operational verification scripts cannot be used unchanged as Intune
custom-compliance discovery scripts. They print diagnostic lines and return a
nonzero exit code when a check fails. Custom compliance requires output that
matches its rule definition, and a discovered noncompliant value is not a
script execution failure.

Use the compliance artifacts for the client required by the target device
group. Each client has independently assignable discovery scripts and a
matching rule JSON.

| Client | macOS discovery | Windows discovery | Rule JSON | Setting |
| --- | --- | --- | --- | --- |
| Codex | [`discover-gateway-macos.sh`](compliance/codex/discover-gateway-macos.sh) | [`Discover-GatewayWindows.ps1`](compliance/codex/Discover-GatewayWindows.ps1) | [`compliance.json`](compliance/codex/compliance.json) | `CodexGatewayConfigured` |
| Claude Desktop | [`discover-gateway-macos.sh`](compliance/claude-desktop/discover-gateway-macos.sh) | [`Discover-GatewayWindows.ps1`](compliance/claude-desktop/Discover-GatewayWindows.ps1) | [`compliance.json`](compliance/claude-desktop/compliance.json) | `ClaudeDesktopGatewayConfigured` |

The discovery scripts check only the durable managed client configuration.
They do not test network reachability, because a temporary Gateway or network
outage must not make every managed device noncompliant. The Claude Desktop
scripts require the `gateway` inference provider, exact approved Gateway URL,
and selected credential model. For Entra ID, they also require the approved
sign-in flow, issuer, client ID, and ID-token setting, and report noncompliance
if the old static key remains. The Codex scripts also require the approved
`env_key` declaration but do not read or report the environment variable's
value.

Before uploading a discovery script, replace its example URL with the approved
address. Include `/v1` for Codex and the configured route prefix, such as
`/claude`, for Claude Desktop. If the organization uses a different Codex
credential variable, also change `EXPECTED_CODEX_ENV_KEY`. Keep these values
aligned with the managed configuration policy. For Entra ID, set
`EXPECTED_CLAUDE_CREDENTIAL_KIND` to `interactive` and populate the expected
OIDC flow, issuer, and client ID. Leave the OIDC values empty for the default
static-key example.

### Configure custom compliance on macOS

1. Select the client-specific `discover-gateway-macos.sh` and
   `compliance.json` files. Create separate policies and assignments when
   different device groups require different clients.
2. Go to **Endpoint security > Device compliance > Scripts > Add > macOS** and
   upload the discovery script.
3. The script resolves the signed-in console user and reads both per-user and
   machine managed preferences, so it supports either the default system
   context or logged-in-user context. If Intune displays an execution-context
   setting, either context is supported. Enable signature enforcement when the
   organization signs scripts.
4. Create a macOS compliance policy, add **Custom Compliance**, select the
   discovery script, and upload the matching `compliance.json`.
5. Assign the policy to the same pilot group as the application and managed
   configuration policies.

Each macOS script returns one JSON object on a single line. For example:

```json
{"CodexGatewayConfigured":true}
```

```json
{"ClaudeDesktopGatewayConfigured":true}
```

The setting name is case-sensitive and must match the corresponding
`SettingName` in `compliance.json`. The value is a JSON Boolean, not a quoted
string. Each script returns exit code `0` for either discovered value. A
nonzero exit code is reserved for a script execution error.

### Configure custom compliance on Windows

1. Select the client-specific `Discover-GatewayWindows.ps1` and
   `compliance.json` files. Create separate policies and assignments when
   different device groups require different clients.
2. Go to **Endpoint security > Device compliance > Scripts > Add > Windows**
   and upload the discovery script.
3. Set **Run this script using the logged on credentials** and **Run script in
   64-bit PowerShell Host** to **Yes**. Enable signature enforcement when the
   organization signs scripts.
4. Create a Windows compliance policy, add **Custom Compliance**, select the
   discovery script, and upload the matching `compliance.json`.
5. Assign the policy to the same pilot group as the application and managed
   configuration policies.

Each Windows script also returns one compressed JSON object. For example:

```json
{"CodexGatewayConfigured":true}
```

```json
{"ClaudeDesktopGatewayConfigured":true}
```

For requirements and limits, see [Custom compliance discovery scripts for
Microsoft
Intune](https://learn.microsoft.com/en-us/intune/device-security/compliance/create-custom-script)
and [Custom compliance JSON files in Microsoft
Intune](https://learn.microsoft.com/en-us/intune/device-security/compliance/create-custom-json).

Custom compliance reports state but does not repair configuration. Keep the
managed preference or remediation policy assigned. A corrected setting can
take up to eight hours to appear compliant.

## Verify delivery and execution

An assignment shows that Intune intends to deliver a script. A per-device or
per-user run status shows that the managed client received and attempted to run
it. Use the reports for the deployment method that you selected.

### macOS status and logs

1. In the Intune admin center, go to **Devices > Manage devices > Scripts and
   remediations > Platform scripts** and select the macOS verification script.
2. Open **Device status** or **User status** and locate the managed Mac.
3. Interpret the latest status.

   - **Succeeded** means that the script ran and returned exit code `0`. All
     enabled verification checks passed.
   - **Failed** means that the script returned a nonzero exit code or Intune
     could not execute a valid script.
   - **Pending** or no status means that execution has not been reported. It
     does not prove delivery.

The user must be signed in because this example runs in the signed-in user's
context. The Intune management agent normally checks for scripts approximately
every eight hours. Company Portal **Check status** can request a device check,
but script retrieval uses an agent check-in that is separate from the normal
MDM check-in.

To troubleshoot a missing or failed status, select the device in the script
report and use **Collect logs**. The verifier writes the same sanitized
`PASS`, `FAIL`, and summary messages that it prints during execution to this
default per-user path:

```text
/Users/USERNAME/Library/Logs/agentgateway/intune-verification.log
```

Replace `USERNAME` with the signed-in user's short name when you enter the path
in Intune. The **Collect logs** field requires a fully expanded absolute path;
it does not expand `$HOME` or `~`. The file is truncated at the start of each
run, uses owner-only permissions, and does not contain decoded configuration,
tokens, prompts, or credentials.

Intune also includes its macOS agent logs from:

```text
/Library/Logs/Microsoft/Intune
~/Library/Logs/Microsoft/Intune
```

Look for files named `IntuneMDMAgent...log` and
`IntuneMDMDaemon...log`. For more information, see [Troubleshoot macOS shell
script policies using log
collection](https://learn.microsoft.com/en-us/intune/device-management/tools/run-shell-scripts-macos#troubleshoot-macos-shell-script-policies-using-log-collection).

### Windows status, output, and logs

1. In the Intune admin center, go to **Devices > Manage devices > Scripts and
   remediations**, select the verification package, and open **Device status**.
2. Locate the managed Windows device and review its latest detection status and
   output.

   - A successful or **Without issues** result means that detection returned
     exit code `0`. All enabled verification checks passed.
   - A failed or **With issues** result means that detection returned exit code
     `1`. One or more checks failed.
   - **Pending** or no status means that execution has not been reported.

The detection output contains concise `PASS` and `FAIL` messages without
configuration contents or credentials. Use **Export** to download the reported
results as CSV. To inspect a single device, go to **Devices > By platform >
Windows**, select the device, and open **Remediations**. During a pilot, an
administrator with the required permission can also use **Run remediation** to
request an on-demand execution.

Windows retrieves new Remediation policy after a device or Intune Management
Extension restart, after user sign-in, and during the extension's approximately
eight-hour check-in. If the result is missing or failed, inspect:

```text
C:\ProgramData\Microsoft\IntuneManagementExtension\Logs\HealthScripts.log
C:\ProgramData\Microsoft\IntuneManagementExtension\Logs\AgentExecutor.log
```

`HealthScripts.log` records recurring Remediations, and `AgentExecutor.log`
records PowerShell execution. See [Understand the Intune Management Extension
logs](https://learn.microsoft.com/en-us/intune/device-management/tools/management-extension-windows#intune-management-extension-logs).

### Pilot acceptance criteria

For each pilot device, confirm all of the following before broad assignment:

1. The device or user appears in the applicable Intune script report.
2. The latest execution reports success and the expected enabled checks pass.
3. After restarting the client, an interactive inference request appears in
   the agentgateway access log as described in the next section.

## Final interactive verification

After the script succeeds, restart the client and send a harmless, unique
prompt. Correlate the request by time in the agentgateway access log:

- Codex must send a successful `POST /v1/responses` request.
- Claude Desktop must send a successful `POST /v1/messages` request.

Confirm the expected hostname, route, authenticated identity when configured,
upstream provider, and successful status. Agentgateway logs must not contain
the bearer token or upstream provider credential.

For Codex, first verify the gateway key independently from the same user
context. The variable must contain the gateway client key, not the OpenAI
provider key.

```sh
curl --fail-with-body \
  --header "Authorization: Bearer $AGENTGATEWAY_API_KEY" \
  "https://llm.example.com/v1/models?client_version=intune-verification"
```

If this request succeeds but Codex returns HTTP 401, confirm that Codex loads
the managed TOML, that `AGENTGATEWAY_API_KEY` is available to the Codex
process, and that the application was fully restarted. The operational and
compliance scripts intentionally do not inspect the secret value, so only an
authenticated request verifies credential delivery.
