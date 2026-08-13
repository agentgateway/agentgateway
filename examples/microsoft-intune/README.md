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
report and use **Collect logs**. Intune includes its macOS agent logs from:

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
