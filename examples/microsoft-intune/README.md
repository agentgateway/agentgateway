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

Any HTTP response proves DNS, transport, and listener reachability. A `401` or
`403` response is therefore a successful connectivity check when agentgateway
requires authentication. The scripts do not send an LLM request and do not
prove that an interactive client request used agentgateway. Complete that
final check from the client and correlate it with the agentgateway access log.

## Configure the scripts

Before uploading a script to Intune, edit the configuration block at the top.

- Set the expected Codex URL, including `/v1`.
- Set the expected Claude Desktop URL, including the route prefix configured
  for Claude Desktop, such as `/claude`.
- Enable the clients that Intune requires on the target group. Disable a client
  when that client is not required.
- Keep the installation check enabled when the approved package uses one of
  the paths in the script. Otherwise, add the organization's package path or
  disable this check and use the Intune managed-app report.
- Keep the network check enabled unless another endpoint control performs it.

Do not add an LLM provider key, bearer token, or another secret to either
script.

## Deploy on macOS

Use
[`verify-agentgateway-clients-macos.sh`](verify-agentgateway-clients-macos.sh)
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
[`Verify-AgentgatewayClientsWindows.ps1`](Verify-AgentgatewayClientsWindows.ps1)
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

## Final interactive verification

After the script succeeds, restart the client and send a harmless, unique
prompt. Correlate the request by time in the agentgateway access log:

- Codex must send a successful `POST /v1/responses` request.
- Claude Desktop must send a successful `POST /v1/messages` request.

Confirm the expected hostname, route, authenticated identity when configured,
upstream provider, and successful status. Agentgateway logs must not contain
the bearer token or upstream provider credential.
