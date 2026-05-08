SHELL := /bin/bash

.PHONY: ayni ayni-sandbox-analyze ayni-sandbox-install \
	docker-cli docker-build docker-install docker-analyze docker-example docker-examples \
	docker-build-rust docker-build-go docker-build-node docker-build-python \
	docker-install-rust-single docker-install-go-single docker-install-node-single docker-install-python-single \
	docker-analyze-rust-mono docker-analyze-go-mono docker-analyze-node-mono docker-analyze-python-mono \
	docker-example-rust docker-example-go docker-example-node docker-example-python \
	tag tag-major tag-minor tag-patch

LANG ?= go
FIXTURE ?= single
APPLY ?= true
DOCKER_IMAGE_PREFIX ?= ayni-example
DOCKER_CLI_IMAGE ?= ayni-cli-builder
DOCKER_OUT ?= .ayni-docker
DOCKER_CLI = target/docker/ayni
DOCKER_IMAGE = $(DOCKER_IMAGE_PREFIX)-$(LANG)
DOCKERFILE = examples/$(LANG)/Dockerfile
FIXTURE_PATH = examples/$(LANG)/$(FIXTURE)
OUT_DIR = $(DOCKER_OUT)/$(LANG)-$(FIXTURE)
WORK_DIR = $(OUT_DIR)/work
DOCKER_USER = $(shell id -u):$(shell id -g)
DOCKER_ENV = -e HOME=/tmp/ayni-home \
	-e GOPATH=/tmp/ayni-go \
	-e UV_TOOL_DIR=/tmp/ayni-uv-tools \
	-e UV_TOOL_BIN_DIR=/tmp/ayni-bin
DOCKER_RUN = docker run --rm \
	--user $(DOCKER_USER) \
	$(DOCKER_ENV) \
	-v "$(CURDIR):/repo" \
	-w /repo \
	$(DOCKER_IMAGE)

ayni:
	@cargo run -p ayni-cli -- analyze --config ./.ayni.toml

ayni-sandbox-analyze:
	@cargo run -p ayni-cli -- analyze --config fixtures/ayni-sandbox/.ayni.toml

ayni-sandbox-install:
	@cargo run -p ayni-cli -- install --repo-root fixtures/ayni-sandbox

docker-cli:
	@mkdir -p target/docker
	@docker build -f Dockerfile.cli -t $(DOCKER_CLI_IMAGE) .
	@docker run --rm \
		--user $(DOCKER_USER) \
		-e HOME=/tmp/ayni-home \
		-e CARGO_HOME=/tmp/ayni-cargo \
		-v "$(CURDIR):/repo" \
		-w /repo \
		$(DOCKER_CLI_IMAGE) \
		bash -lc 'cargo build -p ayni-cli && cp target/debug/ayni $(DOCKER_CLI)'

docker-build:
	@docker build -f $(DOCKERFILE) -t $(DOCKER_IMAGE) .

docker-install: docker-cli
	@mkdir -p $(OUT_DIR)
	@$(DOCKER_RUN) bash -lc 'set -euo pipefail; \
		work=$(WORK_DIR); \
		rm -rf "$$work"; \
		mkdir -p "$$(dirname "$$work")"; \
		cp -a $(FIXTURE_PATH) "$$work"; \
		if [ "$(FIXTURE)" = "single" ]; then \
			rm -rf "$$work/.ayni" "$$work/.ayni.toml" "$$work/.gitignore" "$$work/AGENTS.md"; \
		fi; \
		mkdir -p $(OUT_DIR); \
		args=(install --repo-root "$$work" --language $(LANG)); \
		if [ "$(APPLY)" = "true" ]; then args+=(--apply); fi; \
		/repo/$(DOCKER_CLI) "$${args[@]}" 2>&1 | tee $(OUT_DIR)/install.log; \
		for generated in .ayni.toml .gitignore AGENTS.md; do \
			if [ -f "$$work/$$generated" ]; then cp "$$work/$$generated" "$(OUT_DIR)/$${generated#.}"; fi; \
		done; \
		rm -rf "$$work"'

docker-analyze: docker-cli
	@mkdir -p $(OUT_DIR)
	@$(DOCKER_RUN) bash -lc 'set -euo pipefail; \
		work=$(WORK_DIR); \
		rm -rf "$$work"; \
		mkdir -p "$$(dirname "$$work")"; \
		cp -a $(FIXTURE_PATH) "$$work"; \
		mkdir -p $(OUT_DIR); \
		/repo/$(DOCKER_CLI) install --repo-root "$$work" --language $(LANG) --apply 2>&1 | tee $(OUT_DIR)/install.log; \
		/repo/$(DOCKER_CLI) analyze --config "$$work/.ayni.toml" --language $(LANG) 2>&1 | tee $(OUT_DIR)/analyze.log; \
		if [ -f "$$work/.ayni/last/signals.json" ]; then cp "$$work/.ayni/last/signals.json" $(OUT_DIR)/signals.json; fi; \
		rm -rf "$$work"'

docker-example: docker-build
	@$(MAKE) docker-install LANG=$(LANG) FIXTURE=single APPLY=true
	@$(MAKE) docker-analyze LANG=$(LANG) FIXTURE=mono

docker-examples:
	@set -euo pipefail; \
	for lang in rust go node python; do \
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

docker-install-rust-single:
	@$(MAKE) docker-build LANG=rust
	@$(MAKE) docker-install LANG=rust FIXTURE=single APPLY=true

docker-install-go-single:
	@$(MAKE) docker-build LANG=go
	@$(MAKE) docker-install LANG=go FIXTURE=single APPLY=true

docker-install-node-single:
	@$(MAKE) docker-build LANG=node
	@$(MAKE) docker-install LANG=node FIXTURE=single APPLY=true

docker-install-python-single:
	@$(MAKE) docker-build LANG=python
	@$(MAKE) docker-install LANG=python FIXTURE=single APPLY=true

docker-analyze-rust-mono:
	@$(MAKE) docker-build LANG=rust
	@$(MAKE) docker-analyze LANG=rust FIXTURE=mono

docker-analyze-go-mono:
	@$(MAKE) docker-build LANG=go
	@$(MAKE) docker-analyze LANG=go FIXTURE=mono

docker-analyze-node-mono:
	@$(MAKE) docker-build LANG=node
	@$(MAKE) docker-analyze LANG=node FIXTURE=mono

docker-analyze-python-mono:
	@$(MAKE) docker-build LANG=python
	@$(MAKE) docker-analyze LANG=python FIXTURE=mono

docker-example-rust:
	@$(MAKE) docker-build LANG=rust
	@$(MAKE) docker-install LANG=rust FIXTURE=single APPLY=true
	@$(MAKE) docker-analyze LANG=rust FIXTURE=mono

docker-example-go:
	@$(MAKE) docker-build LANG=go
	@$(MAKE) docker-install LANG=go FIXTURE=single APPLY=true
	@$(MAKE) docker-analyze LANG=go FIXTURE=mono

docker-example-node:
	@$(MAKE) docker-build LANG=node
	@$(MAKE) docker-install LANG=node FIXTURE=single APPLY=true
	@$(MAKE) docker-analyze LANG=node FIXTURE=mono

docker-example-python:
	@$(MAKE) docker-build LANG=python
	@$(MAKE) docker-install LANG=python FIXTURE=single APPLY=true
	@$(MAKE) docker-analyze LANG=python FIXTURE=mono

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
