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

.PHONY: build lib wasm wasm-dev test clean all check

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

# Build WASM and start dev server
wasm-dev: wasm
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
