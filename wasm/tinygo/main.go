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

	// `.` is the WASI preopen root (`/`). gocheck lists top-level 9P entries.
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
