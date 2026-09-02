package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"hexaos.dev/ayo/internal/catalog"
	"hexaos.dev/ayo/internal/manager"
	"hexaos.dev/ayo/internal/model"
	"hexaos.dev/ayo/internal/store"
	"hexaos.dev/ayo/internal/tui"
)

func main() { os.Exit(run(os.Args[1:])) }

func run(arguments []string) int {
	flags := flag.NewFlagSet("ayo", flag.ContinueOnError)
	flags.SetOutput(os.Stderr)
	statePath := flags.String("state", defaultStatePath(), "development HexaFS bridge state")
	authority := flags.String("authority", "power", "operator, power, or guest")
	dimension := flags.String("dimension", "Stable", "active Dimension")
	registrySource := flags.String("registry", os.Getenv("AYO_REGISTRY"), "HTTPS URL or local ayo catalog JSON")
	registryKey := flags.String("registry-key", os.Getenv("AYO_REGISTRY_KEY"), "base64 Ed25519 registry public key")
	plainTUI := flags.Bool("plain", false, "do not clear the screen while using the TUI")
	if err := flags.Parse(arguments); err != nil {
		return 2
	}
	level := model.Authority(strings.ToLower(*authority))
	if level != model.Operator && level != model.Power && level != model.Guest {
		fmt.Fprintln(os.Stderr, "ayo: unknown authority", *authority)
		return 2
	}
	ayo := manager.Manager{Store: store.JSONBridge{Path: *statePath}, Authority: level, Dimension: *dimension}
	args := flags.Args()
	if len(args) == 0 || args[0] == "tui" {
		loaded, err := catalog.Load(context.Background(), *registrySource, *registryKey)
		if err != nil {
			fmt.Fprintln(os.Stderr, "ayo:", err)
			return 1
		}
		if err := (tui.App{Manager: ayo, Catalog: loaded, Input: os.Stdin, Output: os.Stdout, Plain: *plainTUI}).Run(); err != nil {
			fmt.Fprintln(os.Stderr, "ayo:", err)
			return 1
		}
		return 0
	}
	command, operands := args[0], args[1:]
	var err error
	switch command {
	case "slap":
		err = slap(ayo, operands)
	case "yeet":
		err = yeet(ayo, operands)
	case "ghost", "dodge":
		if len(operands) != 1 {
			err = fmt.Errorf("%s needs one FIN or name", command)
		} else if command == "ghost" {
			err = ayo.Ghost(operands[0])
		} else {
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
		label := ""
		if len(operands) > 1 {
			err = errorsFor("manifest accepts at most one label")
		} else {
			if len(operands) == 1 {
				label = operands[0]
			}
			err = ayo.ManifestNamed(label)
		}
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
		fmt.Printf("ayo %s: HexaFS transaction committed in %s. nice.\n", command, *dimension)
	}
	return 0
}

type listFlag []string

func (values *listFlag) String() string { return strings.Join(*values, ",") }
func (values *listFlag) Set(value string) error {
	for _, item := range strings.Split(value, ",") {
		if strings.TrimSpace(item) != "" {
			*values = append(*values, strings.TrimSpace(item))
		}
	}
	return nil
}

func slap(ayo manager.Manager, args []string) error {
	flags := flag.NewFlagSet("slap", flag.ContinueOnError)
	flags.SetOutput(os.Stderr)
	var capabilities, dependencies, provided, compatibility, pimpValues listFlag
	flags.Var(&capabilities, "cap", "required capability (repeat or comma-separate)")
	flags.Var(&dependencies, "dep", "dependency such as Network@>=2.0.0")
	flags.Var(&provided, "provide", "provided Form")
	flags.Var(&compatibility, "compat", "compatibility requirement")
	flags.Var(&pimpValues, "pimp", "PIMP key=value specification")
	if err := flags.Parse(args); err != nil {
		return err
	}
	operands := flags.Args()
	if len(operands) < 1 || len(operands) > 2 {
		return errorsFor("slap usage: slap [options] NAME [VERSION]")
	}
	version := "0.1.0"
	if len(operands) == 2 {
		version = operands[1]
	}
	pimp := make(map[string]string)
	for _, raw := range pimpValues {
		pair := strings.SplitN(raw, "=", 2)
		if len(pair) != 2 || pair[0] == "" {
			return fmt.Errorf("invalid PIMP specification %q", raw)
		}
		pimp[pair[0]] = pair[1]
	}
	return ayo.SlapSpec(manager.InstallSpec{Name: operands[0], Version: version, Capabilities: capabilities, Dependencies: dependencies, ProvidedForms: provided, Compatibility: compatibility, PIMP: pimp})
}

func yeet(ayo manager.Manager, args []string) error {
	flags := flag.NewFlagSet("yeet", flag.ContinueOnError)
	flags.SetOutput(os.Stderr)
	force := flags.Bool("force", false, "deactivate dependents during reconciliation")
	if err := flags.Parse(args); err != nil {
		return err
	}
	operands := flags.Args()
	if len(operands) != 1 {
		return errorsFor("yeet needs one FIN or name")
	}
	return ayo.YeetForce(operands[0], *force)
}

func glance(ayo manager.Manager, operands []string) error {
	if len(operands) > 1 {
		return errorsFor("glance accepts at most one FIN or name")
	}
	state, err := ayo.Read()
	if err != nil {
		return err
	}
	for _, form := range state.Packages {
		if form.Dimension != ayo.Dimension || (len(operands) == 1 && operands[0] != form.Name && operands[0] != form.FIN) {
			continue
		}
		status := "retired"
		if form.Active {
			status = "active"
		} else if form.Hidden {
			status = "ghosted"
		} else if form.Excluded {
			status = "excluded"
		}
		fmt.Printf("%s  %s  v%s  %s  rev=%d\n", form.FIN, form.Name, form.Version, status, form.Revision)
		if len(operands) == 1 {
			fmt.Printf("  capabilities=%s provides=%s depends=%s\n", display(form.Capabilities), display(form.ProvidedForms), display(form.Dependencies))
			if form.LastError != "" {
				fmt.Println("  diagnostic=" + form.LastError)
			}
		}
	}
	return nil
}

func vibecheck(ayo manager.Manager) error {
	state, err := ayo.Read()
	if err != nil {
		return err
	}
	report := manager.InspectHealth(state, ayo.Dimension)
	if len(report.Issues) > 0 {
		return errorsFor(strings.Join(report.Issues, "; "))
	}
	fmt.Printf("vibecheck: healthy=%d active=%d; Forms, FINs, relationships, and constraints look immaculate.\n", report.Healthy, report.Active)
	return nil
}

func flex(ayo manager.Manager) error {
	state, err := ayo.Read()
	if err != nil {
		return err
	}
	report := manager.InspectHealth(state, ayo.Dimension)
	capabilities, relationships := 0, 0
	forms := 0
	for _, form := range state.Packages {
		if form.Dimension == ayo.Dimension {
			forms++
			capabilities += len(form.Capabilities)
			relationships += len(form.Dependencies)
		}
	}
	fmt.Printf("Dimension %s: forms=%d active=%d ghosted=%d excluded=%d capabilities=%d relationships=%d journal=%d manifests=%d\n", ayo.Dimension, forms, report.Active, report.Hidden, report.Excluded, capabilities, relationships, len(state.Journal), len(state.Manifests))
	return nil
}

func display(values []string) string {
	if len(values) == 0 {
		return "-"
	}
	return strings.Join(values, ",")
}
func errorsFor(message string) error { return fmt.Errorf("%s", message) }
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
	fmt.Fprintln(os.Stderr, `ayo v2 — HexaOS Package Form manager

usage: ayo [--authority operator|power|guest] [--dimension Stable]
       ayo [global options] COMMAND

No command opens the interactive package catalog. Use --authority operator to install.
Registry options: --registry HTTPS_URL --registry-key BASE64_ED25519_KEY --plain

commands: slap yeet glance chill fix ghost manifest highfive dodge vibecheck flex

slap options: --cap NAME --dep 'NAME@>=VERSION' --provide FORM --compat RULE --pimp KEY=VALUE`)
}
