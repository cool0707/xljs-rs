VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
CARGO ?= cargo
export XLJS_VERSION := $(VERSION)

.PHONY: all test check-windows windows windows-gnu dist linux clean

all: test

test:
	$(CARGO) test --lib

# Type-check the Windows-only code from any OS.
check-windows:
	$(CARGO) check --target x86_64-pc-windows-gnu --all-targets

# On Windows (MSVC toolchain): dist/<arch>/xljs.exe (GUI) and xljsc.exe
# (console) for x64 and arm64.
windows:
	$(CARGO) build --release --target x86_64-pc-windows-msvc
	$(CARGO) build --release --target x86_64-pc-windows-msvc --features console --target-dir target/console
	$(CARGO) build --release --target aarch64-pc-windows-msvc
	$(CARGO) build --release --target aarch64-pc-windows-msvc --features console --target-dir target/console
	mkdir -p dist/x64 dist/arm64
	cp target/x86_64-pc-windows-msvc/release/xljs.exe dist/x64/xljs.exe
	cp target/console/x86_64-pc-windows-msvc/release/xljs.exe dist/x64/xljsc.exe
	cp target/aarch64-pc-windows-msvc/release/xljs.exe dist/arm64/xljs.exe
	cp target/console/aarch64-pc-windows-msvc/release/xljs.exe dist/arm64/xljsc.exe

# Cross build of the x64 exes from Linux with MinGW-w64 (apt install mingw-w64).
windows-gnu:
	$(CARGO) build --release --target x86_64-pc-windows-gnu
	$(CARGO) build --release --target x86_64-pc-windows-gnu --features console --target-dir target/console
	mkdir -p dist/x64
	cp target/x86_64-pc-windows-gnu/release/xljs.exe dist/x64/xljs.exe
	cp target/console/x86_64-pc-windows-gnu/release/xljs.exe dist/x64/xljsc.exe

# Release archives: dist/xljs-windows-x64.zip, dist/xljs-windows-arm64.zip
# (the executables and docs/ai). Run after `make windows`.
dist:
	cd dist && rm -f xljs-windows-*.zip \
	  && zip -j xljs-windows-x64.zip x64/xljs.exe x64/xljsc.exe \
	  && zip -j xljs-windows-arm64.zip arm64/xljs.exe arm64/xljsc.exe
	rm -rf dist/docs && mkdir -p dist/docs && cp -r docs/ai dist/docs/
	cd dist && zip -r xljs-windows-x64.zip docs && zip -r xljs-windows-arm64.zip docs

linux:
	$(CARGO) build --release
	mkdir -p dist && cp target/release/xljs dist/xljs

clean:
	rm -rf dist
	$(CARGO) clean
