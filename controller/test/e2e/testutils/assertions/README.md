# Assertion

If you intend to introduce a new assertion, please follow this approach:
- We want to avoid writing generic assertions, that are specific to certain tests. Assertions should contain no custom logic, and instead support dependency injection.
- If you are unsure if an assertion is generic, start by adding it directly to your test, and then you can make it more generic in a follow-up.

# Conventions

## Contextual Assertions

Some assertions provided in this package may consider the [install context](./provider.go#L25) for conditional logic based on metadata about the installation being asserted against.
