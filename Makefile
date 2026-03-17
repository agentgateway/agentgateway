.DEFAULT_GOAL := help
SHELL := /bin/bash

ROOT_DIR := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
CONTROLLER_MAKE := $(MAKE) --no-print-directory -C controller
UI_NPM := npm --prefix ui
DEV_ENV := ./tools/dev-env.sh

export PATH := $(ROOT_DIR)tools:$(PATH)

# Image configuration
DOCKER_REGISTRY ?= ghcr.io
DOCKER_REPO ?= agentgateway
IMAGE_NAME ?= agentgateway
VERSION ?= $(shell git describe --tags --always --dirty)
GIT_REVISION ?= $(shell git rev-parse HEAD)
IMAGE_TAG ?= $(VERSION)
IMAGE_FULL_NAME ?= $(DOCKER_REGISTRY)/$(DOCKER_REPO)/$(IMAGE_NAME):$(IMAGE_TAG)
DOCKER_BUILDER ?= docker
DOCKER_BUILD_ARGS ?= --build-arg VERSION=$(VERSION) --build-arg GIT_REVISION=$(GIT_REVISION)

CARGO_BUILD_ARGS ?=
PROFILE ?= local
E2E_PROFILE ?= e2e
E2E_TEST_PATTERN ?= ^TestAgentgatewayIntegration
TILT ?= tilt
# Pass only explicit user overrides through to tools/dev-env.sh so profile defaults
# remain authoritative when the user does not set a value in make or the shell.
DEV_ENV_EXPORT_VARS := CLUSTER_CONTEXT CLUSTER_NAME CONTROLLER_IMAGE_REPOSITORY CONTROLLER_LOG_LEVEL ENABLE_INFERENCE_EXTENSION IMAGE_TAG INSTALL_NAMESPACE PROXY_BUILD_PROFILE PROXY_IMAGE_REPOSITORY REGISTRY_HOST REGISTRY_NAME REGISTRY_PORT TAG TEST_MODE
DEV_ENV_ENV := $(strip $(foreach var,$(DEV_ENV_EXPORT_VARS),$(if $(filter command line environment,$(origin $(var))),$(var)="$($(var))")))

.PHONY: help
help: NAME_COLUMN_WIDTH=24
help: LINE_COLUMN_WIDTH=5
help: ## Show the supported root-level developer commands.
	@grep -hnE '^[%a-zA-Z0-9_.-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = "[:]|(## )"}; {printf "\033[36mL%-$(LINE_COLUMN_WIDTH)s%-$(NAME_COLUMN_WIDTH)s\033[0m %s\n", $$1, $$2, $$4}'

.PHONY: bootstrap
bootstrap: ## Bootstrap repo-managed dependencies and UI packages.
	@./tools/bootstrap.sh

.PHONY: doctor
doctor: ## Verify the required local toolchain and optional Tilt support.
	@./tools/doctor.sh

# docker
.PHONY: docker
docker: ## Build the proxy container image for the current platform.
ifeq ($(OS),Windows_NT)
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) -f Dockerfile.windows -t $(IMAGE_FULL_NAME) .
else
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) -t $(IMAGE_FULL_NAME) . --progress=plain
endif

.PHONY: docker-ci
docker-ci: ## Build the CI proxy container image for the current platform.
ifeq ($(OS),Windows_NT)
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) --build-arg PROFILE=ci -f Dockerfile.windows -t $(IMAGE_FULL_NAME) .
else
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) --build-arg PROFILE=ci -t $(IMAGE_FULL_NAME) . --progress=plain
endif

.PHONY: docker-musl
docker-musl: ## Build the musl proxy container image.
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) -t $(IMAGE_FULL_NAME)-musl --build-arg=BUILDER=musl . --progress=plain

# build
.PHONY: build
build: ## Build the Rust proxy in release mode.
	cargo build --release --features ui $(CARGO_BUILD_ARGS)

.PHONY: build-target
build-target: ## Build the Rust proxy for TARGET/PROFILE overrides.
	cargo build --features ui $(CARGO_BUILD_ARGS) --target $(TARGET) --profile $(PROFILE)

# lint / fix
.PHONY: lint
lint: lint-proxy lint-controller lint-ui ## Run all check-only lint flows.

.PHONY: lint-proxy
lint-proxy: ## Run Rust formatting and clippy checks.
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

.PHONY: lint-controller
lint-controller: ## Run the controller lint flow.
	$(CONTROLLER_MAKE) analyze

.PHONY: lint-ui
lint-ui: ui-deps ## Run the UI lint flow.
	$(UI_NPM) run lint

.PHONY: fix
fix: fix-proxy fix-controller fix-ui ## Run all mutating formatting and auto-fix flows.

.PHONY: fix-proxy
fix-proxy: ## Run Rust auto-fixes and formatting.
	cargo clippy --fix --allow-staged --allow-dirty --allow-no-vcs
	cargo fmt

.PHONY: fix-controller
fix-controller: ## Format the controller Go code.
	$(CONTROLLER_MAKE) fmt

.PHONY: fix-ui
fix-ui: ui-deps ## Apply UI lint auto-fixes.
	$(UI_NPM) run fix

.PHONY: format
format: ## Format the Rust workspace.
	cargo fmt

# test
.PHONY: test
test: test-proxy test-controller test-ui ## Run the fast default repo verification suite.

.PHONY: test-proxy
test-proxy: ## Run the Rust proxy tests.
	cargo test --all-targets

.PHONY: test-controller
test-controller: ## Run the controller unit and integration tests that stay local.
	$(CONTROLLER_MAKE) unit

.PHONY: test-ui
test-ui: ui-deps ## Run the UI verification flow.
	$(UI_NPM) run verify

.PHONY: env-up
env-up: ## Create the shared local Kubernetes environment for PROFILE=$(PROFILE).
	@$(DEV_ENV_ENV) $(DEV_ENV) --profile $(PROFILE) up

.PHONY: env-down
env-down: ## Tear down the shared local Kubernetes environment for PROFILE=$(PROFILE).
	@$(DEV_ENV_ENV) $(DEV_ENV) --profile $(PROFILE) down

.PHONY: env-status
env-status: ## Show shared environment status for PROFILE=$(PROFILE).
	@$(DEV_ENV_ENV) $(DEV_ENV) --profile $(PROFILE) status

.PHONY: dev
dev: ## Start the preferred daily inner loop through Tilt.
	@$(DEV_ENV_ENV) $(TILT) up

.PHONY: e2e
e2e: ## Run the supported local end-to-end workflow against the shared env path.
	@$(DEV_ENV_ENV) $(DEV_ENV) --profile $(E2E_PROFILE) up
	PERSIST_INSTALL=true CGO_ENABLED=0 go test -tags=e2e -v ./controller/test/e2e/tests -run '$(E2E_TEST_PATTERN)'

# clean
.PHONY: clean
clean: ## Clean Rust build outputs.
	cargo clean

.PHONY: check-clean-repo
check-clean-repo: ## Fail when tracked generated files are dirty.
	@tools/check_clean_repo.sh

.PHONY: gen
gen: generate-apis generate-schema format ## Regenerate repo-owned generated artifacts.
	@:

.PHONY: generate-schema
generate-schema: ## Regenerate the config and CEL schema docs/artifacts.
	@cargo xtask schema

# Code generation for xds apis
.PHONY: generate-apis
generate-apis: ## Regenerate the xDS Go APIs.
	@PATH="./common/tools:$(PATH)" buf generate --path crates/agentgateway/proto/resource.proto

.PHONY: run-validation-deps
run-validation-deps: ## Start external services used for config validation.
	@tools/manage-validation-deps.sh start

.PHONY: stop-validation-deps
stop-validation-deps: ## Stop external services used for config validation.
	@tools/manage-validation-deps.sh stop

.PHONY: validate
validate: ## Validate checked-in example and schema config files.
	@tools/validate-configs.sh

.PHONY: ui-deps
ui-deps:
	@if [ ! -d ui/node_modules ]; then $(UI_NPM) ci; fi
