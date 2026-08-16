SHELL := /bin/bash

.PHONY: ayni \
	docker-build docker-check docker-example docker-examples \
	docker-build-rust docker-build-go docker-build-node docker-build-python docker-build-kotlin \
	docker-check-rust-mono docker-check-go-mono docker-check-node-mono docker-check-python-mono docker-check-kotlin-mono \
	docker-example-rust docker-example-go docker-example-node docker-example-python docker-example-kotlin \
	tag tag-major tag-minor tag-patch

LANG ?= go
FIXTURE ?= mono
DOCKER_IMAGE_PREFIX ?= ayni-example
DOCKER_IMAGE = $(DOCKER_IMAGE_PREFIX)-$(LANG)
DOCKERFILE = examples/$(LANG)/Dockerfile
FIXTURE_PATH = examples/$(LANG)/$(FIXTURE)
DOCKER_USER = $(shell id -u):$(shell id -g)
DOCKER_TMPFS_SIZE ?= 4g
DOCKER_ENV = -e HOME=/tmp/ayni-home \
	-e GOPATH=/tmp/ayni-go \
	-e UV_TOOL_DIR=/tmp/ayni-uv-tools \
	-e UV_TOOL_BIN_DIR=/tmp/ayni-bin
DOCKER_RUN = docker run --rm \
	--tmpfs /tmp:rw,exec,nosuid,size=$(DOCKER_TMPFS_SIZE) \
	--user $(DOCKER_USER) \
	$(DOCKER_ENV) \
	-v "$(CURDIR):/repo:ro" \
	-w /repo \
	$(DOCKER_IMAGE)

ayni:
	@cargo run -p ayni-cli -- check --host --config ./.ayni.toml

docker-build:
	@docker build -f $(DOCKERFILE) -t $(DOCKER_IMAGE) .

docker-check:
	@$(DOCKER_RUN) bash -c 'set -euo pipefail; \
		work=$$(mktemp -d -t ayni-$(LANG)-$(FIXTURE)-XXXXXX); \
		cp -a /repo/$(FIXTURE_PATH)/. "$$work"; \
		ayni check --host --config "$$work/.ayni.toml"; \
		rm -rf "$$work"'

docker-example: docker-build
	@$(MAKE) docker-check LANG=$(LANG) FIXTURE=mono

docker-examples:
	@set -euo pipefail; \
	for lang in rust go node python kotlin; do \
		$(MAKE) docker-example-$$lang; \
	done

docker-build-rust:
	@$(MAKE) docker-build LANG=rust

docker-build-go:
	@$(MAKE) docker-build LANG=go

docker-build-node:
	@$(MAKE) docker-build LANG=node

docker-build-python:
	@$(MAKE) docker-build LANG=python

docker-build-kotlin:
	@$(MAKE) docker-build LANG=kotlin

docker-check-rust-mono:
	@$(MAKE) docker-build LANG=rust
	@$(MAKE) docker-check LANG=rust FIXTURE=mono

docker-check-go-mono:
	@$(MAKE) docker-build LANG=go
	@$(MAKE) docker-check LANG=go FIXTURE=mono

docker-check-node-mono:
	@$(MAKE) docker-build LANG=node
	@$(MAKE) docker-check LANG=node FIXTURE=mono

docker-check-python-mono:
	@$(MAKE) docker-build LANG=python
	@$(MAKE) docker-check LANG=python FIXTURE=mono

docker-check-kotlin-mono:
	@$(MAKE) docker-build LANG=kotlin
	@$(MAKE) docker-check LANG=kotlin FIXTURE=mono

docker-example-rust:
	@$(MAKE) docker-build LANG=rust
	@$(MAKE) docker-check LANG=rust FIXTURE=mono

docker-example-go:
	@$(MAKE) docker-build LANG=go
	@$(MAKE) docker-check LANG=go FIXTURE=mono

docker-example-node:
	@$(MAKE) docker-build LANG=node
	@$(MAKE) docker-check LANG=node FIXTURE=mono

docker-example-python:
	@$(MAKE) docker-build LANG=python
	@$(MAKE) docker-check LANG=python FIXTURE=mono

docker-example-kotlin:
	@$(MAKE) docker-build LANG=kotlin
	@$(MAKE) docker-check LANG=kotlin FIXTURE=mono

# Semver tag helpers
# Usage:
#   make tag BUMP=major
#   make tag BUMP=minor
#   make tag BUMP=patch
# Optional:
#   make tag BUMP=patch PUSH=true
BUMP ?= patch
PUSH ?= false

tag:
	@set -euo pipefail; \
	if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then \
		echo "Not inside a git repository"; \
		exit 1; \
	fi; \
	if ! git diff --quiet || ! git diff --cached --quiet; then \
		echo "Working tree is dirty. Commit or stash changes before tagging."; \
		exit 1; \
	fi; \
	case "$(BUMP)" in \
		major|minor|patch) ;; \
		*) echo "Invalid BUMP='$(BUMP)'. Use major|minor|patch."; exit 1 ;; \
	esac; \
	latest_tag=$$(git tag --list 'v[0-9]*.[0-9]*.[0-9]*' --sort=-v:refname | head -n1); \
	if [ -z "$$latest_tag" ]; then \
		latest_tag="v0.0.0"; \
	fi; \
	version=$${latest_tag#v}; \
	IFS='.' read -r major minor patch <<< "$$version"; \
	case "$(BUMP)" in \
		major) major=$$((major + 1)); minor=0; patch=0 ;; \
		minor) minor=$$((minor + 1)); patch=0 ;; \
		patch) patch=$$((patch + 1)) ;; \
	esac; \
	new_tag="v$$major.$$minor.$$patch"; \
	echo "Latest tag: $$latest_tag"; \
	echo "New tag:    $$new_tag"; \
	git tag -a "$$new_tag" -m "Release $$new_tag"; \
	echo "Created tag $$new_tag"; \
	if [ "$(PUSH)" = "true" ]; then \
		git push origin "$$new_tag"; \
		echo "Pushed tag $$new_tag"; \
	else \
		echo "Tag created locally. Push with: git push origin $$new_tag"; \
	fi

tag-major:
	@$(MAKE) tag BUMP=major PUSH=$(PUSH)

tag-minor:
	@$(MAKE) tag BUMP=minor PUSH=$(PUSH)

tag-patch:
	@$(MAKE) tag BUMP=patch PUSH=$(PUSH)
