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
export PATH := ./tools:$(PATH)

# docker
.PHONY: docker
docker:
ifeq ($(OS),Windows_NT)
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) -f Dockerfile.windows -t $(IMAGE_FULL_NAME) .
else
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) -t $(IMAGE_FULL_NAME) . --progress=plain
endif

.PHONY: docker-musl
docker-musl:
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) -t $(IMAGE_FULL_NAME)-musl --build-arg=BUILDER=musl . --progress=plain

CARGO_BUILD_ARGS ?=
# build
.PHONY: build
build:
	cargo build --release --features ui $(CARGO_BUILD_ARGS)
.PHONY: build-target
build-target:
	cargo build --features ui $(CARGO_BUILD_ARGS) --target $(TARGET) --profile $(PROFILE)

# lint
.PHONY: lint
LINT_CARGO_ARGS ?=
lint:
	cargo fmt --check
	cargo clippy --all-targets $(LINT_CARGO_ARGS) -- -D warnings

.PHONY: lint-local
LOCAL_PACKAGES = -p a2a-sdk -p agentgateway -p cel -p cel-derive -p agent-celx -p agent-core -p agent-hbone -p agent-xds
lint-local:
	cargo fmt --check
	cargo clippy --all-targets $(LOCAL_PACKAGES) -- -D warnings

.PHONY: fix-lint
fix-lint:
	cargo clippy --fix --allow-staged --allow-dirty --allow-no-vcs
	cargo fmt

.PHONY: format
format:
	cargo fmt

# test
.PHONY: test
TEST_RUST_MIN_STACK ?= 8388608
TEST_CARGO_ARGS ?=
test:
	RUST_MIN_STACK=$(TEST_RUST_MIN_STACK) cargo test --all-targets $(TEST_CARGO_ARGS)

.PHONY: test-local
test-local:
	RUST_MIN_STACK=$(TEST_RUST_MIN_STACK) cargo test --all-targets $(LOCAL_PACKAGES)

# clean
.PHONY: clean
clean:
	cargo clean

objects := $(wildcard examples/*/config.json)

.PHONY: check-clean-repo
check-clean-repo:
	@tools/check_clean_repo.sh

.PHONY: gen
gen: generate-apis generate-schema format
	@:

.PHONY: generate-schema
generate-schema:
	@cargo xtask schema

# Code generation for xds apis
.PHONY: generate-apis
generate-apis:
	@PATH="./common/tools:$(PATH)" buf generate --path crates/agentgateway/proto/resource.proto

.PHONY: run-validation-deps
run-validation-deps:
	@tools/manage-validation-deps.sh start

.PHONY: stop-validation-deps
stop-validation-deps:
	@tools/manage-validation-deps.sh stop

.PHONY: validate
validate:
	@tools/validate-configs.sh
