# Deprecation Policy

Agentgateway evolves over time, and user-facing functionality may occasionally need
to be replaced or removed. This policy gives users a predictable migration window
and gives contributors a consistent process for making incompatible changes.

This policy is based on the
[Kubernetes deprecation policy](https://kubernetes.io/docs/reference/using-api/deprecation-policy/)
and is adapted to agentgateway's release lifecycle and interfaces.

## Scope

This policy applies to supported, user-visible interfaces, including:

- Kubernetes custom resources and their fields, values, and behavior
- standalone configuration schemas and their fields and values
- Helm charts, including values, rendered resources, and upgrade behavior
- network APIs and protocol extensions defined by agentgateway
- command-line commands, flags, and output intended for automation
- documented features and user-visible behavior
- feature gates
- metrics intended for users

It does not apply to internal implementation details, undocumented development
interfaces, test utilities, or changes between arbitrary commits. Compatibility
guarantees apply between official releases.

## Stability levels

Interfaces have one of the following stability levels:

- **GA** (generally available or stable): suitable for production use.
- **Beta**: available for evaluation and production use with an increased risk of
  change.
- **Alpha** (experimental): under active development and subject to change or
  removal.

An interface is GA unless its documentation or name explicitly identifies it as
Alpha, Beta, experimental, or preview. When agentgateway exposes an interface from
an upstream project, its effective stability is no higher than the stability
assigned by that project.

In the requirements below, a "release" means a minor production release. Patch
releases and Alpha, Beta, or release-candidate builds do not count toward a
release-based minimum. When both a duration and a number of releases are specified,
the longer period applies.

## General requirements

The following requirements apply to every deprecation:

1. A replacement must be at the same or a higher stability level than the
   deprecated interface.
2. Deprecation must be announced in the release notes and documentation. The
   announcement must identify the replacement or migration path and the earliest
   release or date when removal is permitted.
3. The deprecated interface must continue to function during its deprecation
   period.
4. Use of a deprecated interface must produce a warning when a warning can be
   emitted without breaking compatibility. Warnings should identify the
   replacement and planned removal.
5. Removal must be called out in release notes and include upgrade or migration
   instructions.

Changing an interface's documented semantics or default in a way that can break
existing users is considered removal of the old behavior and is subject to this
policy.

## APIs and configuration

API elements include resources, fields, enumerated or constant values, standalone
configuration fields, and versioned component configuration.

The following rules apply:

1. An API element may only be removed by incrementing the API version. An element
   added to an API version must not be removed from that version or have its
   behavior changed incompatibly.
2. When conversion between API versions is provided, objects must round-trip
   between supported versions in a release without information loss. A release
   must continue to decode any representation that it may have persisted.
3. A supported API version must not be deprecated in favor of a less stable
   version.
4. A new preferred or storage version must not be selected until a production
   release has supported both the new and previous versions.

Minimum API lifetimes are shown below. The period before deprecation begins when
the interface is introduced. The period after deprecation begins when the
deprecation is announced.

| Stability | Before deprecation | After deprecation |
| --- | --- | --- |
| GA | No minimum | Next major, and 12 months or 2 releases |
| Beta | 9 months or 3 releases | 9 months or 3 releases |
| Alpha | No minimum | No minimum |

Deprecating a GA API does not permit its removal from the current major version.
Alpha APIs may be changed or removed without advance notice, but removals must
still be documented in release notes.

## Helm charts

Published Helm charts are user-facing APIs. A chart's API includes:

- the chart name, registry or repository location, and availability
- the keys, types, defaults, and behavior defined by `values.yaml` and any values
  schema
- resources rendered by default or by documented values
- resource names and namespaces, labels and selectors used for integration,
  ownership, and adoption
- install, upgrade, rollback, and uninstall behavior
- supported Helm and Kubernetes versions
- chart dependencies that affect user-supplied values or rendered resources

Template organization, formatting of rendered YAML, internal helper templates,
comments, tests, and generated metadata with no documented integration purpose
are implementation details.

The chart `version`, rather than `appVersion`, governs compatibility of the chart
API. As described in the
[Helm chart documentation](https://helm.sh/docs/topics/charts/), chart versions
must follow Semantic Versioning:

- Patch and minor chart releases must remain backward compatible.
- An incompatible chart change requires a new major chart version and must also
  satisfy the GA API deprecation period in this policy.
- A major chart version signals that manual upgrade action may be required. It
  does not replace the deprecation period or migration requirements.

Removing or renaming a value, changing its type or meaning, or changing its
default in a way that alters an existing installation is an incompatible change.
So are changes that rename or replace resources, alter immutable fields, change
selectors or ownership, or otherwise prevent a normal `helm upgrade`. Adding an
optional value is compatible only when its default preserves existing behavior.

A deprecated value must remain accepted and functional throughout its
deprecation period. When it is replaced:

1. The old and new values should be supported together where practical.
2. Precedence must be documented when both values are set.
3. `values.yaml`, generated value references, and the values schema must identify
   the old value as deprecated and name its replacement.
4. `NOTES.txt` should warn when use of the old value can be detected without
   failing an install or upgrade.
5. Tests must exercise upgrades using the deprecated value until it is removed.

Each incompatible change must be documented in a chart upgrade guide, grouped by
chart version, with before-and-after values and any required manual steps. Release
notes must identify affected chart versions and link to that guide.

Dropping a supported Helm or Kubernetes version, or making an incompatible
dependency upgrade, is a chart API change. It must follow a published support
policy or, when no more specific policy exists, the GA API requirements in this
document. The chart's `kubeVersion` constraint and compatibility documentation
must be updated in the release that makes the change.

To deprecate an entire chart:

1. Publish a final chart release with `deprecated: true` in `Chart.yaml`.
2. Document the replacement, migration procedure, support end date, and artifact
   location.
3. Keep already published chart artifacts available and unmodified for at least
   12 months or 2 releases after deprecation. The source may be removed after the
   final deprecated release, but its documentation and artifact location must
   remain available for that period.

Deprecating a chart does not authorize removal of installed custom resources or
other user data. CRDs delivered by a chart remain subject to the API and
configuration requirements in this policy, and their migration must be documented
separately.

## Command-line interfaces

Command-line interface (CLI) elements include commands, subcommands, flags, and
machine-readable output formats. Unless explicitly identified as Alpha or Beta, a
CLI element is GA.

User-facing CLI elements, such as `agctl` commands, must function after announced
deprecation for at least:

| Stability | Minimum deprecation period |
| --- | --- |
| GA | 12 months or 2 releases |
| Beta | 3 months or 1 release |
| Alpha | No minimum |

Operator-facing CLI elements, such as agentgateway process flags, must function
after announced deprecation for at least:

| Stability | Minimum deprecation period |
| --- | --- |
| GA | 6 months or 1 release |
| Beta | 3 months or 1 release |
| Alpha | No minimum |

Deprecated CLI elements must emit a warning when used. Help text must identify
their deprecated status and replacement.

## Features and behavior

A significant GA feature or user-visible behavior must continue to function for
at least 12 months after its announced deprecation. This applies to behavior that
affects request processing, policy enforcement, protocol compatibility, or the
operation of an agentgateway deployment.

When migration requires user action, the project should provide tooling,
documentation, compatibility modes, or other assistance where practical.

## Feature gates

Feature gates represent a feature's development lifecycle and are not long-term
interfaces. A feature gate must be deprecated when its feature reaches GA or is
removed.

| Transition | Minimum deprecation period |
| --- | --- |
| Beta feature to GA | 6 months or 2 releases |
| Beta feature to removal | 3 months or 1 release |
| Alpha feature to removal | No minimum |

Deprecated feature gates must produce a warning when used. Release notes and CLI
help must state whether the gate is still operational.

## Metrics

Metrics have `STABLE`, `BETA`, or `ALPHA` stability. Unless explicitly identified
otherwise, a documented metric is `STABLE`.

Metrics must be available for the following minimum period after introduction:

| Stability | Minimum lifetime |
| --- | --- |
| STABLE | 12 months or 4 releases |
| BETA | 8 months or 2 releases |
| ALPHA | No minimum |

After deprecation, metrics must continue to be available for at least:

| Stability | Minimum deprecation period |
| --- | --- |
| STABLE | 9 months or 3 releases |
| BETA | 4 months or 1 release |
| ALPHA | No minimum |

When practical, deprecated metrics should be marked in their help text and emit a
replacement metric alongside the old metric during the migration window.

## Deprecation process

A change that deprecates a supported interface must:

1. Open or reference a tracking issue describing the reason, affected users,
   replacement, migration steps, stability level, and earliest permitted removal.
2. Mark the interface as deprecated in source annotations, generated references,
   user documentation, schemas, Helm values and upgrade guides, and CLI help, as
   applicable.
3. Add a warning at the point of use when technically possible.
4. Include the deprecation in release notes for the release that begins the
   deprecation period.
5. Retain tests for the deprecated behavior until it is removed.

A change that removes a deprecated interface must confirm that the required
period has elapsed and must include release notes and migration guidance.

## Exceptions

Maintainers may shorten a deprecation period when continuing support would create
a critical security vulnerability, risk data loss, violate a legal requirement,
or depend on an upstream interface that is no longer available. The exception,
rationale, user impact, and safest available migration path must be documented
publicly.
