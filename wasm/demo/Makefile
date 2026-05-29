.PHONY: build clobber

# `-target=wasi` produces a WASI preview1 module with an exported `_start`,
# which is exactly what wamr-sys expects. `-opt=z` + `-no-debug` keep the
# artifact small enough to copy over 9p into /tmp without churning PSRAM.
build:
	tinygo build \
		-o demo.wasm \
		-target=wasi \
		-opt=z \
		-no-debug \
		.

clobber:
	rm -f demo.wasm
