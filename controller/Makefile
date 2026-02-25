# imports should be after the set up flags so are lower

# https://www.gnu.org/software/make/manual/html_node/Special-Variables.html#Special-Variables
.DEFAULT_GOAL := help
SHELL := /bin/bash
export PATH := ../tools:$(PATH)
#----------------------------------------------------------------------------------
# Help
#----------------------------------------------------------------------------------
# Our Makefile is quite large, and hard to reason through
# `make help` can be used to self-document targets
# To update a target to be self-documenting (and appear with the `help` command),
# place a comment after the target that is prefixed by `##`. For example:
#	custom-target: ## comment that will appear in the documentation when running `make help`
#
# **NOTE TO DEVELOPERS**
# As you encounter make targets that are frequently used, please make them self-documenting
.PHONY: help
help: NAME_COLUMN_WIDTH=35
help: LINE_COLUMN_WIDTH=5
help: ## Output the self-documenting make targets
	@grep -hnE '^[%a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = "[:]|(## )"}; {printf "\033[36mL%-$(LINE_COLUMN_WIDTH)s%-$(NAME_COLUMN_WIDTH)s\033[0m %s\n", $$1, $$2, $$4}'

#----------------------------------------------------------------------------------
# Base
#----------------------------------------------------------------------------------

ROOTDIR := $(shell pwd)
OUTPUT_DIR ?= $(ROOTDIR)/_output

export IMAGE_REGISTRY ?= ghcr.io/agentgateway

# Kind of a hack to make sure _output exists
z := $(shell mkdir -p $(OUTPUT_DIR))

BUILDX_BUILD ?= docker buildx build -q

# A 'v'-prefixed semver used only locally. Most calling GHA jobs customize
# this. Exported for use in goreleaser.yaml. Because our docker images are
# tagged with a 'v' prefix, we use the prefix here and strip the 'v' prefix
# where actual semver is desired.
VERSION ?= v1.0.1-dev
TAG ?= $(VERSION)
export VERSION

export LDFLAGS := -X 'github.com/agentgateway/agentgateway/controller/pkg/version.Version=$(VERSION)' -s -w
export GCFLAGS ?=

UNAME_M := $(shell uname -m)
# if `GOARCH` is set, then it will keep its value. Else, it will be changed based off the machine's host architecture.
# if the machines architecture is set to arm64 then we want to set the appropriate values, else we only support amd64
IS_ARM_MACHINE := $(or	$(filter $(UNAME_M), arm64), $(filter $(UNAME_M), aarch64))
ifneq ($(IS_ARM_MACHINE), )
	ifneq ($(GOARCH), amd64)
		GOARCH := arm64
	endif
else
	# currently we only support arm64 and amd64 as a GOARCH option.
	ifneq ($(GOARCH), arm64)
		GOARCH := amd64
	endif
endif

PLATFORM := --platform=linux/$(GOARCH)

GOOS ?= $(shell uname -s | tr '[:upper:]' '[:lower:]')

GO_BUILD_FLAGS := CGO_ENABLED=0 GOARCH=$(GOARCH)

TEST_ASSET_DIR ?= $(ROOTDIR)/_test

# This is the location where assets are placed after a test failure
# This is used by our e2e tests to emit information about the running instance of agentgateway
BUG_REPORT_DIR := $(TEST_ASSET_DIR)/bug_report
$(BUG_REPORT_DIR):
	mkdir -p $(BUG_REPORT_DIR)

# Base Alpine image used for all containers. Exported for use in goreleaser.yaml.
export ALPINE_BASE_IMAGE ?= alpine:3.17.6

DEPSGOBIN ?= $(OUTPUT_DIR)
ANALYZE_ARGS ?= --fix --verbose --max-issues-per-linter 0 --max-same-issues 0
CUSTOM_GOLANGCI_LINT_BIN ?= $(DEPSGOBIN)/golangci-lint-custom
CUSTOM_GOLANGCI_LINT_RUN ?= $(CUSTOM_GOLANGCI_LINT_BIN) run --build-tags e2e
CUSTOM_GOLANGCI_LINT_FMT ?= $(CUSTOM_GOLANGCI_LINT_BIN) fmt

#----------------------------------------------------------------------------------
# Macros
#----------------------------------------------------------------------------------

# This macro takes a relative path as its only argument and returns all the files
# in the tree rooted at that directory that match the given criteria.
get_sources = $(shell find $(1) -name "*.go" | grep -v test | grep -v generated.go | grep -v mock_)

#----------------------------------------------------------------------------------
# Repo setup
#----------------------------------------------------------------------------------

.PHONY: init-git-hooks
init-git-hooks:  ## Use the tracked version of Git hooks from this repo
	git config core.hooksPath .githooks

.PHONY: fmt
fmt:  ## Format the code with golangci-lint
	golangci-lint fmt ./...

.PHONY: fmt-changed
fmt-changed: ## Format only the changed code with golangci-lint (skip deleted files)
	git status -s -uno | awk '{print $$2}' | grep '.*.go$$' | xargs -r -I{} bash -lc '[ -f "{}" ] && $(CUSTOM_GOLANGCI_LINT_FMT) "{}" || true'

# must be a separate target so that make waits for it to complete before moving on
.PHONY: mod-download
mod-download:  ## Download the dependencies
	cd .. && go mod download all
	cd ../tools && go mod download all

.PHONY: mod-tidy-nested
mod-tidy-nested:  ## Tidy go mod files in nested modules
	@echo "Tidying tools..." && cd ../tools && go mod tidy
	@echo "Tidying test/e2e/defaults/extproc..." && cd test/e2e/defaults/extproc && go mod tidy

.PHONY: mod-tidy
mod-tidy: mod-download mod-tidy-nested ## Tidy the go mod file
	cd .. && go mod tidy

#----------------------------------------------------------------------------
# Analyze
#----------------------------------------------------------------------------

.PHONY: analyze
analyze: $(CUSTOM_GOLANGCI_LINT_BIN)  ## Run golangci-lint. Override options with ANALYZE_ARGS.
	$(CUSTOM_GOLANGCI_LINT_RUN) $(ANALYZE_ARGS) ./...

$(CUSTOM_GOLANGCI_LINT_BIN): ../go.mod ../go.sum .custom-gcl.yml
	golangci-lint custom

.PHONY: lint-actions
lint-actions: ## Lint the GitHub Actions workflows
	actionlint

#----------------------------------------------------------------------------------
# Ginkgo Tests
#----------------------------------------------------------------------------------

FLAKE_ATTEMPTS ?= 3
GINKGO_VERSION ?= $(shell echo $(shell go list -m github.com/onsi/ginkgo/v2) | cut -d' ' -f2)
GINKGO_ENV ?= ACK_GINKGO_RC=true ACK_GINKGO_DEPRECATIONS=$(GINKGO_VERSION)

GINKGO_FLAGS ?= -tags=purego --trace -progress -race --fail-fast -fail-on-pending --randomize-all --compilers=5 --flake-attempts=$(FLAKE_ATTEMPTS)
GINKGO_REPORT_FLAGS ?= --json-report=test-report.json --junit-report=junit.xml -output-dir=$(OUTPUT_DIR)
GINKGO_COVERAGE_FLAGS ?= --cover --covermode=atomic --coverprofile=coverage.cov
TEST_PKG ?= ./... # Default to run all tests except e2e tests

# This is a way for a user executing `make test` to be able to provide flags which we do not include by default
# For example, you may want to run tests multiple times, or with various timeouts
GINKGO_USER_FLAGS ?=
GINKGO ?= ginkgo

.PHONY: test
test: ## Run all tests with ginkgo, or only run the test package at {TEST_PKG} if it is specified
	$(GO_TEST_ENV) $(GINKGO_ENV) $(GINKGO) -ldflags='$(LDFLAGS)' \
		$(GINKGO_FLAGS) $(GINKGO_REPORT_FLAGS) $(GINKGO_USER_FLAGS) \
		$(TEST_PKG)

# To run only e2e tests, we restrict to ./test/e2e/tests. We say
# '-tags=e2e' because untagged files contain unit tests cases, not e2e test
# cases, so we have to allow `go` to see our e2e tests. Someone might forget to
# label a new e2e test case with `//go:build e2e`, in which case `make unit`
# will error because there is no kind cluster.
#
# This build-tag approach makes unit tests run faster since e2e tests are not
# compiled, but it might be better to set an environment variable `E2E=true`
# and have end-to-end test cases report that they were skipped if it's not
# truthy. As it stands, a developer who runs `make unit` or `go test ./...`
# will still have e2e tests run by Github Actions once they publish a pull
# request.
.PHONY: e2e-test
e2e-test: extproc-server-docker kind-load-extproc-server
e2e-test: go-test
e2e-test: TEST_TAG = e2e
e2e-test: GO_TEST_ARGS = $(E2E_GO_TEST_ARGS)


# https://go.dev/blog/cover#heat-maps
.PHONY: test-with-coverage
test-with-coverage: GINKGO_FLAGS += $(GINKGO_COVERAGE_FLAGS)
test-with-coverage: test
	cover -html $(OUTPUT_DIR)/coverage.cov

.PHONY: golden-deployer
golden-deployer:  ## Refreshes golden files for ./test/deployer snapshot testing
	REFRESH_GOLDEN=true go test ./test/deployer/... > /dev/null || true
	@echo ""
	@echo "This must pass after refreshing:"
	go test ./test/deployer/...

.PHONY: golden-helm
golden-helm:  ## Refreshes golden files for ./test/helm snapshot testing
	REFRESH_GOLDEN=true go test ./test/helm/... > /dev/null || true
	@echo ""
	@echo "This must pass after refreshing:"
	go test ./test/helm/...

#----------------------------------------------------------------------------------
# Go Tests
#----------------------------------------------------------------------------------

# Fix for macOS linker warning with race detector on arm64 (which still warns
# you that -ld_classic is deprecated, but that's better than broken race
# condition detection)
# See: https://github.com/golang/go/issues/61229
GO_TEST_ENV ?=
ifeq ($(GOOS), darwin)
ifeq ($(GOARCH), arm64)
	override GO_TEST_ENV := CGO_LDFLAGS="-Wl,-ld_classic"
endif
endif

# Skip -race on e2e. This requires building the codebase twice, and provides no value as the only code executed is test code.
# Skip -vet; we already run it on the linter step and its very slow.
E2E_GO_TEST_ARGS ?= -vet=off -timeout=25m -outputdir=$(OUTPUT_DIR)
# Testing flags: https://pkg.go.dev/cmd/go#hdr-Testing_flags
# The default timeout for a suite is 10 minutes, but this can be overridden by setting the -timeout flag. Currently set
# to 25 minutes based on the time it takes to run the longest test setup.
GO_TEST_ARGS ?= -timeout=25m -outputdir=$(OUTPUT_DIR) -race
GO_TEST_COVERAGE_ARGS ?= --cover --covermode=atomic --coverprofile=cover.out
GO_TEST_COVERAGE ?= github.com/vladopajic/go-test-coverage/v2

# This is a way for a user executing `make go-test` to be able to provide args which we do not include by default
# For example, you may want to run tests multiple times, or with various timeouts
GO_TEST_USER_ARGS ?=
GO_TEST_RETRIES ?= 0
GOTESTSUM ?= gotestsum
GOTESTSUM_ARGS ?= --format=standard-verbose

.PHONY: go-test
go-test: ## Run all tests, or only run the test package at {TEST_PKG} if it is specified
go-test: reset-bug-report
	$(GO_TEST_ENV) $(GOTESTSUM) $(GOTESTSUM_ARGS) --rerun-fails-abort-on-data-race --rerun-fails=$(GO_TEST_RETRIES) --packages="$(TEST_PKG)" -- -ldflags='$(LDFLAGS)' $(if $(TEST_TAG),-tags=$(TEST_TAG)) $(GO_TEST_ARGS) $(GO_TEST_USER_ARGS)

# https://go.dev/blog/cover#heat-maps
.PHONY: go-test-with-coverage
go-test-with-coverage: GO_TEST_ARGS += $(GO_TEST_COVERAGE_ARGS)
go-test-with-coverage: go-test

# https://go.dev/blog/cover#heat-maps
.PHONY: unit-with-coverage
unit-with-coverage:
	@$(MAKE) --no-print-directory unit GO_TEST_ARGS="$(GO_TEST_ARGS) $(GO_TEST_COVERAGE_ARGS)"

.PHONY: unit
unit: ## Run all unit tests (excludes e2e tests)
	@echo "Running unit tests (excluding e2e)..."
	@$(MAKE) --no-print-directory go-test TEST_TAG=""

.PHONY: validate-test-coverage
validate-test-coverage: ## Validate the test coverage
	$(GO_TEST_COVERAGE) --config=./test_coverage.yml

# https://go.dev/blog/cover#heat-maps
.PHONY: view-test-coverage
view-test-coverage:
	cover -html $(OUTPUT_DIR)/cover.out

#----------------------------------------------------------------------------------
# MARK: Clean
#----------------------------------------------------------------------------------

# Important to clean before pushing new releases. Dockerfiles and binaries may not update properly
.PHONY: clean
clean:
	rm -rf _output
	rm -rf _test
	git clean -f -X install
	@# Note: _output removal also cleans stamps since STAMP_DIR is in _output

.PHONY: clean-tests
clean-tests:
	find * -type f -name '*.test' -exec rm {} \;
	find * -type f -name '*.cov' -exec rm {} \;
	find * -type f -name 'junit*.xml' -exec rm {} \;

# NB: 'reset-bug-report: clean-bug-report $(BUG_REPORT_DIR)' would be a subtle
# bug since we would never run 'mkdir' if the directory already existed.
.PHONY: reset-bug-report
reset-bug-report: clean-bug-report
	@$(MAKE) --no-print-directory $(BUG_REPORT_DIR)

.PHONY: clean-bug-report
clean-bug-report:
	rm -rf $(BUG_REPORT_DIR)

#----------------------------------------------------------------------------------
# MARK: Generated Code
#----------------------------------------------------------------------------------
# This section uses stamp files to optimize 'make generate-all' by tracking dependencies.
#
# For local development:
#   - 'make generate-all' only regenerates code when source files change (fast!)
#   - Use 'make clean-stamps' to force full regeneration
#
# For CI (always regenerates to catch dependency tracking bugs):
#   - 'make verify' cleans stamps and always regenerates everything
#   - This ensures CI catches any mistakes in our dependency tracking
#
# How it works:
#   - Each generation step creates a stamp file in _output/stamps/
#   - Make compares stamp file timestamps with source file timestamps
#   - Only re-runs steps when source files are newer than stamps
#----------------------------------------------------------------------------------

# Stamp directory for tracking generation steps
STAMP_DIR := $(OUTPUT_DIR)/stamps
$(STAMP_DIR):
	mkdir -p $(STAMP_DIR)

# Source files that trigger API codegen
API_SOURCE_FILES := $(shell find api/v1alpha1 -name "*.go" ! -name "zz_generated*")
API_SOURCE_FILES += hack/generate.sh hack/generate.go

# Files that track dependency changes (in root module)
MOD_FILES := ../go.mod ../go.sum

# Clean generated code
.PHONY: clean-gen
clean-gen:
	rm -rf api/applyconfiguration
	rm -rf pkg/generated/openapi
	rm -rf pkg/client
	rm -f install/helm/agentgateway-crds/templates/agentgateway.dev_*.yaml

# Clean all stamp files to force regeneration
.PHONY: clean-stamps
clean-stamps:
	rm -rf $(STAMP_DIR)

# API code generation with dependency tracking
$(STAMP_DIR)/go-generate-apis: $(API_SOURCE_FILES) | $(STAMP_DIR)
	@echo "Running API code generation..."
	GO111MODULE=on go generate ./hack/...
	$(MAKE) fmt-changed
	@touch $@

# Combine both generation steps
$(STAMP_DIR)/go-generate-all: $(STAMP_DIR)/go-generate-apis
	@touch $@

# Module tidy with dependency tracking
$(STAMP_DIR)/mod-tidy: $(MOD_FILES) | $(STAMP_DIR)
	@echo "Running mod tidy..."
	@$(MAKE) --no-print-directory mod-download
	@$(MAKE) --no-print-directory mod-tidy-nested
	cd .. && go mod tidy
	@touch $@

# License generation with dependency tracking
$(STAMP_DIR)/generate-licenses: $(MOD_FILES) | $(STAMP_DIR)
	@echo "Generating licenses..."
	GO111MODULE=on go run hack/utils/oss_compliance/oss_compliance.go osagen -c "GNU General Public License v2.0,GNU General Public License v3.0,GNU Lesser General Public License v2.1,GNU Lesser General Public License v3.0,GNU Affero General Public License v3.0"
	GO111MODULE=on go run hack/utils/oss_compliance/oss_compliance.go osagen -s "Mozilla Public License 2.0,GNU General Public License v2.0,GNU General Public License v3.0,GNU Lesser General Public License v2.1,GNU Lesser General Public License v3.0,GNU Affero General Public License v3.0"> hack/utils/oss_compliance/osa_provided.md
	GO111MODULE=on go run hack/utils/oss_compliance/oss_compliance.go osagen -i "Mozilla Public License 2.0"> hack/utils/oss_compliance/osa_included.md
	@touch $@

# Formatting - only runs if generation steps changed
$(STAMP_DIR)/fmt: $(STAMP_DIR)/go-generate-all $(CUSTOM_GOLANGCI_LINT_BIN)
	@echo "Formatting code..."
	$(CUSTOM_GOLANGCI_LINT_FMT) ./...
	@touch $@

# Fast generation using stamp files (for local development)
$(STAMP_DIR)/generated-code: $(STAMP_DIR)/go-generate-all $(STAMP_DIR)/mod-tidy $(STAMP_DIR)/generate-licenses $(STAMP_DIR)/fmt
	@touch $@

.PHONY: verify
verify: generated-code  ## Verify that generated code is up to date (always regenerates for CI safety)
	git diff -U3 --exit-code

.PHONY: generate-all
generate-all: $(STAMP_DIR)/generated-code  ## Generate all code with optimized dependencies (uses stamp files for speed)

.PHONY: generate
generate: generate-all  ## Alias for generate

# Force full regeneration by cleaning stamps and generated files
.PHONY: generated-code
generated-code: clean-gen clean-stamps  ## Force regenerate all code (always runs, ignoring stamps)
	@$(MAKE) --no-print-directory generate-all

# Convenience PHONY targets that trigger stamp-based generation
.PHONY: go-generate-all
go-generate-all: $(STAMP_DIR)/go-generate-all  ## Run all go generate directives (with dependency tracking)

.PHONY: go-generate-apis
go-generate-apis: $(STAMP_DIR)/go-generate-apis  ## Run all go generate directives in the repo, including codegen for protos, mockgen, and more

.PHONY: go-generate-mocks
go-generate-mocks: $(STAMP_DIR)/go-generate-mocks  ## Runs all generate directives for mockgen in the repo

.PHONY: generate-licenses
generate-licenses: $(STAMP_DIR)/generate-licenses  ## Generate the licenses for the project

#----------------------------------------------------------------------------------
# Controller
#----------------------------------------------------------------------------------

# TODO: rename to agentgateway-controller
CONTROLLER_OUTPUT_DIR=$(OUTPUT_DIR)/pkg/agentgateway
export AGENTGATEWAY_IMAGE_REPO ?= agentgateway-controller

.PHONY: agentgateway-controller
agentgateway-controller:
	 $(GO_BUILD_FLAGS) GOOS=linux go build -ldflags='$(LDFLAGS)' -gcflags='$(GCFLAGS)' -o $(CONTROLLER_OUTPUT_DIR)/agentgateway-linux-$(GOARCH) ./cmd/agentgateway

$(CONTROLLER_OUTPUT_DIR)/Dockerfile.agentgateway: cmd/agentgateway/Dockerfile.agentgateway
	cp $< $@

.PHONY: agentgateway-controller-docker
agentgateway-controller-docker: agentgateway-controller $(CONTROLLER_OUTPUT_DIR)/Dockerfile.agentgateway
	$(BUILDX_BUILD) --load $(PLATFORM) $(CONTROLLER_OUTPUT_DIR) -f $(CONTROLLER_OUTPUT_DIR)/Dockerfile.agentgateway \
		--build-arg GOARCH=$(GOARCH) \
		-t $(IMAGE_REGISTRY)/$(AGENTGATEWAY_IMAGE_REPO):$(VERSION)

.PHONY: agentgateway-controller-docker-local
agentgateway-controller-docker-local: agentgateway-controller $(CONTROLLER_OUTPUT_DIR)/Dockerfile.agentgateway
	docker buildx build --push $(PLATFORM) $(CONTROLLER_OUTPUT_DIR) -f $(CONTROLLER_OUTPUT_DIR)/Dockerfile.agentgateway \
		--build-arg GOARCH=$(GOARCH) \
		-t localhost:5000/$(AGENTGATEWAY_IMAGE_REPO):$(TAG)

#----------------------------------------------------------------------------------
# dummy idp (used in e2e tests)
#----------------------------------------------------------------------------------

DUMMY_IDP_DIR=hack/dummy-idp
DUMMY_IDP_OUTPUT_DIR=$(OUTPUT_DIR)/$(DUMMY_IDP_DIR)
export DUMMY_IDP_IMAGE_REPO ?= dummy-idp
DUMMY_IDP_VERSION=0.0.1

.PHONY: dummy-idp
dummy-idp:
	$(GO_BUILD_FLAGS) GOOS=linux go build -ldflags='$(LDFLAGS)' -gcflags='$(GCFLAGS)' -o $(DUMMY_IDP_OUTPUT_DIR)/dummy-idp-linux-$(GOARCH) ./hack/dummy-idp...

$(DUMMY_IDP_OUTPUT_DIR)/Dockerfile.dummy-idp: ./hack/dummy-idp/Dockerfile
	cp $< $@

.PHONY: dummy-idp-docker
dummy-idp-docker: dummy-idp $(DUMMY_IDP_OUTPUT_DIR)/Dockerfile.dummy-idp
	$(BUILDX_BUILD) --load $(PLATFORM) $(DUMMY_IDP_OUTPUT_DIR) -f $(DUMMY_IDP_OUTPUT_DIR)/Dockerfile.dummy-idp \
		--build-arg GOARCH=$(GOARCH) \
		--build-arg BASE_IMAGE=$(ALPINE_BASE_IMAGE) \
		-t $(IMAGE_REGISTRY)/$(DUMMY_IDP_IMAGE_REPO):$(DUMMY_IDP_VERSION)

.PHONY: kind-load-dummy-idp
kind-load-dummy-idp:
	kind load docker-image $(IMAGE_REGISTRY)/$(DUMMY_IDP_IMAGE_REPO):$(DUMMY_IDP_VERSION) --name $(CLUSTER_NAME)

#----------------------------------------------------------------------------------
# extproc-server (used in e2e tests)
#----------------------------------------------------------------------------------

EXTPROC_SERVER_DIR=test/e2e/defaults/extproc
EXTPROC_SERVER_OUTPUT_DIR=$(OUTPUT_DIR)/$(EXTPROC_SERVER_DIR)
export EXTPROC_SERVER_IMAGE_REPO ?= extproc-server
EXTPROC_SERVER_VERSION=0.0.1

$(EXTPROC_SERVER_OUTPUT_DIR)/.docker-stamp-$(EXTPROC_SERVER_VERSION)-$(GOARCH): $(shell find $(EXTPROC_SERVER_DIR) -name '*.go') $(EXTPROC_SERVER_DIR)/Dockerfile
	$(BUILDX_BUILD) --load $(PLATFORM) $(EXTPROC_SERVER_DIR) -f $(EXTPROC_SERVER_DIR)/Dockerfile \
		-t $(IMAGE_REGISTRY)/$(EXTPROC_SERVER_IMAGE_REPO):$(EXTPROC_SERVER_VERSION)
	@mkdir -p $(dir $@)
	@touch $@

.PHONY: extproc-server-docker
extproc-server-docker: $(EXTPROC_SERVER_OUTPUT_DIR)/.docker-stamp-$(EXTPROC_SERVER_VERSION)-$(GOARCH)

.PHONY: kind-load-extproc-server
kind-load-extproc-server:
	kind load docker-image $(IMAGE_REGISTRY)/$(EXTPROC_SERVER_IMAGE_REPO):$(EXTPROC_SERVER_VERSION) --name $(CLUSTER_NAME)

#----------------------------------------------------------------------------------
# Helm
#----------------------------------------------------------------------------------

HELM ?= helm
# Publish charts with both semver variants to preserve compatibility.
# Keep VERSION as-is for image tags and other release identifiers.
HELM_CHART_VERSION_NO_V ?= $(patsubst v%,%,$(VERSION))
HELM_CHART_VERSION_V ?= v$(HELM_CHART_VERSION_NO_V)
HELM_PACKAGE_ARGS ?= --version $(HELM_CHART_VERSION_NO_V) --app-version $(HELM_CHART_VERSION_NO_V)
HELM_CHART_DIR_AGW=install/helm/agentgateway
HELM_CHART_DIR_AGW_CRD=install/helm/agentgateway-crds
HELM_ADDITIONAL_VALUES ?= hack/helm/dev.yaml

.PHONY: package-agentgateway-charts
package-agentgateway-charts: package-agentgateway-chart package-agentgateway-crd-chart ## Package the agentgateway charts

.PHONY: package-agentgateway-chart
package-agentgateway-chart: ## Package the agentgateway chart
	mkdir -p $(TEST_ASSET_DIR); \
	$(HELM) package $(HELM_PACKAGE_ARGS) --destination $(TEST_ASSET_DIR) $(HELM_CHART_DIR_AGW); \
	$(HELM) repo index $(TEST_ASSET_DIR);

.PHONY: package-agentgateway-crd-chart
package-agentgateway-crd-chart: ## Package the agentgateway crd chart
	mkdir -p $(TEST_ASSET_DIR); \
	$(HELM) package $(HELM_PACKAGE_ARGS) --destination $(TEST_ASSET_DIR) $(HELM_CHART_DIR_AGW_CRD); \
	$(HELM) repo index $(TEST_ASSET_DIR);

CHART_NAMES := agentgateway agentgateway-crds

.PHONY: release-charts
release-charts: package-agentgateway-charts
	mkdir -p $(TEST_ASSET_DIR); \
	$(HELM) package --version $(HELM_CHART_VERSION_V) --app-version $(HELM_CHART_VERSION_V) --destination $(TEST_ASSET_DIR) $(HELM_CHART_DIR_AGW); \
	$(HELM) repo index $(TEST_ASSET_DIR);
	mkdir -p $(TEST_ASSET_DIR); \
	$(HELM) package --version $(HELM_CHART_VERSION_V) --app-version $(HELM_CHART_VERSION_V) --destination $(TEST_ASSET_DIR) $(HELM_CHART_DIR_AGW_CRD); \
	$(HELM) repo index $(TEST_ASSET_DIR);
	@for chart in $(CHART_NAMES); do \
		$(HELM) push $(TEST_ASSET_DIR)/$$chart-$(HELM_CHART_VERSION_NO_V).tgz oci://$(IMAGE_REGISTRY)/charts; \
		$(HELM) push $(TEST_ASSET_DIR)/$$chart-$(HELM_CHART_VERSION_V).tgz oci://$(IMAGE_REGISTRY)/charts; \
	done;

.PHONY: deploy-agentgateway-crd-chart
deploy-agentgateway-crd-chart: ## Deploy the agentgateway crd chart
	$(HELM) upgrade --install agentgateway-crds $(TEST_ASSET_DIR)/agentgateway-crds-$(VERSION).tgz --namespace $(INSTALL_NAMESPACE) --create-namespace

.PHONY: deploy-agentgateway-chart
deploy-agentgateway-chart: ## Deploy the agentgateway chart
	$(HELM) upgrade --install agentgateway $(TEST_ASSET_DIR)/agentgateway-$(VERSION).tgz \
	--namespace $(INSTALL_NAMESPACE) --create-namespace \
	--set image.registry=$(IMAGE_REGISTRY) \
	--set image.tag=$(VERSION) \
	--set controller.image.repository=$(AGENTGATEWAY_IMAGE_REPO) \
	-f $(HELM_ADDITIONAL_VALUES)

.PHONY: lint-charts
lint-charts: ## Lint the agentgateway charts
	$(HELM) lint $(HELM_CHART_DIR_AGW)
	$(HELM) lint $(HELM_CHART_DIR_AGW_CRD)

#----------------------------------------------------------------------------------
# Release
#----------------------------------------------------------------------------------

.PHONY: release-notes
release-notes: ## Generate release notes (PREVIOUS_TAG required, CURRENT_TAG optional)
	./hack/generate-release-notes.sh -p $(PREVIOUS_TAG) -c $(or $(CURRENT_TAG),HEAD)

#----------------------------------------------------------------------------------
# MARK: Development
#----------------------------------------------------------------------------------

CLUSTER_NAME ?= kind
INSTALL_NAMESPACE ?= agentgateway-system

# The version of the Node Docker image to use for booting the kind cluster: https://hub.docker.com/r/kindest/node/tags
# This version should stay in sync with `hack/kind/setup-kind.sh`.
CLUSTER_NODE_VERSION ?= v1.35.0@sha256:452d707d4862f52530247495d180205e029056831160e22870e37e3f6c1ac31f

.PHONY: kind-create
kind-create: ## Create a KinD cluster
	kind get clusters | grep $(CLUSTER_NAME) || kind create cluster --name $(CLUSTER_NAME) --image kindest/node:$(CLUSTER_NODE_VERSION)

CONFORMANCE_CHANNEL ?= experimental
CONFORMANCE_VERSION ?= v1.4.1
.PHONY: gw-api-crds
gw-api-crds: ## Install the Gateway API CRDs. HACK: Use SSA to avoid the issue with the CRD annotations being too long.
ifeq ($(shell echo $(CONFORMANCE_VERSION) | grep -q '^v[0-9]' && echo yes),yes)
	kubectl apply --server-side -f "https://github.com/kubernetes-sigs/gateway-api/releases/download/$(CONFORMANCE_VERSION)/$(CONFORMANCE_CHANNEL)-install.yaml"
else
ifeq ($(CONFORMANCE_CHANNEL), standard)
	kubectl apply --server-side --kustomize "https://github.com/kubernetes-sigs/gateway-api/config/crd?ref=$(CONFORMANCE_VERSION)"
else
	kubectl apply --server-side --kustomize "https://github.com/kubernetes-sigs/gateway-api/config/crd/$(CONFORMANCE_CHANNEL)?ref=$(CONFORMANCE_VERSION)"
endif
endif

# The version of the k8s gateway api inference extension CRDs to install.
# Managed by `make bump-gie`.
GIE_CRD_VERSION ?= v1.3.0
# When we are back on a release version instead of `main`, we can do this
#GIE_CRD_VERSION ?= $(shell go list -m -f '{{.Version}}' sigs.k8s.io/gateway-api-inference-extension)

.PHONY: gie-crds
gie-crds: ## Install the Gateway API Inference Extension CRDs
ifeq ($(shell echo $(GIE_CRD_VERSION) | grep -q '^v[0-9]' && echo yes),yes)
	kubectl apply -f "https://github.com/kubernetes-sigs/gateway-api-inference-extension/releases/download/$(GIE_CRD_VERSION)/manifests.yaml"
else
	kubectl apply --kustomize "https://github.com/kubernetes-sigs/gateway-api-inference-extension/config/crd?ref=$(GIE_CRD_VERSION)"
endif

.PHONY: kind-metallb
metallb: ## Install the MetalLB load balancer
	./hack/kind/setup-metalllb-on-kind.sh

.PHONY: deploy-agentgateway
deploy-agentgateway: package-agentgateway-charts deploy-agentgateway-crd-chart deploy-agentgateway-chart ## Deploy the agentgateway chart and CRDs

.PHONY: setup-base
setup-base: kind-create gw-api-crds gie-crds metallb ## Setup the base infrastructure (kind cluster, CRDs, and MetalLB)

# Creates a functional kind cluster, builds and loads all images, and packages charts
# Does NOT deploy anything to the cluster
.PHONY: setup
setup: setup-base kind-build-and-load package-agentgateway-charts dummy-idp-docker kind-load-dummy-idp  ## Setup the complete infrastructure (base setup plus images and charts)

.PHONY: run
run: setup deploy-agentgateway ## Set up complete development environment

#----------------------------------------------------------------------------------
# Build assets for kubernetes e2e tests
#----------------------------------------------------------------------------------

kind-setup: ## Set up the KinD cluster. Deprecated: use kind-create instead.
	VERSION=${VERSION} CLUSTER_NAME=${CLUSTER_NAME} ./hack/kind/setup-kind.sh

kind-load-%:
	kind load docker-image $(IMAGE_REGISTRY)/$*:$(VERSION) --name $(CLUSTER_NAME)

# Build an image and load it into the KinD cluster
# Depends on: IMAGE_REGISTRY, VERSION, CLUSTER_NAME
kind-build-and-load-%: %-docker kind-load-% ; ## Use to build specified image and load it into kind

# Update the docker image used by a deployment
# This works for most of our deployments because the deployment name and container name both match
# NOTE TO DEVS:
#	I explored using a special format of the wildcard to pass deployment:image,
# 	but ran into some challenges with that pattern, while calling this target from another one.
#	It could be a cool extension to support, but didn't feel pressing so I stopped
kind-set-image-%:
	kubectl rollout pause deployment $* -n $(INSTALL_NAMESPACE) || true
	kubectl set image deployment/$* $*=$(IMAGE_REGISTRY)/$*:$(VERSION) -n $(INSTALL_NAMESPACE)
	kubectl patch deployment $* -n $(INSTALL_NAMESPACE) -p '{"spec": {"template":{"metadata":{"annotations":{"agentgateway-kind-last-update":"$(shell date)"}}}} }'
	kubectl rollout resume deployment $* -n $(INSTALL_NAMESPACE)

# Reload an image in KinD
# This is useful to developers when changing a single component
# You can reload an image, which means it will be rebuilt and reloaded into the kind cluster, and the deployment
# will be updated to reference it
# Depends on: IMAGE_REGISTRY, VERSION, INSTALL_NAMESPACE , CLUSTER_NAME
kind-reload-%: kind-build-and-load-% kind-set-image-% ; ## Use to build specified image, load it into kind, and restart its deployment

.PHONY: kind-build-and-load ## Use to build all images and load them into kind
kind-build-and-load: kind-build-and-load-agentgateway-controller
kind-build-and-load: kind-build-and-load-dummy-idp

.PHONY: kind-load ## Use to load all images into kind
kind-load: kind-load-agentgateway-controller
kind-load: kind-load-dummy-idp

#----------------------------------------------------------------------------------
# Targets for running Agent Gateway conformance tests
#----------------------------------------------------------------------------------

# Agent Gateway conformance test configuration
AGW_CONFORMANCE_REPORT_ARGS ?= -report-output=$(TEST_ASSET_DIR)/conformance/agw-$(VERSION)-report.yaml -organization=agentgateway -project=agentgateway -version=$(VERSION) -url=github.com/agentgateway/agentgateway -contact=github.com/agentgateway/agentgateway/issues/new/choose
AGW_CONFORMANCE_ARGS := -gateway-class=agentgateway $(AGW_CONFORMANCE_REPORT_ARGS)
.PHONY: agw-conformance ## Run the agent gateway conformance test suite
agw-conformance:
	@mkdir -p $(TEST_ASSET_DIR)/conformance
	CGO_ENABLED=0 go test -mod=mod -ldflags='$(LDFLAGS)' -tags conformance -test.v ./test/conformance/... -args $(AGW_CONFORMANCE_ARGS)

#----------------------------------------------------------------------------------
# Targets for running Gateway API Inference Extension conformance tests
#----------------------------------------------------------------------------------

# Reporting flags, identical to CONFORMANCE_REPORT_ARGS but with "inference-"
GIE_CONFORMANCE_REPORT_ARGS ?= \
    -report-output=$(TEST_ASSET_DIR)/conformance/inference-$(VERSION)-report.yaml \
    -organization=agentgateway \
    -project=agentgateway \
    -version=$(VERSION) \
    -url=github.com/agentgateway/agentgateway \
    -contact=github.com/agentgateway/agentgateway/issues/new/choose

# The args to pass into the Gateway API Inference Extension conformance test suite.
GIE_CONFORMANCE_ARGS := \
    -gateway-class=agentgateway \
    $(GIE_CONFORMANCE_REPORT_ARGS)

.PHONY: gie-conformance
gie-conformance: gie-crds ## Run the Gateway API Inference Extension conformance suite
	@mkdir -p $(TEST_ASSET_DIR)/conformance
	CGO_ENABLED=0 go test -mod=mod -ldflags='$(LDFLAGS)' \
	    -tags conformance \
	    -timeout=25m \
	    -v $(shell go mod download && go list -m -f '{{.Dir}}' sigs.k8s.io/gateway-api-inference-extension/conformance) \
	    -args $(GIE_CONFORMANCE_ARGS)

# An alias to run both Gateway API and Inference Extension conformance tests.
.PHONY: all-conformance
all-conformance: gie-conformance agw-conformance ## Run all conformance test suites
	@echo "All conformance suites have completed."

#----------------------------------------------------------------------------------
# Dependency Bumping
#----------------------------------------------------------------------------------

.PHONY: bump-gtw
bump-gtw: ## Bump Gateway API deps to $DEP_REF (or $DEP_VERSION). Example: make bump-gtw DEP_REF=198e6cab...
	@if [ -z "$${DEP_REF:-}" ] && [ -n "$${DEP_VERSION:-}" ]; then DEP_REF="$$DEP_VERSION"; fi; \
	if [ -z "$${DEP_REF:-}" ]; then \
	  echo "DEP_REF is not set (or DEP_VERSION). e.g. make bump-gtw DEP_REF=v1.3.0 or DEP_REF=198e6cab6774..."; \
	  exit 2; \
	fi; \
	echo "Bumping Gateway API to $${DEP_REF}"; \
	hack/bump_deps.sh gtw "$$DEP_REF"; \
	echo "Updating licensing..."; \
	$(MAKE) generate-licenses

.PHONY: bump-gie
bump-gie: ## Bump Gateway API Inference Extension to $DEP_REF (or $DEP_VERSION). Example: make bump-gie DEP_REF=198e6cab...
	@if [ -z "$${DEP_REF:-}" ] && [ -n "$${DEP_VERSION:-}" ]; then DEP_REF="$$DEP_VERSION"; fi; \
	if [ -z "$${DEP_REF:-}" ]; then \
	  echo "DEP_REF is not set (or DEP_VERSION). e.g. make bump-gie DEP_REF=v0.5.1 or DEP_REF=198e6cab6774..."; \
	  exit 2; \
	fi; \
	echo ">>> Bumping Gateway API Inference Extension to $${DEP_REF}"; \
	hack/bump_deps.sh gie "$$DEP_REF"; \
	echo "Updating licensing..."; \
	$(MAKE) generate-licenses

#----------------------------------------------------------------------------------
# Printing makefile variables utility
#----------------------------------------------------------------------------------

# use `make print-MAKEFILE_VAR` to print the value of MAKEFILE_VAR

print-%  : ; @echo $($*)
