package tui

import (
	"bufio"
	"fmt"
	"io"
	"strconv"
	"strings"

	"expos.dev/ayo/internal/catalog"
	"expos.dev/ayo/internal/manager"
	"expos.dev/ayo/internal/model"
)

type App struct {
	Manager manager.Manager
	Catalog catalog.Catalog
	Input   io.Reader
	Output  io.Writer
	Plain   bool
}

func (app App) Run() error {
	input := bufio.NewScanner(app.Input)
	filter, notice := "", ""
	for {
		packages := app.filtered(filter)
		app.draw(packages, filter, notice)
		notice = ""
		if !input.Scan() {
			return input.Err()
		}
		command := strings.TrimSpace(input.Text())
		switch {
		case command == "q" || command == "quit":
			return nil
		case command == "" || command == "h" || command == "help":
			continue
		case command == "i" || command == "installed":
			notice = app.installed()
		case strings.HasPrefix(command, "/"):
			filter = strings.TrimSpace(strings.TrimPrefix(command, "/"))
		case strings.HasPrefix(command, "d "):
			notice = app.details(strings.TrimSpace(strings.TrimPrefix(command, "d ")), packages)
		case strings.HasPrefix(command, "x "):
			name := strings.TrimSpace(strings.TrimPrefix(command, "x "))
			if err := app.Manager.Yeet(name); err != nil {
				notice = "Cannot remove: " + err.Error()
			} else {
				notice = "Yeeted " + name + ". Its FIN history remains."
			}
		default:
			pkg, ok := selectPackage(command, packages)
			if !ok {
				notice = "Choose a package number/name, /search, d NAME, x NAME, i, or q."
				continue
			}
			state, err := app.Manager.Read()
			if err != nil {
				notice = err.Error()
				continue
			}
			plan, err := app.Catalog.Resolve(pkg.Name, app.Manager.Dimension, state)
			if err != nil {
				notice = "Cannot resolve: " + err.Error()
				continue
			}
			if len(plan) == 0 {
				notice = pkg.Name + " is already active and compatible."
				continue
			}
			if app.Manager.Authority != model.Operator {
				notice = "DIESE: browsing is allowed, but installation needs Operator authority. Relaunch with --authority operator."
				continue
			}
			fmt.Fprintf(app.Output, "\nInstall %s? Plan: %s [y/N] ", pkg.Name, planNames(plan))
			if !input.Scan() {
				return input.Err()
			}
			if !strings.EqualFold(strings.TrimSpace(input.Text()), "y") {
				notice = "Install cancelled; no state changed."
				continue
			}
			if err := app.Manager.InstallPlan(plan); err != nil {
				notice = "Install failed: " + err.Error()
			} else {
				notice = fmt.Sprintf("Downloaded and installed %s with %d Form(s) in one verified transaction. nice.", pkg.Name, len(plan))
			}
		}
	}
}

func (app App) draw(packages []catalog.Package, filter, notice string) {
	if !app.Plain {
		fmt.Fprint(app.Output, "\x1b[2J\x1b[H")
	}
	state, err := app.Manager.Read()
	installed := 0
	if err == nil {
		for _, form := range state.Packages {
			if form.Dimension == app.Manager.Dimension && form.Active {
				installed++
			}
		}
	}
	fmt.Fprintln(app.Output, "┌──────────────────────────────────────────────────────────────────────┐")
	fmt.Fprintln(app.Output, "│  AYO v3 // VERIFIED PACKAGE DECK                                    │")
	fmt.Fprintf(app.Output, "│  %-30s Dimension: %-12s Active: %-3d │\n", app.Catalog.Name, app.Manager.Dimension, installed)
	fmt.Fprintln(app.Output, "├────┬────────────────┬──────────┬────────────────────────────────────┤")
	for index, pkg := range packages {
		marker := " "
		if isInstalled(state, app.Manager.Dimension, pkg.Name) {
			marker = "✓"
		}
		fmt.Fprintf(app.Output, "│ %2d%s│ %-14s │ %-8s │ %-34s │\n", index+1, marker, trim(pkg.Name, 14), trim(pkg.Version, 8), trim(pkg.Summary, 34))
	}
	if len(packages) == 0 {
		fmt.Fprintln(app.Output, "│                    no Package Forms match                           │")
	}
	fmt.Fprintln(app.Output, "└────┴────────────────┴──────────┴────────────────────────────────────┘")
	if filter != "" {
		fmt.Fprintln(app.Output, "filter:", filter)
	}
	if notice != "" {
		fmt.Fprintln(app.Output, "\n", notice)
	}
	fmt.Fprint(app.Output, "\n[number/name] install  /text search  d NAME details  x NAME remove\n[i] installed          [q] quit\nayo> ")
}

func (app App) filtered(filter string) []catalog.Package {
	if filter == "" {
		return app.Catalog.Packages
	}
	needle := strings.ToLower(filter)
	result := make([]catalog.Package, 0)
	for _, pkg := range app.Catalog.Packages {
		if strings.Contains(strings.ToLower(pkg.Name+" "+pkg.Summary+" "+pkg.Category), needle) {
			result = append(result, pkg)
		}
	}
	return result
}

func (app App) installed() string {
	state, err := app.Manager.Read()
	if err != nil {
		return err.Error()
	}
	items := make([]string, 0)
	for _, form := range state.Packages {
		if form.Dimension == app.Manager.Dimension {
			status := "inactive"
			if form.Active {
				status = "active"
			}
			files := 0
			if form.Artifact != nil {
				files = len(form.Artifact.Files)
			}
			items = append(items, fmt.Sprintf("%s@%s (%s, %d files)", form.Name, form.Version, status, files))
		}
	}
	if len(items) == 0 {
		return "No Package Forms installed in this Dimension."
	}
	return "Installed: " + strings.Join(items, ", ")
}

func (app App) details(identity string, packages []catalog.Package) string {
	pkg, ok := selectPackage(identity, packages)
	if !ok {
		pkg, ok = app.Catalog.Find(identity)
	}
	if !ok {
		return "No registry Package Form named " + identity
	}
	artifact := "legacy metadata only"
	if pkg.Artifact != nil {
		artifact = fmt.Sprintf("%s (%s, sha256 %s)", pkg.Artifact.Source, pkg.Artifact.Format, pkg.Artifact.SHA256)
	}
	return fmt.Sprintf("%s@%s — %s\nCategory: %s\nArchitectures: %s\nProvides: %s\nCapabilities: %s\nDepends: %s\nCompatibility: %s\nArtifact: %s\nCatalog trust: %s\nCatalog checksum: %s", pkg.Name, pkg.Version, pkg.Summary, pkg.Category, values(pkg.Architectures), values(pkg.ProvidedForms), values(pkg.Capabilities), values(pkg.Dependencies), values(pkg.Compatibility), artifact, app.Catalog.Trust, pkg.Checksum)
}

func selectPackage(identity string, packages []catalog.Package) (catalog.Package, bool) {
	if number, err := strconv.Atoi(identity); err == nil && number > 0 && number <= len(packages) {
		return packages[number-1], true
	}
	for _, pkg := range packages {
		if strings.EqualFold(pkg.Name, identity) {
			return pkg, true
		}
	}
	return catalog.Package{}, false
}

func isInstalled(state model.State, dimension, name string) bool {
	form, err := state.Find(name, dimension)
	return err == nil && form.Active
}

func planNames(plan []manager.InstallSpec) string {
	names := make([]string, len(plan))
	for index, spec := range plan {
		names[index] = spec.Name + "@" + spec.Version
	}
	return strings.Join(names, " → ")
}
func values(items []string) string {
	if len(items) == 0 {
		return "-"
	}
	return strings.Join(items, ", ")
}
func trim(value string, width int) string {
	if len(value) <= width {
		return value
	}
	if width < 2 {
		return value[:width]
	}
	return value[:width-1] + "…"
}
