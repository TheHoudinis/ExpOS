package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"expos.dev/ayo/internal/catalog"
	"expos.dev/ayo/internal/manager"
	"expos.dev/ayo/internal/model"
	"expos.dev/ayo/internal/store"
	"expos.dev/ayo/internal/tui"
)

func main() { os.Exit(run(os.Args[1:])) }

func run(arguments []string) int {
	flags := flag.NewFlagSet("ayo", flag.ContinueOnError)
	flags.SetOutput(os.Stderr)
	statePath := flags.String("state", defaultStatePath(), "development ExpFS bridge state")
	authority := flags.String("authority", "power", "operator, power, or guest")
	dimension := flags.String("dimension", "Stable", "active Dimension")
	registrySource := flags.String("registry", os.Getenv("AYO_REGISTRY"), "HTTPS URL or local ayo catalog JSON")
	registryKey := flags.String("registry-key", os.Getenv("AYO_REGISTRY_KEY"), "base64 Ed25519 registry public key")
	installRoot := flags.String("root", defaultInstallRoot(), "user-owned artifact install root")
	plainTUI := flags.Bool("plain", false, "do not clear the screen while using the TUI")
	showVersion := flags.Bool("version", false, "print ayo version")
	flags.Usage = usage
	if err := flags.Parse(arguments); err != nil {
		return 2
	}
	if *showVersion {
		fmt.Println("ayo v3")
		return 0
	}
	level := model.Authority(strings.ToLower(*authority))
	if level != model.Operator && level != model.Power && level != model.Guest {
		fmt.Fprintln(os.Stderr, "ayo: unknown authority", *authority)
		return 2
	}
	ayo := manager.Manager{Store: store.JSONBridge{Path: *statePath}, Authority: level, Dimension: *dimension, InstallRoot: *installRoot}
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
	case "install":
		err = install(context.Background(), ayo, *registrySource, *registryKey, operands)
	case "slap":
		if len(operands) == 1 {
			err = install(context.Background(), ayo, *registrySource, *registryKey, operands)
		} else {
			err = slap(ayo, operands)
		}
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
		err = glance(context.Background(), ayo, *registrySource, *registryKey, operands)
	case "vibecheck":
		err = vibecheck(ayo)
	case "flex":
		err = flex(ayo)
	case "files":
		err = files(ayo, operands)
	case "recover":
		if len(operands) != 0 {
			err = errorsFor("recover accepts no operands")
		} else {
			err = ayo.RecoverArtifacts()
		}
	case "version":
		fmt.Println("ayo v3")
	default:
		err = fmt.Errorf("unknown command %q", command)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "ayo:", err)
		return 1
	}
	if isMutation(command) {
		fmt.Printf("ayo %s: ExpFS transaction committed in %s. nice.\n", command, *dimension)
	}
	return 0
}

func install(ctx context.Context, ayo manager.Manager, source, publicKey string, operands []string) error {
	if len(operands) != 1 {
		return errorsFor("install needs one package name")
	}
	registry, err := catalog.Load(ctx, source, publicKey)
	if err != nil {
		return err
	}
	state, err := ayo.Read()
	if err != nil {
		return err
	}
	plan, err := registry.Resolve(operands[0], ayo.Dimension, state)
	if err != nil {
		return err
	}
	if len(plan) == 0 {
		return fmt.Errorf("%s is already installed and compatible", operands[0])
	}
	return ayo.InstallPlanContext(ctx, plan)
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
	source := flags.String("source", "", "HTTPS URL, file URL, or local artifact path")
	checksum := flags.String("sha256", "", "artifact SHA-256")
	format := flags.String("format", "", "raw, tar, or tar.gz")
	target := flags.String("target", "", "install path for a raw artifact")
	mode := flags.Uint("mode", 0o644, "raw artifact mode (executable bits only)")
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
	var artifactSpec *model.ArtifactSpec
	if *source != "" || *checksum != "" || *format != "" || *target != "" {
		if *source == "" || *checksum == "" || *format == "" {
			return errorsFor("artifact slap requires --source, --sha256, and --format")
		}
		artifactSpec = &model.ArtifactSpec{Source: *source, SHA256: *checksum, Format: *format, Target: *target, Mode: uint32(*mode)}
	}
	return ayo.SlapSpec(manager.InstallSpec{Name: operands[0], Version: version, Capabilities: capabilities, Dependencies: dependencies, ProvidedForms: provided, Compatibility: compatibility, PIMP: pimp, Artifact: artifactSpec})
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

func glance(ctx context.Context, ayo manager.Manager, source, publicKey string, operands []string) error {
	if len(operands) > 1 {
		return errorsFor("glance accepts at most one FIN, name, category, or search term")
	}
	state, err := ayo.Read()
	if err != nil {
		return err
	}
	if len(operands) == 1 {
		registry, loadErr := catalog.Load(ctx, source, publicKey)
		if loadErr != nil {
			return loadErr
		}
		if pkg, found := registry.Find(operands[0]); found {
			printCatalogPackage(pkg, registry.Trust, installedForm(state, ayo.Dimension, pkg.Name))
			return nil
		}
		matches := registry.Search(operands[0])
		if len(matches) > 0 {
			for _, pkg := range matches {
				fmt.Printf("%-16s %-16s v%-8s %s\n", pkg.Name, pkg.Category, pkg.Version, pkg.Summary)
			}
			return nil
		}
	}
	found := false
	for _, form := range state.Packages {
		if form.Dimension != ayo.Dimension || (len(operands) == 1 && !strings.EqualFold(operands[0], form.Name) && operands[0] != form.FIN) {
			continue
		}
		found = true
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
			if form.Artifact != nil {
				fmt.Printf("  artifact=%s sha256=%s files=%d root=%s\n", form.Artifact.Source, form.Artifact.SHA256, len(form.Artifact.Files), ayo.InstallRoot)
			}
			if form.LastError != "" {
				fmt.Println("  diagnostic=" + form.LastError)
			}
		}
	}
	if len(operands) == 1 && !found {
		return fmt.Errorf("no installed or catalog Package Form matches %q", operands[0])
	}
	return nil
}

func installedForm(state model.State, dimension, name string) *model.PackageForm {
	form, err := state.Find(name, dimension)
	if err != nil {
		return nil
	}
	return form
}

func printCatalogPackage(pkg catalog.Package, trust string, installed *model.PackageForm) {
	fin, status := "assigned during installation", "available"
	if installed != nil {
		fin = installed.FIN
		status = "installed"
		if !installed.Active {
			status = "inactive"
		}
	}
	signature := "no (unverified local metadata)"
	if trust == "ed25519" {
		signature = "yes (Ed25519 catalog)"
	} else if trust == "built-in" {
		signature = "compiled-in release trust"
	} else if trust == "checksummed" {
		signature = "no (checksummed metadata only)"
	}
	fmt.Printf("%s\nVersion: %s\nFIN: %s\nSignature: %s\nCategory: %s\nArchitectures: %s\nStatus: %s\nDependencies: %s\nProvides: %s\nCapabilities: %s\n", pkg.Name, pkg.Version, fin, signature, pkg.Category, display(pkg.Architectures), status, display(pkg.Dependencies), display(pkg.ProvidedForms), display(pkg.Capabilities))
}

func files(ayo manager.Manager, operands []string) error {
	if len(operands) != 1 {
		return errorsFor("files needs one FIN or package name")
	}
	state, err := ayo.Read()
	if err != nil {
		return err
	}
	form, err := state.Find(operands[0], ayo.Dimension)
	if err != nil {
		return err
	}
	if form.Artifact == nil {
		fmt.Println("metadata-only Package Form; no artifact files")
		return nil
	}
	for _, file := range form.Artifact.Files {
		fmt.Printf("%s  %s  %d bytes\n", file.SHA256, file.Path, file.Size)
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
	files := 0
	for _, form := range state.Packages {
		if form.Dimension == ayo.Dimension && form.Artifact != nil {
			files += len(form.Artifact.Files)
		}
	}
	fmt.Printf("Dimension %s: forms=%d active=%d ghosted=%d excluded=%d capabilities=%d relationships=%d owned-files=%d journal=%d manifests=%d\n", ayo.Dimension, forms, report.Active, report.Hidden, report.Excluded, capabilities, relationships, files, len(state.Journal), len(state.Manifests))
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
	case "install", "slap", "yeet", "ghost", "dodge", "highfive", "chill", "fix", "manifest", "recover":
		return true
	}
	return false
}
func defaultStatePath() string {
	if root, err := os.UserConfigDir(); err == nil {
		return filepath.Join(root, "expos", "ayo-bridge.json")
	}
	return "ayo-bridge.json"
}

func defaultInstallRoot() string {
	if configured := os.Getenv("AYO_ROOT"); configured != "" {
		return configured
	}
	if home, err := os.UserHomeDir(); err == nil {
		return filepath.Join(home, ".local", "share", "expos", "ayo-root")
	}
	return "ayo-root"
}

func usage() {
	fmt.Fprintln(os.Stderr, `ayo v3 — verified artifact + Package Form manager

usage: ayo [--authority operator|power|guest] [--dimension Stable] [--root PATH]
       ayo [global options] COMMAND

No command opens the interactive package catalog. Operator authority is required to install.
Registry options: --registry HTTPS_URL --registry-key BASE64_ED25519_KEY --plain

commands: install slap yeet files recover glance chill fix ghost manifest
          highfive dodge vibecheck flex version

slap NAME (or install NAME) resolves, downloads, verifies, stages, and owns catalog artifacts.
slap NAME VERSION registers a local metadata Package Form; artifact options attach bytes.
slap artifact options: --source URL --sha256 HEX --format raw|tar|tar.gz [--target PATH]
metadata options: --cap NAME --dep 'NAME@>=VERSION' --provide FORM --compat RULE --pimp KEY=VALUE

Ayo never executes package scripts and refuses system/root installation paths.`)
}
