# APIs for kgateway

This directory contains Go types for kgateway APIs & custom resources.

## Adding a new API / CRD

These are the steps required to add a new CRD to be used in the Kubernetes Gateway integration:

1. If creating a new API version (e.g. `v1`, `v2alpha1`), create a new directory for the version and create a `doc.go` file with the `// +kubebuilder:object:generate=true` annotation, so that Go types in that directory will be converted into CRDs when codegen is run.
    - The `groupName` marker specifies the API group name for the generated CRD.
    - RBAC rules are defined via the `+kubebuilder:rbac` annotation (note: this annotation should not belong to the type, but rather the file or package).
2. Create a `_types.go` file in the API version directory. Following [gateway_parameters_types.go](/api/v1alpha1/kgateway/gateway_parameters_types.go) as an example:
    - Define a struct for the resource (containing the metadata fields, `Spec`, and `Status`). Follow the [API guidelines](#api-guidelines) below.
    - Define a struct for the resource list (containing the metadata fields and `Items`)
3. Run codegen via `make generated-code -B`. This will invoke the `controller-gen` command specified in [generate.go](/hack/generate.go), which should result in the following:
    - A `zz_generated.deepcopy.go` file is created in the same directory as the Go types.
    - A `zz_generated.register.go` file is created in the same directory as the Go types, to help with registering the Go types with the scheme.
    - CRDs are generated in the CRD helm chart template dir: [install/helm/kgateway-crds/templates](/install/helm/kgateway-crds/templates)
    - RBAC roles are generated in [install/helm/kgateway/templates/role.yaml](/install/helm/kgateway/templates/role.yaml)
    - Updates the [api/applyconfiguration](/api/applyconfiguration), [pkg/generated](/pkg/generated) and [pkg/client](/pkg/client) folders with kube clients. These are used in plugin initialization and the fake client is used in tests.

## API guidelines
- Include documentation as well as any appropriate json and kubebuilder annotations on all fields.
- Document the default value for each field, if applicable.
- For optional fields:
    - Use the `+optional` marker.
    - Use the `omitempty` json struct tag.
    - Use pointer types (e.g. `*string`), unless the type has a nil zero value (e.g. slices/maps). An exception is if the field has a default value (`+kubebuilder:default=...`); then it it acceptable to use a non-pointer type.
- For required fields:
    - Use the `+required` marker.
    - Required fields MUST NOT set the `omitempty` json struct tag.
- Avoid using slices with pointers (e.g. use `[]string` instead of `[]*string`). See: https://github.com/kubernetes/code-generator/issues/166
- For time duration fields, use the `metav1.Duration` type and use CEL validation rules to ensure it is within the correct range.

### Custom validation markers

The CRD generation pipeline supports custom kubebuilder validation markers, implemented in
`controller/hack/crdgen`.

- `+kubebuilder:validation:AtLeastOneFieldSet` requires at least one field on the struct to be set.
- `+kubebuilder:validation:ExactlyOneFieldSet` requires exactly one field on the struct to be set.
- `+kubebuilder:validation:IfThenOnlyFields:if="...",fields=a;b;c,message="..."` requires that, when `if` is true,
  only the listed fields may be set.

Optional arguments for `AtLeastOneFieldSet` and `ExactlyOneFieldSet`:

- `fields=a;b;c` to explicitly pick fields to count.
- `exclude=a;b` to drop fields from the counted set.
- `message="..."` to override the generated validation message.

These markers are intended for struct/type comments (`markers.DescribesType`).

### Replicating Gateway API policies in TrafficPolicy API

Gateway API policies may be replicated as a part of the TrafficPolicy API to enable policy attachment at different levels in the config hierarchy, such as at the Gateway, Gateway's listener, or route level. The following guidelines should be considered when doing so:
- When the Gateway API types are considered sufficient to meet the requirements, they can be embedded as is in the TrafficPolicy API. TrafficPolicy's `cors` is an example where the Gateway API type `HTTPCORSFilter` is embedded directly.
- When embedding the Gateway API type, it is important to consider whether the type is marked as `<gateway:experimental>`, as experimental types may introduce breaking changes and should be noted similarly in the TrafficPolicy API. It is discouraged to embed experimental types in the TrafficPolicy API. However, if there is a breaking change in the Gateway API type, it is recommended to replicate the previous version of that type into the TrafficPolicy API and not propagate the breaking change to the TrafficPolicy API.
- When the Gateway API types are not sufficient and a more advanced API is required, a new type should be created in the TrafficPolicy API instead of embedding the Gateway API type. TrafficPolicy's `retry` and `timeouts` are examples that define new types instead of reusing the `HTTPRouteRetry` and `HTTPRouteTimeouts` types from the Gateway API.
