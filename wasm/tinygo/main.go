// Sample WASI guest for the `/ctl exec` runner.
//
//	make build           -> gocheck.wasm
//	cp gocheck.wasm /tmp
//	echo 'exec /tmp/gocheck.wasm' > /ctl
//
// Output mirrors wasm/zig/main.zig so the two guests can be diffed quickly.
// stdout/stderr land in /tmp/exec.log on the device.
package main

import (
	"fmt"
	"os"
)

func main() {
	if cwd, err := os.Getwd(); err == nil {
		fmt.Printf("Dir: %s\n", cwd)
	} else {
		fmt.Println("Dir: n/a")
	}

	fmt.Print("Args:")
	for _, a := range os.Args {
		fmt.Printf(" %s", a)
	}
	fmt.Println()

	fmt.Println("Env:")
	for _, e := range os.Environ() {
		fmt.Printf(" %s\n", e)
	}
	fmt.Println()

	// `.` is the WASI preopen the device hands us (fd 3). The stick host
	// stubs out the underlying syscalls, so this may legitimately report
	// an empty listing — the goal here is to prove `_start` ran and the
	// `os` package wired through fd_readdir without trapping.
	fmt.Print("Root:")
	if entries, err := os.ReadDir("."); err == nil {
		for _, e := range entries {
			if e.IsDir() {
				fmt.Printf(" %s/", e.Name())
			} else {
				fmt.Printf(" %s", e.Name())
			}
		}
	}
	fmt.Println()
}
