# Actions

Actions are intended to mirror actions that users of the product can take. 


If you intend to introduce a new action, please follow this approach:
- We want to avoid writing custom code that is _just_ used by our tests. The core action should live in a utility package ([utils](/pkg/utils), [cli-utils](/pkg/cliutil), ...etc)

**Example**:
If you expanded the functionality that a user would perform via `kubectl`
1. Introduce that logic into the [kubectl](/pkg/utils/kubeutils/kubectl) package