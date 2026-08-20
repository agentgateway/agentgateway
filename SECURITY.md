# Security Policy

## Reporting a Vulnerability

To report a vulnerability, [file a private vulnerability report](https://github.com/agentgateway/agentgateway/security/advisories/new).
This report will be privately reviewed by the Agentgateway security team.

**Please do not report security vulnerabilities through public GitHub issues.**
If you aren't sure if an issue is a security vulnerability, it's best to err on the side of caution and report it privately.

## Vulnerability Policy

This policy describes how the project handles security reports.
Project maintainers are responsible for determining whether a report describes a security vulnerability. If you are unsure, report the issue privately: a private report can later be made public, but a public issue cannot be made retroactively private.

The goal of this policy is to distinguish **security vulnerabilities**, which warrant security-specific handling and potentially urgent user action, from ordinary bugs, configuration mistakes, and product limitations.

A CVE is not simply "anything with security impact." We want CVEs to remain a useful signal to users rather than create scanner noise.

### Core principles

* **A vulnerability is something users reasonably need to be urgently informed about.** Hypothetical, highly contrived edge cases are generally bugs rather than vulnerabilities.
* **A vulnerability defeats a promised security boundary.** An attacker must be able to defeat a security boundary that agentgateway claims to enforce without already possessing equivalent authority. Outcomes caused by user configuration, user-written CEL, trusted extensions or services, documented behavior, or already-privileged access are generally bugs, limitations, or user error.
* **Maintainer discretion ultimately applies.** Classification is based on what best helps users, balancing actual risk against CVE and scanner signal-to-noise ratio.
* **User-written policy is the user's responsibility.** If a user writes an incorrect policy, even if it was a reasonable mistake, it is their responsibility to fix it.
* **Poor or ambiguous documentation is not itself a vulnerability.** We may fix documentation and call out in release notes that users should review affected configurations, without issuing a CVE.
* **External policy components are trusted, at least in part.** ext-auth, ext-proc, external rate limiting, and similar configured services are expected to behave correctly. A malicious or incorrectly implemented external policy service is generally not an agentgateway vulnerability.
* **Administrative interfaces are privileged.** Interfaces intended for localhost or trusted operators are not designed as an untrusted security boundary. Exposing them publicly is normally a deployment error.
* **Required privileges matter.** Issues remotely exploitable by an unauthenticated attacker are generally more serious than those requiring authentication, administrative access, infrastructure privileges, or equivalent authority. Existing privileges may reduce an issue's severity unless the issue crosses an additional meaningful security boundary.

### Classification examples

The following are high-level examples intended to explain how we generally apply this policy.
They are neither exhaustive nor guarantees that a similar report will receive the same classification.
Details, exploitability, impact, deployment assumptions, and other context matter; maintainers retain discretion over every classification.

Issues that are generally more likely to be considered vulnerabilities include:

* A request that reliably crashes the gateway.
* A remotely supplied request that causes resource consumption disproportionate to the attacker's effort under realistic deployment conditions.
* Unauthorized cross-namespace Kubernetes writes that violate an intended authorization boundary.

Issues that are generally more likely to be considered bugs, limitations, or user error include:

* A dangerous but documented default or behavior.
* A users CEL expression resulting in an unexpected behavior, including failing to compile/failing to evaluate - the author is required to be aware 
* An administrator-supplied configuration that crashes the control plane.
* A trusted ext-auth, ext-proc, or external rate-limiting component acting maliciously.
* LLM Guardrails not applying in the way a user intended.
