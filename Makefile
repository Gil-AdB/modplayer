# Modplayer Build System
#
# Targets:
#   make build     - Build native binary (release)
#   make lib       - Build C static library for Revival integration
#   make wasm      - Build WASM package via wasm-pack
#   make wasm-dev  - Build WASM + start webpack dev server
#   make test      - Run workspace tests
#   make clean     - Clean all build artifacts
#   make all       - Build everything (native + lib + wasm)

# Deployment
PUBLISH_REPO = https://github.com/Gil-AdB/rust-modplayer
PUBLISH_DIR = .publish

.PHONY: build lib wasm wasm-dev test clean all check publish

# Default target
all: build lib wasm

# Build native binary
build:
	cargo build --release -p modplayer-bin

# Build C static library (used by Revival project)
lib:
	cargo build --release -p modplayer-lib
	@echo ""
	@echo "Static library built: target/release/libmodplayer.a"

# Build WASM package
wasm:
	wasm-pack build modplayer-wasm --target bundler --release
	@echo ""
	@echo "WASM package built in modplayer-wasm/pkg/"

# Build WASM and start dev server with debug symbols
wasm-dev:
	wasm-pack build modplayer-wasm --target bundler --dev
	cd modplayer-wasm/www && npm install && npm start

# Run tests
test:
	cargo test --workspace

# Alias for test
check: test

# Clean all build artifacts
clean:
	cargo clean
	rm -rf modplayer-wasm/pkg
	rm -rf modplayer-wasm/www/dist
	rm -rf modplayer-wasm/www/node_modules
	rm -rf $(PUBLISH_DIR)

# Publish to the production repo
publish: wasm
	@if [ ! -d "$(PUBLISH_DIR)" ]; then \
		echo "Cloning deployment repository..."; \
		git clone $(PUBLISH_REPO) $(PUBLISH_DIR); \
	else \
		echo "Updating deployment repository..."; \
		cd $(PUBLISH_DIR) && git pull; \
	fi
	@echo "Building production bundles..."
	cd modplayer-wasm/www && npm run build
	@echo "Syncing files to $(PUBLISH_DIR)..."
	find $(PUBLISH_DIR) -mindepth 1 -maxdepth 1 -not -name ".git" -not -name "README.md" -not -name "LICENSE" -exec rm -rf {} +
	cp -r modplayer-wasm/www/dist/* $(PUBLISH_DIR)/
	cd $(PUBLISH_DIR) && git add -A
	@echo "----------------------------------------------------------"
	@echo "SUCCESS: Build is staged in $(PUBLISH_DIR)"
	@echo "Run 'cd $(PUBLISH_DIR) && git commit -m \"Update\" && git push' to deploy."
	@echo "----------------------------------------------------------"
