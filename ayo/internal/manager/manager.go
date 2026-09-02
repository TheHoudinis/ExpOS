package manager

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"time"

	"hexaos.dev/ayo/internal/model"
	"hexaos.dev/ayo/internal/store"
)

type Manager struct {
	Store     store.Store
	Authority model.Authority
	Dimension string
}

type InstallSpec struct {
	Name, Version string
	Capabilities  []string
	ProvidedForms []string
	Dependencies  []string
	Compatibility []string
	PIMP          map[string]string
}

type HealthReport struct {
	Healthy, Active, Hidden, Excluded int
	Issues                            []string
}

const hexaOSVersion = "8.0.0"

func (manager Manager) Read() (model.State, error) { return manager.Store.Load() }

func (manager Manager) mutate(action string, operation func(*model.State) (string, string, error)) error {
	if manager.Authority != model.Operator {
		return fmt.Errorf("DIESE denied %s: Operator authority is required", action)
	}
	commit := func(state *model.State) error { return state.Commit(action, operation) }
	if atomic, ok := manager.Store.(store.AtomicStore); ok {
		return atomic.Update(commit)
	}
	state, err := manager.Store.Load()
	if err != nil {
		return err
	}
	if err := commit(&state); err != nil {
		return err
	}
	return manager.Store.Save(state)
}

func (manager Manager) Slap(name, version string, capabilities, dependencies []string) error {
	return manager.SlapSpec(InstallSpec{Name: name, Version: version, Capabilities: capabilities, Dependencies: dependencies})
}

func (manager Manager) SlapSpec(spec InstallSpec) error {
	if strings.TrimSpace(spec.Name) == "" || strings.TrimSpace(spec.Version) == "" {
		return errors.New("slap needs a Package Form name and version")
	}
	if _, err := parseVersion(spec.Version); err != nil {
		return err
	}
	if err := checkCompatibility(spec.Compatibility, manager.Dimension); err != nil {
		return err
	}
	if strings.EqualFold(spec.PIMP["activation"], "denied") {
		return errors.New("PIMP denied package activation")
	}
	for _, dependency := range spec.Dependencies {
		if _, _, err := parseDependency(dependency); err != nil {
			return err
		}
	}
	return manager.mutate("slap", func(state *model.State) (string, string, error) {
		if err := checkDependencies(*state, manager.Dimension, spec.Dependencies); err != nil {
			return "", "", err
		}
		now := time.Now().UTC()
		if existing, err := state.Find(spec.Name, manager.Dimension); err == nil {
			existing.Version, existing.Active, existing.DesiredActive = spec.Version, true, true
			existing.Hidden, existing.Excluded, existing.LastError = false, false, ""
			existing.Capabilities = unique(spec.Capabilities)
			existing.ProvidedForms = unique(spec.ProvidedForms)
			existing.Dependencies = unique(spec.Dependencies)
			existing.Compatibility = unique(spec.Compatibility)
			existing.PIMP = cloneMap(spec.PIMP)
			existing.Revision++
			existing.UpdatedAt = now
			return existing.FIN, "updated " + existing.Name + "@" + existing.Version, nil
		}
		fin, err := model.NewFIN()
		if err != nil {
			return "", "", err
		}
		state.Packages = append(state.Packages, model.PackageForm{
			FIN: fin, Name: spec.Name, Version: spec.Version, Dimension: manager.Dimension,
			Active: true, DesiredActive: true, Revision: 1,
			Capabilities: unique(spec.Capabilities), ProvidedForms: unique(spec.ProvidedForms),
			Dependencies: unique(spec.Dependencies), Compatibility: unique(spec.Compatibility),
			PIMP: cloneMap(spec.PIMP), InstalledAt: now, UpdatedAt: now,
		})
		state.Sort()
		return fin, "activated " + spec.Name + "@" + spec.Version, nil
	})
}

func (manager Manager) Yeet(identity string) error { return manager.YeetForce(identity, false) }

func (manager Manager) YeetForce(identity string, force bool) error {
	return manager.mutate("yeet", func(state *model.State) (string, string, error) {
		form, err := state.Find(identity, manager.Dimension)
		if err != nil {
			return "", "", err
		}
		if dependents := activeDependents(*state, manager.Dimension, form.Name); len(dependents) > 0 && !force {
			return form.FIN, "", fmt.Errorf("cannot yeet %s; active dependents: %s (use --force to reconcile them off)", form.Name, strings.Join(dependents, ", "))
		}
		form.Active, form.DesiredActive, form.Hidden = false, false, false
		form.Revision++
		form.UpdatedAt = time.Now().UTC()
		if force {
			reconcile(state, manager.Dimension)
		}
		return form.FIN, "revoked activation and capabilities", nil
	})
}

func (manager Manager) Ghost(identity string) error {
	return manager.mutate("ghost", func(state *model.State) (string, string, error) {
		form, err := state.Find(identity, manager.Dimension)
		if err != nil {
			return "", "", err
		}
		if dependents := activeDependents(*state, manager.Dimension, form.Name); len(dependents) > 0 {
			return form.FIN, "", fmt.Errorf("cannot ghost %s; active dependents: %s", form.Name, strings.Join(dependents, ", "))
		}
		form.Active, form.DesiredActive, form.Hidden = false, false, true
		form.Revision++
		form.UpdatedAt = time.Now().UTC()
		return form.FIN, "identity and history preserved", nil
	})
}

func (manager Manager) Dodge(identity string) error {
	return manager.mutate("dodge", func(state *model.State) (string, string, error) {
		form, err := state.Find(identity, manager.Dimension)
		if err != nil {
			return "", "", err
		}
		form.Active, form.DesiredActive, form.Excluded = false, false, true
		form.Revision++
		form.UpdatedAt = time.Now().UTC()
		reconcile(state, manager.Dimension)
		return form.FIN, "excluded from reconciliation", nil
	})
}

func (manager Manager) Highfive(leftIdentity, rightIdentity string) error {
	return manager.mutate("highfive", func(state *model.State) (string, string, error) {
		left, err := state.Find(leftIdentity, manager.Dimension)
		if err != nil {
			return "", "", err
		}
		right, err := state.Find(rightIdentity, manager.Dimension)
		if err != nil {
			return "", "", err
		}
		if !left.Active || !right.Active {
			return left.FIN, "", errors.New("highfive requires two active Package Forms")
		}
		left.Capabilities = unique(append(left.Capabilities, right.Capabilities...))
		left.ProvidedForms = unique(append(left.ProvidedForms, right.ProvidedForms...))
		left.MergedFrom = unique(append(left.MergedFrom, right.FIN))
		left.Revision++
		left.UpdatedAt = time.Now().UTC()
		return left.FIN, "controlled merge from " + right.Name, nil
	})
}

func (manager Manager) Manifest() error { return manager.ManifestNamed("") }

func (manager Manager) ManifestNamed(label string) error {
	return manager.mutate("manifest", func(state *model.State) (string, string, error) {
		if label == "" {
			label = fmt.Sprintf("checkpoint-%d", state.Sequence+1)
		}
		snapshot := state.Clone().Packages
		raw, err := json.Marshal(snapshot)
		if err != nil {
			return "", "", err
		}
		digest := sha256.Sum256(raw)
		state.Manifests = append(state.Manifests, model.Manifest{Sequence: state.Sequence + 1, Label: label, At: time.Now().UTC(), Forms: len(snapshot), Checksum: hex.EncodeToString(digest[:]), Snapshot: snapshot})
		return "", "checkpoint " + label, nil
	})
}

func (manager Manager) Chill() error {
	return manager.mutate("chill", func(state *model.State) (string, string, error) {
		changed := reconcile(state, manager.Dimension)
		return "", fmt.Sprintf("PIMP desired state reconciled; %d Forms changed", changed), nil
	})
}

func (manager Manager) Fix() error {
	return manager.mutate("fix", func(state *model.State) (string, string, error) {
		for index := range state.Packages {
			form := &state.Packages[index]
			form.Capabilities = unique(form.Capabilities)
			form.ProvidedForms = unique(form.ProvidedForms)
			form.Dependencies = unique(form.Dependencies)
			form.Compatibility = unique(form.Compatibility)
		}
		state.Sort()
		changed := reconcile(state, manager.Dimension)
		return "", fmt.Sprintf("normalized metadata and repaired %d activation states", changed), nil
	})
}

func Vibecheck(state model.State) error {
	report := InspectHealth(state, "")
	if len(report.Issues) > 0 {
		return errors.New(strings.Join(report.Issues, "; "))
	}
	return nil
}

func InspectHealth(state model.State, dimension string) HealthReport {
	report := HealthReport{}
	if err := state.Validate(); err != nil {
		report.Issues = append(report.Issues, err.Error())
		return report
	}
	for _, form := range state.Packages {
		if dimension != "" && form.Dimension != dimension {
			continue
		}
		switch {
		case form.Active:
			report.Active++
		case form.Hidden:
			report.Hidden++
		case form.Excluded:
			report.Excluded++
		}
		if form.Active {
			if err := packageRequirements(state, form); err != nil {
				report.Issues = append(report.Issues, form.Name+": "+err.Error())
			} else {
				report.Healthy++
			}
		}
	}
	for _, manifest := range state.Manifests {
		if manifest.Checksum == "" || manifestChecksum(manifest.Snapshot) != manifest.Checksum {
			report.Issues = append(report.Issues, "manifest "+manifest.Label+" failed checksum validation")
		}
	}
	return report
}

func manifestChecksum(snapshot []model.PackageForm) string {
	raw, err := json.Marshal(snapshot)
	if err != nil {
		return ""
	}
	digest := sha256.Sum256(raw)
	return hex.EncodeToString(digest[:])
}

func reconcile(state *model.State, dimension string) int {
	changed := 0
	for pass := 0; pass <= len(state.Packages); pass++ {
		passChanged := false
		for index := range state.Packages {
			form := &state.Packages[index]
			if form.Dimension != dimension {
				continue
			}
			wanted := form.DesiredActive && !form.Hidden && !form.Excluded && !strings.EqualFold(form.PIMP["activation"], "denied")
			err := packageRequirements(*state, *form)
			shouldActivate := wanted && err == nil
			message := ""
			if wanted && err != nil {
				message = err.Error()
			}
			if form.Active != shouldActivate || form.LastError != message {
				form.Active, form.LastError = shouldActivate, message
				form.Revision++
				form.UpdatedAt = time.Now().UTC()
				changed++
				passChanged = true
			}
		}
		if !passChanged {
			break
		}
	}
	return changed
}

func packageRequirements(state model.State, form model.PackageForm) error {
	if err := checkCompatibility(form.Compatibility, form.Dimension); err != nil {
		return err
	}
	return checkDependencies(state, form.Dimension, form.Dependencies)
}

func checkCompatibility(requirements []string, dimension string) error {
	for _, raw := range requirements {
		rule := strings.TrimSpace(raw)
		switch {
		case strings.HasPrefix(strings.ToLower(rule), "dimension="):
			wanted := strings.TrimSpace(strings.SplitN(rule, "=", 2)[1])
			if !strings.EqualFold(wanted, dimension) {
				return fmt.Errorf("compatibility requires Dimension %s, current Dimension is %s", wanted, dimension)
			}
		case strings.HasPrefix(strings.ToLower(rule), "hexaos"):
			constraint := strings.TrimSpace(rule[len("hexaos"):])
			ok, err := satisfies(hexaOSVersion, constraint)
			if err != nil {
				return fmt.Errorf("invalid compatibility %q: %w", rule, err)
			}
			if !ok {
				return fmt.Errorf("compatibility requires HexaOS %s, current version is %s", constraint, hexaOSVersion)
			}
		default:
			return fmt.Errorf("unknown compatibility requirement %q", rule)
		}
	}
	return nil
}

func checkDependencies(state model.State, dimension string, dependencies []string) error {
	for _, raw := range dependencies {
		name, constraint, err := parseDependency(raw)
		if err != nil {
			return err
		}
		dependency, err := state.Find(name, dimension)
		if err != nil || !dependency.Active {
			return fmt.Errorf("missing active dependency %s", name)
		}
		ok, err := satisfies(dependency.Version, constraint)
		if err != nil {
			return err
		}
		if !ok {
			return fmt.Errorf("dependency %s is %s but requires %s", name, dependency.Version, constraint)
		}
	}
	return nil
}

func activeDependents(state model.State, dimension, name string) []string {
	var result []string
	for _, form := range state.Packages {
		if form.Dimension != dimension || !form.Active {
			continue
		}
		for _, raw := range form.Dependencies {
			dependency, _, _ := parseDependency(raw)
			if strings.EqualFold(dependency, name) {
				result = append(result, form.Name)
				break
			}
		}
	}
	sort.Strings(result)
	return result
}

func parseDependency(raw string) (string, string, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return "", "", errors.New("empty dependency declaration")
	}
	parts := strings.SplitN(raw, "@", 2)
	name, constraint := parts[0], "*"
	if len(parts) == 2 {
		constraint = strings.TrimSpace(parts[1])
	}
	if name == "" || constraint == "" {
		return "", "", fmt.Errorf("invalid dependency %q", raw)
	}
	return name, constraint, nil
}

func satisfies(version, constraint string) (bool, error) {
	if constraint == "" || constraint == "*" {
		_, err := parseVersion(version)
		return err == nil, err
	}
	op, wanted := "=", constraint
	for _, candidate := range []string{">=", "<=", ">", "<", "^", "="} {
		if strings.HasPrefix(constraint, candidate) {
			op, wanted = candidate, strings.TrimSpace(strings.TrimPrefix(constraint, candidate))
			break
		}
	}
	actualParts, err := parseVersion(version)
	if err != nil {
		return false, err
	}
	wantedParts, err := parseVersion(wanted)
	if err != nil {
		return false, err
	}
	cmp := compareVersion(actualParts, wantedParts)
	switch op {
	case ">=":
		return cmp >= 0, nil
	case "<=":
		return cmp <= 0, nil
	case ">":
		return cmp > 0, nil
	case "<":
		return cmp < 0, nil
	case "^":
		return cmp >= 0 && actualParts[0] == wantedParts[0], nil
	default:
		return cmp == 0, nil
	}
}

func parseVersion(raw string) ([3]int, error) {
	var result [3]int
	clean := strings.TrimPrefix(strings.SplitN(strings.TrimSpace(raw), "-", 2)[0], "v")
	parts := strings.Split(clean, ".")
	if len(parts) > 3 || clean == "" {
		return result, fmt.Errorf("invalid version %q", raw)
	}
	for index, part := range parts {
		value, err := strconv.Atoi(part)
		if err != nil || value < 0 {
			return result, fmt.Errorf("invalid version %q", raw)
		}
		result[index] = value
	}
	return result, nil
}

func compareVersion(left, right [3]int) int {
	for index := range left {
		if left[index] < right[index] {
			return -1
		}
		if left[index] > right[index] {
			return 1
		}
	}
	return 0
}

func unique(values []string) []string {
	seen := make(map[string]bool)
	result := make([]string, 0, len(values))
	for _, raw := range values {
		value := strings.TrimSpace(raw)
		if value != "" && !seen[value] {
			seen[value] = true
			result = append(result, value)
		}
	}
	sort.Strings(result)
	return result
}

func cloneMap(values map[string]string) map[string]string {
	if len(values) == 0 {
		return nil
	}
	result := make(map[string]string, len(values))
	for key, value := range values {
		result[strings.TrimSpace(key)] = strings.TrimSpace(value)
	}
	return result
}
