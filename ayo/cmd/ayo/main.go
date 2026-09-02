package main

import (
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"hexaos.dev/ayo/internal/manager"
	"hexaos.dev/ayo/internal/model"
	"hexaos.dev/ayo/internal/store"
)

func main() {
	os.Exit(run(os.Args[1:]))
}

func run(arguments []string) int {
	flags := flag.NewFlagSet("ayo", flag.ContinueOnError)
	flags.SetOutput(os.Stderr)
	statePath := flags.String("state", defaultStatePath(), "development HexaFS bridge state")
	authority := flags.String("authority", "power", "operator, power, or guest")
	dimension := flags.String("dimension", "Stable", "active Dimension")
	if err := flags.Parse(arguments); err != nil {
		return 2
	}
	args := flags.Args()
	if len(args) == 0 {
		usage()
		return 2
	}

	level := model.Authority(strings.ToLower(*authority))
	if level != model.Operator && level != model.Power && level != model.Guest {
		fmt.Fprintln(os.Stderr, "ayo: unknown authority", *authority)
		return 2
	}
	ayo := manager.Manager{Store: store.JSONBridge{Path: *statePath}, Authority: level, Dimension: *dimension}
	command, operands := args[0], args[1:]
	var err error
	switch command {
	case "slap":
		if len(operands) < 1 {
			err = fmt.Errorf("slap needs a package name")
		} else {
			version := "0.1.0"
			if len(operands) > 1 {
				version = operands[1]
			}
			err = ayo.Slap(operands[0], version, nil, nil)
		}
	case "yeet", "ghost", "dodge":
		if len(operands) != 1 {
			err = fmt.Errorf("%s needs one FIN or name", command)
			break
		}
		switch command {
		case "yeet":
			err = ayo.Yeet(operands[0])
		case "ghost":
			err = ayo.Ghost(operands[0])
		case "dodge":
			err = ayo.Dodge(operands[0])
		}
	case "highfive":
		if len(operands) != 2 {
			err = fmt.Errorf("highfive needs two FINs or names")
		} else {
			err = ayo.Highfive(operands[0], operands[1])
		}
	case "chill":
		err = ayo.Chill()
	case "fix":
		err = ayo.Fix()
	case "manifest":
		err = ayo.Manifest()
	case "glance":
		err = glance(ayo, operands)
	case "vibecheck":
		err = vibecheck(ayo)
	case "flex":
		err = flex(ayo)
	default:
		err = fmt.Errorf("unknown command %q", command)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "ayo:", err)
		return 1
	}
	if isMutation(command) {
		fmt.Printf("ayo %s: transaction committed in %s. nice.\n", command, *dimension)
	}
	return 0
}

func glance(ayo manager.Manager, operands []string) error {
	state, err := ayo.Read()
	if err != nil {
		return err
	}
	for _, form := range state.Packages {
		if form.Dimension != ayo.Dimension {
			continue
		}
		if len(operands) == 1 && operands[0] != form.Name && operands[0] != form.FIN {
			continue
		}
		status := "retired"
		if form.Active {
			status = "active"
		} else if form.Hidden {
			status = "ghosted"
		}
		fmt.Printf("%s  %s  v%s  %s  rev=%d\n", form.FIN, form.Name, form.Version, status, form.Revision)
	}
	return nil
}

func vibecheck(ayo manager.Manager) error {
	state, err := ayo.Read()
	if err != nil {
		return err
	}
	if err := manager.Vibecheck(state); err != nil {
		return err
	}
	fmt.Println("vibecheck: Forms, FINs, and dependencies look healthy.")
	return nil
}

func flex(ayo manager.Manager) error {
	state, err := ayo.Read()
	if err != nil {
		return err
	}
	active, hidden := 0, 0
	for _, form := range state.Packages {
		if form.Dimension == ayo.Dimension {
			if form.Active {
				active++
			}
			if form.Hidden {
				hidden++
			}
		}
	}
	fmt.Printf("Dimension %s: forms=%d active=%d ghosted=%d journal=%d manifests=%d\n", ayo.Dimension, len(state.Packages), active, hidden, len(state.Journal), len(state.Manifests))
	return nil
}

func isMutation(command string) bool {
	switch command {
	case "slap", "yeet", "ghost", "dodge", "highfive", "chill", "fix", "manifest":
		return true
	}
	return false
}

func defaultStatePath() string {
	if root, err := os.UserConfigDir(); err == nil {
		return filepath.Join(root, "hexaos", "ayo-bridge.json")
	}
	return "ayo-bridge.json"
}

func usage() {
	fmt.Fprintln(os.Stderr, `ayo — native HexaOS Package Form manager

usage: ayo [--authority operator|power|guest] [--dimension Stable] COMMAND

commands: slap yeet glance chill fix ghost manifest highfive dodge vibecheck flex`)
}
