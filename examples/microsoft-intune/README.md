# Microsoft Intune client management examples

These example scripts let Microsoft Intune verify that managed Codex and
Claude Desktop clients use an approved agentgateway address. Each script can
check either client or both clients without returning configuration contents,
tokens, or provider credentials to Intune.

## Provide a Claude Desktop credential helper

Use a static agentgateway client key only for a limited pilot. The exported
Claude Desktop policy contains the key, so assign it only to the pilot group
and rotate or revoke it after testing. For a production gateway API key rollout
or subscription-passthrough mode, use an organization-owned credential helper.
A helper is an executable installed on the managed endpoint. Claude Desktop
runs it with no arguments whenever it needs an inference credential and sets
`CLAUDE_HELPER_CONTEXT` to describe why it is running.

The helper must:

- run as the signed-in user and retrieve that user's assigned gateway key or
  subscription token from Keychain, Credential Manager, or an internal secret
  broker;
- return exit code `0` and write only a bare token, or the supported JSON
  object, to standard output;
- write only nonsecret diagnostics to standard error; and
- return a nonzero exit code instead of waiting for user input when a
  noninteractive lookup or refresh cannot complete.

Claude Desktop caches the result for `inferenceCredentialHelperTtlSec` seconds
and re-runs the helper when necessary. The default TTL is 3600 seconds. The
`CLAUDE_HELPER_CONTEXT` value can be `interactive`, `mid-session-refresh`,
`scheduled-task`, `setup-test`, or `background`. Only the `interactive`
context should start an authentication flow that requires user input.

These examples do not include a universal credential helper executable. A
secure implementation must integrate with the organization's credential
provisioning system or secret broker. A script that embeds a shared key or
reads one from an ordinary plaintext file defeats the purpose of the helper.

Use this deployment checklist:

1. Build or obtain a signed helper that implements the [Claude Desktop
   credential-helper
   contract](https://claude.com/docs/third-party/claude-desktop/credential-helper).
2. Deploy it with Intune to a fixed absolute path that the user cannot modify.
3. Provision each user's or device's credential separately through secure
   storage or broker authorization. Do not include it in the helper, Intune
   script, or Claude Desktop profile.
4. Set `inferenceCredentialKind` to `helper-script` and set
   `inferenceCredentialHelper` to the absolute executable path.
5. Run the helper as the intended user with
   `CLAUDE_HELPER_CONTEXT=setup-test`. Confirm a successful exit without
   printing or recording its standard output.
6. Use Claude Desktop **Test connection**, then send an inference request and
   confirm it in the agentgateway access log.

The scripts check:

- whether the selected client is installed;
- whether its effective managed configuration contains the expected
  agentgateway address; and
- whether the managed device can reach that address and receive an HTTP
  response.

On macOS, the Codex verifier checks both user-specific and device-level managed
preferences, including `/Library/Managed Preferences/com.openai.codex.plist`.
It then falls back to the effective user preference domain. This supports an
Intune preference profile assigned through either management scope.

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
- On macOS, keep the default verification log or set the
  `AGENTGATEWAY_INTUNE_LOG_FILE` environment variable to another absolute
  `.log` path. Because Intune does not provide a custom environment-variable
  field for platform scripts, edit the `LOG_FILE` default before upload when
  the organization requires a different managed path.

Do not add an LLM provider key, bearer token, or another secret to either
script.

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
scripts require both the `gateway` inference provider and the exact approved
Gateway URL.

Before uploading a discovery script, replace its example URL with the approved
address. Include `/v1` for Codex and the configured route prefix, such as
`/claude`, for Claude Desktop. Keep the expected URL aligned with the managed
configuration policy.

### Configure custom compliance on macOS

1. Select the client-specific `discover-gateway-macos.sh` and
   `compliance.json` files. Create separate policies and assignments when
   different device groups require different clients.
2. Go to **Endpoint security > Device compliance > Scripts > Add > macOS** and
   upload the discovery script.
3. Set **Run this script using the logged on credentials** to **Yes**. Enable
   signature enforcement when the organization signs scripts.
4. Create a macOS compliance policy, add **Custom Compliance**, select the
   discovery script, and upload the matching `compliance.json`.
5. Assign the policy to the same pilot group as the application and managed
   configuration policies.

The macOS script prints only `true` or `false`, as required for this single
Boolean rule, and returns exit code `0` for either discovered value. A nonzero
exit code is reserved for a script execution error.

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

Each Windows script returns one compressed JSON object. For example:

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
