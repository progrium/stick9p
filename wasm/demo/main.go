package main

import (
	"fmt"
	"log"
	"os"
	"time"
)

func main() {
	log.Println("starting..")
	if err := os.WriteFile("/dev/display/ctl", []byte("fill 000000\n"), 0644); err != nil {
		log.Fatalf("error writing to display/ctl: %v", err)
	}

	pressed := false
	changed := false
	for {
		buf, err := os.ReadFile("/dev/buttons/a")
		if err != nil {
			log.Fatalf("error reading buttons/a: %v", err)
		}
		if string(buf) == "1\n" {
			if !pressed {
				changed = true
			}
			pressed = true
		} else {
			if pressed {
				changed = true
			}
			pressed = false
		}

		if changed {
			if pressed {
				if err := os.WriteFile("/dev/display/ctl", []byte("fill ff0000\n"), 0644); err != nil {
					fmt.Println("error writing to display/ctl")
					continue
				}
			} else {
				if err := os.WriteFile("/dev/display/ctl", []byte("fill 000000\n"), 0644); err != nil {
					fmt.Println("error writing to display/ctl")
					continue
				}
			}
			changed = false
		}

		time.Sleep(200 * time.Millisecond)
	}

}
