package model

import (
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"sort"
	"strings"
	"time"
)

type Authority string

const (
	Operator Authority = "operator"
	Power    Authority = "power"
	Guest    Authority = "guest"
)

type PackageForm struct {
	FIN          string   `json:"fin"`
	Name         string   `json:"name"`
	Version      string   `json:"version"`
	Dimension    string   `json:"dimension"`
	Active       bool     `json:"active"`
	Hidden       bool     `json:"hidden"`
	Excluded     bool     `json:"excluded"`
	Revision     uint64   `json:"revision"`
	Capabilities []string `json:"capabilities,omitempty"`
	Dependencies []string `json:"dependencies,omitempty"`
}

type JournalEntry struct {
	Sequence uint64    `json:"sequence"`
	Action   string    `json:"action"`
	FIN      string    `json:"fin,omitempty"`
	At       time.Time `json:"at"`
}

type Manifest struct {
	Sequence uint64    `json:"sequence"`
	At       time.Time `json:"at"`
	Forms    int       `json:"forms"`
}

type State struct {
	Packages  []PackageForm  `json:"package_forms"`
	Journal   []JournalEntry `json:"journal"`
	Manifests []Manifest     `json:"manifests"`
	Sequence  uint64         `json:"sequence"`
}

func NewFIN() (string, error) {
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", err
	}
	encoded := strings.ToUpper(hex.EncodeToString(raw[:]))
	return fmt.Sprintf("%s-%s-%s-%s-%s", encoded[0:8], encoded[8:12], encoded[12:16], encoded[16:20], encoded[20:32]), nil
}

func (state *State) Commit(action, fin string, change func(next *State) error) error {
	next := state.Clone()
	if err := change(&next); err != nil {
		return err
	}
	if err := next.Validate(); err != nil {
		return err
	}
	next.Sequence++
	next.Journal = append(next.Journal, JournalEntry{Sequence: next.Sequence, Action: action, FIN: fin, At: time.Now().UTC()})
	*state = next
	return nil
}

func (state State) Clone() State {
	next := state
	next.Packages = append([]PackageForm(nil), state.Packages...)
	for index := range next.Packages {
		next.Packages[index].Capabilities = append([]string(nil), state.Packages[index].Capabilities...)
		next.Packages[index].Dependencies = append([]string(nil), state.Packages[index].Dependencies...)
	}
	next.Journal = append([]JournalEntry(nil), state.Journal...)
	next.Manifests = append([]Manifest(nil), state.Manifests...)
	return next
}

func (state State) Validate() error {
	seenFIN := make(map[string]bool)
	for _, form := range state.Packages {
		if form.FIN == "" || form.Name == "" || form.Dimension == "" || form.Revision == 0 {
			return errors.New("invalid Package Form record")
		}
		if seenFIN[form.FIN] {
			return fmt.Errorf("duplicate FIN %s", form.FIN)
		}
		seenFIN[form.FIN] = true
	}
	return nil
}

func (state *State) Find(identity, dimension string) (*PackageForm, error) {
	for index := range state.Packages {
		form := &state.Packages[index]
		if form.Dimension == dimension && (form.FIN == identity || form.Name == identity) {
			return form, nil
		}
	}
	return nil, fmt.Errorf("Package Form %q is not visible in Dimension %q", identity, dimension)
}

func (state *State) Sort() {
	sort.Slice(state.Packages, func(i, j int) bool {
		if state.Packages[i].Name == state.Packages[j].Name {
			return state.Packages[i].FIN < state.Packages[j].FIN
		}
		return state.Packages[i].Name < state.Packages[j].Name
	})
}
