# Known issues

Tracked bugs and gaps not yet fixed. Stage 3 adds Plus2 PDM mic (`/dev/mic`); speaker/`/dev/spk` deferred (weak buzzer only on Plus2).

---

## `/dev/buttons/event` does not stream presses

**Status:** Open (deferred)  
**Board:** M5StickC Plus2 (GPIO37 = A, GPIO39 = B, active-low)

### Symptoms

- `cat dev/buttons/a` and `cat dev/buttons/b` reflect level correctly (`1` = pressed, `0` = released).
- `cat dev/buttons/event` blocks (Ctrl+C works) but **no lines appear** when A/B are pressed and released.
- `echo flush > dev/buttons/ctl` or `> dev/buttons/event` should clear the queue; ctl write was failing with I/O error on some builds (mode / firmware mismatch); event write path added as alternate.
- Earlier builds: stale queued lines drained without touching the button; then read hung (fixed: removed broken `pending` partial-read state). Directory `ls dev/buttons` panicked on 128-byte copy buffer (fixed).

### Expected behavior (DESIGN.md)

- Newline-delimited edge stream: `a down\n`, `a up\n`, `b down\n`, `b up\n`.
- Each `Tread` returns one line; blocking read until an edge is queued.

### Implementation notes

| Layer | Location |
|-------|----------|
| GPIO poll + edge detect | `firmware/src/dev/plus2.rs` — `buttons_task`, 20 ms period |
| Queue + `try_read_event` | `devices/src/buttons.rs` |
| 9P tree | `ninep/src/fs.rs` — `event`, `ctl` (`flush`) |
| Blocking read without wedging mount | `ninep/src/server.rs` — deferred `WaitEvent`, poll + interleaved packets |

Edges are pushed only on **change** (`a_down != prev_a`). Level files update every poll via `set_a` / `set_b`.

### Things to verify when revisiting

1. **Edges actually queued** — serial log in `push_event` or queue depth after press/release while `cat event` is blocked.
2. **`buttons_task` running** — if `a`/`b` levels update, task is alive; confirm `push_event` is reached on transitions.
3. **9P read path** — `handle_read` on `DevBtnEvent` uses `buttons::try_read_event`; deferred wait loop in `Session::run` should deliver Rread when queue non-empty.
4. **Client offset** — reads must use offset `0` per event line; non-zero offset returns 0 (EOF). Confirm v9fs `cat` first read offset is 0.
5. **Per-fid queues** — DESIGN calls for per-open-fid event streams; current design is one global queue (should still deliver events to one reader).
6. **Debounce** — 20 ms sample may miss very fast taps; unlikely to explain zero events on normal press/release.

### Workarounds

- Poll levels: `cat dev/buttons/a` / `b` in a shell loop.
- Revisit after Stage 2 polish or when adding StickS3 button GPIOs (different pins).

---

## `/dev/display/brightness` — no usable dimming on Plus2

**Status:** Open (deferred until StickS3 hardware available)  
**Board:** M5StickC Plus2 only tested so far

### Symptoms

- `echo <N> > dev/display/brightness` (0–255) accepts writes and `cat dev/display/brightness` reflects the value.
- **Perceived backlight is effectively on/off** — no smooth dimming across the range; only a narrow band near full brightness appears to change anything.
- May be hardware-limited on this panel/backlight driver (GPIO27 + LEDC), not only a software curve issue.

### Implementation notes

| Layer | Location |
|-------|----------|
| LEDC (Timer1 / Ch1, GPIO27, 40 kHz, 10-bit duty) | `firmware/src/dev/plus2.rs` — `apply_backlight`, `brightness_to_duty_pct` |
| Stored level 0–255 | `devices/src/display.rs` — `handle_brightness` |
| 9P node | `dev/display/brightness` |

Several duty mappings were tried (linear, gamma, remap 1–255 → raw duty ~860–1023); behavior stayed ~binary.

### Plan

- **Do not spend more time on Plus2 PWM** for now.
- Re-test brightness when **M5Stick S3** hardware is available — S3 may use **M5PM1** for backlight (`DESIGN.md`: `dev/display/backlight` / L3B rail) rather than the Plus2 GPIO27 LEDC path; dimming might work there or need a different driver altogether.
- If S3 also fails, document as a product limitation; if S3 works, keep Plus2-specific mapping or expose dim/full only on Plus2 `info`.

---

## `/dev/mic/pcm` — no capture on Plus2 (PDM)

**Status:** Open (deferred)  
**Board:** M5StickC Plus2 (SPM1423 PDM — GPIO0 CLK/WS, GPIO34 DATA)

### Symptoms

- `echo start > dev/mic/ctl` succeeds; `cat dev/mic/ctl` shows `running=1 rate=44100 …`.
- **`queued=` stays 0** while running (no PCM ever reaches the ring buffer).
- `dd if=dev/mic/pcm …` blocks indefinitely; Ctrl+C leaves `running=1`, file empty.
- Earlier builds: stack overflow on 9P connect (fixed — static `SessionStorage`); slice panic on `bs=8192` (fixed — cap `msize` to 4096).

Example:

```text
echo start > /mnt/stick/dev/mic/ctl
cat /mnt/stick/dev/mic/ctl
# running=1 rate=44100 queued=0 fmt=s16le
dd if=/mnt/stick/dev/mic/pcm of=clip.raw bs=4096 count=50
# hangs; queued= still 0
```

### Expected behavior (DESIGN.md / README)

- After `start`, I2S/PDM DMA fills the ring; `queued` increases; blocking reads on `pcm` return s16le mono @ 44100 Hz.

### Implementation notes

| Layer | Location |
|-------|----------|
| PDM + I2S DMA task | `firmware/src/dev/mic.rs` — WS on GPIO0 (TX), DIN on GPIO34 (RX), `pdm_conf` register poke, async circular DMA |
| Ring + ctl | `devices/src/mic.rs` |
| 9P blocking read | `ninep/src/server.rs` — `WaitStream` / `MicPcm` while `running` and queue empty |

`/dev/spk` omitted on Plus2 (buzzer only). StickS3 ES8311 mic path deferred.

### Things to verify when revisiting

1. **DMA actually produces bytes** — log `got` from `rx_xfer.pop` in `mic_task` (may be 0 forever today).
2. **PDM + esp-hal** — ESP32 has no first-class PDM API in esp-hal 1.1; compare M5 Arduino example / ESP-IDF `i2s_pdm` pin and clock setup.
3. **Clock path** — M5 uses GPIO0 as I2S WS; tried `with_mclk`/`CLK_OUT1` (wrong) then TX `with_ws` + silence DMA; still `queued=0`.
4. **Mic vs buzzer** — M5 docs: mic and speaker amp should not run together; avoid boot chime during capture tests.
5. **Sample rate** — hardware fixed at 44100 Hz on ESP32; `ctl rate` is stored intent only.

### Workarounds

- None on device today. Stage 3 mic remains **tree + ctl API only** until PDM capture works.

---
