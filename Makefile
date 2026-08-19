SHELL := /bin/bash

.PHONY: ayni

ayni:
	@cargo run -p ayni-cli -- check --host --config ./.ayni.toml
