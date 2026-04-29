.PHONY: ayni ayni-sandbox-analyze ayni-sandbox-install tag tag-major tag-minor tag-patch

ayni:
	@cargo run -p ayni-cli -- analyze --config ./.ayni.toml

ayni-sandbox-analyze:
	@cargo run -p ayni-cli -- analyze --config fixtures/ayni-sandbox/.ayni.toml

ayni-sandbox-install:
	@cargo run -p ayni-cli -- install --repo-root fixtures/ayni-sandbox

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
