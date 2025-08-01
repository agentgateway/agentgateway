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

# lint
.PHONY: lint
lint:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

# test
.PHONY: test
test:
	cargo test --all-targets

# clean
.PHONY: clean
clean:
	cargo clean

objects := $(wildcard examples/*/config.json)

.PHONY: install-go-tools
install-go-tools:
	go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.36.6

.PHONY: check-clean-repo
check-clean-repo:
	@common/scripts/check_clean_repo.sh

.PHONY: gen
gen: generate-apis generate-schema
	@:

.PHONY: generate-schema
generate-schema:
	@cargo xtask schema

# Code generation for xds apis
.PHONY: generate-apis
generate-apis: install-go-tools
	@protoc --proto_path=./crates/agentgateway/proto/ \
		--go_out=./go/api \
		--go_opt=paths=source_relative \
		./crates/agentgateway/proto/resource.proto
	@protoc --proto_path=./crates/agentgateway/proto/ \
		--go_out=./go/api \
		--go_opt=paths=source_relative \
		./crates/agentgateway/proto/workload.proto

.PHONY: run-validate-deps
run-validate-deps:
	@common/scripts/manage-validation-deps.sh start

.PHONY: stop-validate-deps
stop-validate-deps:
	@common/scripts/manage-validation-deps.sh stop

CONFIG_FILES := $(wildcard examples/*/config.yaml)

.PHONY: validate
validate: run-validate-deps $(CONFIG_FILES) stop-validate-deps

.PHONY: $(CONFIG_FILES)
$(CONFIG_FILES):
	@cargo run -- -f $@ --validate-only