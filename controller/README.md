At time of writing this folder contains alot of logic that only works when run in the context of https://github.com/kgateway-dev/kgateway

It may be easiest to run commands from there unless you want to pull from ci. 
A good example is all references of hack/kind/setup-kind.sh working in kgateway but being replaced by est/setup/setup-kind-ci.sh when ci was [enabled](https://github.com/agentgateway/agentgateway/pull/938).