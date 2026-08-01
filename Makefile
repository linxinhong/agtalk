# agtalk 构建入口（统一 custom-protocol，防 GUI 白屏——AGENTS.md §5.5）

.PHONY: build debug release deploy check test fmt clippy

# debug 构建（开发/交付统一入口，GUI 内嵌 dist）
build debug:
	./scripts/build.sh

# release 构建（带 custom-protocol，GUI 内嵌 dist）
release:
	./scripts/build.sh --release

# 部署（release + 安装到 ~/.local/bin）
deploy:
	./scripts/build.sh --release
	install -m 0755 target/release/agtalk ~/.local/bin/agtalk

check:
	cargo check -p agtalk

test:
	cargo test -p agtalk -- --test-threads=1

fmt:
	cargo fmt --check

clippy:
	cargo clippy -p agtalk -- -D warnings
