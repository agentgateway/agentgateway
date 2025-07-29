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

.PHONY: docker-ext
docker-ext:
	$(DOCKER_BUILDER) build $(DOCKER_BUILD_ARGS) -t $(IMAGE_FULL_NAME)-ext -f Dockerfile.ext .

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
	go install github.com/golang/protobuf/protoc-gen-go


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


CONFIG_FILES := $(wildcard examples/*/config.yaml)

.PHONY: validate
validate: $(CONFIG_FILES)

.PHONY: $(CONFIG_FILES)
$(CONFIG_FILES):
	@cargo run -- -f $@ --validate-only