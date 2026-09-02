package manager

import (
	"errors"
	"fmt"
	"sort"
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

func (manager Manager) Read() (model.State, error) { return manager.Store.Load() }

func (manager Manager) mutate(action string, operation func(*model.State) (string, error)) error {
	if manager.Authority != model.Operator {
		return fmt.Errorf("DIESE denied %s: Operator authority is required", action)
	}
	state, err := manager.Store.Load()
	if err != nil {
		return err
	}
	fin := ""
	err = state.Commit(action, fin, func(next *model.State) error {
		var operationError error
		fin, operationError = operation(next)
		return operationError
	})
	if err != nil {
		return err
	}
	if len(state.Journal) != 0 {
		state.Journal[len(state.Journal)-1].FIN = fin
	}
	return manager.Store.Save(state)
}

func (manager Manager) Slap(name, version string, capabilities, dependencies []string) error {
	if name == "" || version == "" {
		return errors.New("slap needs a Form name and version")
	}
	return manager.mutate("slap", func(state *model.State) (string, error) {
		if existing, err := state.Find(name, manager.Dimension); err == nil {
			if existing.Active {
				return existing.FIN, fmt.Errorf("%s is already active; chill, it is installed", name)
			}
			existing.Active, existing.Hidden = true, false
			existing.Revision++
			return existing.FIN, nil
		}
		fin, err := model.NewFIN()
		if err != nil {
			return "", err
		}
		state.Packages = append(state.Packages, model.PackageForm{FIN: fin, Name: name, Version: version, Dimension: manager.Dimension, Active: true, Revision: 1, Capabilities: unique(capabilities), Dependencies: unique(dependencies)})
		state.Sort()
		return fin, nil
	})
}

func (manager Manager) Yeet(identity string) error {
	return manager.update("yeet", identity, func(form *model.PackageForm) { form.Active = false; form.Hidden = false })
}

func (manager Manager) Ghost(identity string) error {
	return manager.update("ghost", identity, func(form *model.PackageForm) { form.Active = false; form.Hidden = true })
}

func (manager Manager) Dodge(identity string) error {
	return manager.update("dodge", identity, func(form *model.PackageForm) { form.Excluded = true })
}

func (manager Manager) update(action, identity string, change func(*model.PackageForm)) error {
	return manager.mutate(action, func(state *model.State) (string, error) {
		form, err := state.Find(identity, manager.Dimension)
		if err != nil {
			return "", err
		}
		change(form)
		form.Revision++
		return form.FIN, nil
	})
}

func (manager Manager) Highfive(leftIdentity, rightIdentity string) error {
	return manager.mutate("highfive", func(state *model.State) (string, error) {
		left, err := state.Find(leftIdentity, manager.Dimension)
		if err != nil {
			return "", err
		}
		right, err := state.Find(rightIdentity, manager.Dimension)
		if err != nil {
			return "", err
		}
		left.Capabilities = unique(append(left.Capabilities, right.Capabilities...))
		left.Revision++
		return left.FIN, nil
	})
}

func (manager Manager) Manifest() error {
	return manager.mutate("manifest", func(state *model.State) (string, error) {
		state.Manifests = append(state.Manifests, model.Manifest{Sequence: state.Sequence + 1, At: time.Now().UTC(), Forms: len(state.Packages)})
		return "", nil
	})
}

func (manager Manager) Chill() error {
	return manager.mutate("chill", func(state *model.State) (string, error) {
		for index := range state.Packages {
			form := &state.Packages[index]
			if form.Dimension == manager.Dimension && !form.Excluded && !form.Hidden {
				form.Active = true
			}
		}
		return "", nil
	})
}

func (manager Manager) Fix() error {
	return manager.mutate("fix", func(state *model.State) (string, error) {
		for index := range state.Packages {
			state.Packages[index].Capabilities = unique(state.Packages[index].Capabilities)
		}
		state.Sort()
		return "", state.Validate()
	})
}

func Vibecheck(state model.State) error {
	if err := state.Validate(); err != nil {
		return err
	}
	active := make(map[string]bool)
	for _, form := range state.Packages {
		if form.Active {
			active[form.Name] = true
		}
	}
	for _, form := range state.Packages {
		if !form.Active {
			continue
		}
		for _, dependency := range form.Dependencies {
			if !active[dependency] {
				return fmt.Errorf("%s misses active dependency %s", form.Name, dependency)
			}
		}
	}
	return nil
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
