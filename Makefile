.PHONY: frontend release deploy

frontend:
	pnpm build

release: frontend
	cargo build --release --features custom-protocol

deploy: release
	mkdir -p ~/.local/bin
	cp target/release/agtalk ~/.local/bin/agtalk
	@if [ "$$(uname -s)" = "Darwin" ]; then \
		codesign --sign - --force ~/.local/bin/agtalk; \
	fi
	@echo "Deployed: ~/.local/bin/agtalk"
