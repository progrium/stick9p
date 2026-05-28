#pragma once

#include <stddef.h>
#include <stdint.h>
#include <stdarg.h>

void stick_wamr_capture_reset(void);
const char *stick_wamr_capture_ptr(void);
size_t stick_wamr_capture_len(void);
int stick_wamr_vprintf(const char *format, va_list ap);
void stick_wamr_set_runtime_heap(void *heap_buf, uint32_t heap_size);
int stick_wamr_init_runtime(void);
int stick_wamr_run(const uint8_t *wasm_bytes, uint32_t wasm_len, char *argv[],
                   uint32_t argc, const char *env[], uint32_t env_count,
                   char *err, uint32_t err_len);
