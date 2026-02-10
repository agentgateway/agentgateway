# Load Balancing Tests

## Background
### What is the goal?
Each end-to-end test is executed on a running Kubernetes Cluster. If we use a single cluster, and run tests serially, our CI pipeline will take a long time. Therefore, we load balance our tests against a batch of Kubernetes Clusters.

Our goal is to make the most efficient use of our hardware for executing tests in our CI pipeline.

### What was our previous strategy?
Our previous strategy was to group tests by domain. This did not scale well because different domains had a different amount of tests. Therefore, we noticed that it would take some test clusters twice as long to complete the tests as others.

### What is our current strategy?
Our current strategy is to group tests by runtime. This allows us to easily move tests between clusters as necessary, following the Kubernetes mantra "cattle, not pets".

The groupings are defined in our [GitHub action matrix](/.github/workflows/pr-kubernetes-tests.yaml). This allows us to isolate this complexity to our CI pipeline and not impact local development.

 _A side effect of this approach is that if you add a new test function and forget to add it in CI, it will not run. In the short term we have accepted this drawback, expecting PR review to identify it. If this is not sufficient, we will build automation to detect this._.

## Re-Balancing

### When should it occur?
Re-balancing of tests is intentionally a very easy action, though it shouldn't need to occur often. This should happen if:
- Tests on one cluster are completing well before tests on another cluster
- All clusters are exhausted, and we need to introduce a new cluster into the rotation

### Steps to take
1. Review the recent results from CI, and identify which tests can be migrated
2. Adjust the run functions that are invoked in our [GitHub action matrix](/.github/workflows/pr-kubernetes-tests.yaml)
4. Document the **new** results, on the matrix that runs the tests
4. Open a PR clearly documenting in the PR body the results before and after the change


## Adding a new test
When adding a new test suite:
1. Check the most recently merged PR's action for Kubernetes tests
2. Determine the cluster with the lowest runtime
3. Add your test suite to that cluster's definition in our [GitHub action matrix](/.github/workflows/pr-kubernetes-tests.yaml)
