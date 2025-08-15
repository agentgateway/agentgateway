# Image configuration
DOCKER_REGISTRY ?= ghcr.io
DOCKER_REPO ?= agentgateway
IMAGE_NAME ?= agentgateway
VERSION ?= $(shell git describe --tags --always --dirty)
IMAGE_TAG ?= $(VERSION)
IMAGE_FULL_NAME ?= $(DOCKER_REGISTRY)/$(DOCKER_REPO)/$(IMAGE_NAME):$(IMAGE_TAG)
DOCKER_BUILDER ?= docker
DOCKER_BUILD_ARGS ?=
KIND_CLUSTER_NAME ?= agentgateway

# docker
.PHONY: docker
docker:
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) -t $(IMAGE_FULL_NAME) . --progress=plain

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
	cargo build --release --features ui $(CARGO_BUILD_ARGS) --target $(TARGET)

# lint
.PHONY: lint
lint:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

fix-lint:
	cargo clippy --fix --allow-staged --allow-dirty --workspace
	cargo fmt

# test
.PHONY: test
test:
	cargo test --all-targets

# clean
.PHONY: clean
clean:
	cargo clean

objects := $(wildcard examples/*/config.json)

.PHONY: check-clean-repo
check-clean-repo:
ifeq ($(OS),Windows_NT)
	@powershell -ExecutionPolicy Bypass -Command common/scripts/check_clean_repo.ps1
else
	@common/scripts/check_clean_repo.sh
endif


.PHONY: gen
gen: generate-apis generate-schema fix-lint
	@:

.PHONY: generate-schema
generate-schema:
	@cargo xtask schema

# Code generation for xds apis
.PHONY: generate-apis
generate-apis:
	@PATH=./common/tools:$(PATH) buf generate --path crates/agentgateway/proto/resource.proto --path crates/agentgateway/proto/workload.proto

.PHONY: run-validation-deps
run-validation-deps:
ifeq ($(OS),Windows_NT)
	@powershell -ExecutionPolicy Bypass -Command common/scripts/manage-validation-deps.ps1 start
else
	@common/scripts/manage-validation-deps.sh start
endif

.PHONY: stop-validation-deps
stop-validation-deps:
ifeq ($(OS),Windows_NT)
	@powershell -ExecutionPolicy Bypass -Command common/scripts/manage-validation-deps.ps1 stop
else
	@common/scripts/manage-validation-deps.sh stop
endif

CONFIG_FILES := $(wildcard examples/*/config.yaml)

.PHONY: validate
validate: run-validation-deps $(CONFIG_FILES) stop-validation-deps

.PHONY: $(CONFIG_FILES)
$(CONFIG_FILES):
	@cargo run -- -f $@ --validate-only
