# assumes m5stick (s3 by default)
#
# Requires espup-installed toolchains:
#   ~/export-esp.sh        (sets PATH + LIBCLANG_PATH for xtensa-esp-elf gcc/clang)
# If you don't have it: `cargo install espup && espup install`.

SHELL := bash
ESP_ENV := source $$HOME/export-esp.sh

build:
	$(ESP_ENV) && cargo build -p firmware --no-default-features --features board-sticks3 --target xtensa-esp32s3-none-elf
.PHONY: build

flash:
	$(ESP_ENV) && cargo run -p firmware --no-default-features --features board-sticks3 --target xtensa-esp32s3-none-elf
.PHONY: flash
