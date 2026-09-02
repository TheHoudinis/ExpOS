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

const CurrentSchema uint32 = 2

const (
	Operator Authority = "operator"
	Power    Authority = "power"
	Guest    Authority = "guest"
)

// PackageForm is a persistent Form-native package record. Dependencies use
// Name@constraint syntax (for example Network@>=2.0.0), never filesystem paths.
type PackageForm struct {
	FIN           string            `json:"fin"`
	Name          string            `json:"name"`
	Version       string            `json:"version"`
	Dimension     string            `json:"dimension"`
	Active        bool              `json:"active"`
	DesiredActive bool              `json:"desired_active"`
	Hidden        bool              `json:"hidden"`
	Excluded      bool              `json:"excluded"`
	Revision      uint64            `json:"revision"`
	Capabilities  []string          `json:"capabilities,omitempty"`
	ProvidedForms []string          `json:"provided_forms,omitempty"`
	Dependencies  []string          `json:"dependencies,omitempty"`
	Compatibility []string          `json:"compatibility,omitempty"`
	PIMP          map[string]string `json:"pimp,omitempty"`
	MergedFrom    []string          `json:"merged_from,omitempty"`
	LastError     string            `json:"last_error,omitempty"`
	InstalledAt   time.Time         `json:"installed_at"`
	UpdatedAt     time.Time         `json:"updated_at"`
}

type JournalEntry struct {
	Sequence uint64    `json:"sequence"`
	Action   string    `json:"action"`
	FIN      string    `json:"fin,omitempty"`
	Detail   string    `json:"detail,omitempty"`
	At       time.Time `json:"at"`
}

type Manifest struct {
	Sequence uint64        `json:"sequence"`
	Label    string        `json:"label"`
	At       time.Time     `json:"at"`
	Forms    int           `json:"forms"`
	Checksum string        `json:"checksum"`
	Snapshot []PackageForm `json:"snapshot"`
}

type State struct {
	SchemaVersion uint32         `json:"schema_version"`
	Packages      []PackageForm  `json:"package_forms"`
	Journal       []JournalEntry `json:"journal"`
	Manifests     []Manifest     `json:"manifests"`
	Sequence      uint64         `json:"sequence"`
}

func NewFIN() (string, error) {
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", err
	}
	encoded := strings.ToUpper(hex.EncodeToString(raw[:]))
	return fmt.Sprintf("%s-%s-%s-%s-%s", encoded[0:8], encoded[8:12], encoded[12:16], encoded[16:20], encoded[20:32]), nil
}

func (state *State) Commit(action string, change func(next *State) (string, string, error)) error {
	next := state.Clone()
	next.SchemaVersion = CurrentSchema
	fin, detail, err := change(&next)
	if err != nil {
		return err
	}
	if err := next.Validate(); err != nil {
		return err
	}
	next.Sequence++
	next.Journal = append(next.Journal, JournalEntry{Sequence: next.Sequence, Action: action, FIN: fin, Detail: detail, At: time.Now().UTC()})
	*state = next
	return nil
}

// Migrate upgrades earlier development bridge records without changing FINs.
func (state *State) Migrate() error {
	if state.SchemaVersion > CurrentSchema {
		return fmt.Errorf("ayo state schema %d is newer than supported schema %d", state.SchemaVersion, CurrentSchema)
	}
	if state.SchemaVersion == 0 {
		for index := range state.Packages {
			state.Packages[index].DesiredActive = state.Packages[index].Active
		}
	}
	state.SchemaVersion = CurrentSchema
	return state.Validate()
}

func (state State) Clone() State {
	next := state
	next.Packages = clonePackages(state.Packages)
	next.Journal = append([]JournalEntry(nil), state.Journal...)
	next.Manifests = append([]Manifest(nil), state.Manifests...)
	for index := range next.Manifests {
		next.Manifests[index].Snapshot = clonePackages(state.Manifests[index].Snapshot)
	}
	return next
}

func clonePackages(forms []PackageForm) []PackageForm {
	next := append([]PackageForm(nil), forms...)
	for index := range next {
		next[index].Capabilities = append([]string(nil), forms[index].Capabilities...)
		next[index].ProvidedForms = append([]string(nil), forms[index].ProvidedForms...)
		next[index].Dependencies = append([]string(nil), forms[index].Dependencies...)
		next[index].Compatibility = append([]string(nil), forms[index].Compatibility...)
		next[index].MergedFrom = append([]string(nil), forms[index].MergedFrom...)
		if forms[index].PIMP != nil {
			next[index].PIMP = make(map[string]string, len(forms[index].PIMP))
			for key, value := range forms[index].PIMP {
				next[index].PIMP[key] = value
			}
		}
	}
	return next
}

func (state State) Validate() error {
	if state.SchemaVersion > CurrentSchema {
		return fmt.Errorf("unsupported ayo state schema %d", state.SchemaVersion)
	}
	seenFIN := make(map[string]bool)
	seenName := make(map[string]bool)
	for _, form := range state.Packages {
		if form.FIN == "" || form.Name == "" || form.Version == "" || form.Dimension == "" || form.Revision == 0 {
			return errors.New("invalid Package Form record")
		}
		if form.Active && (form.Hidden || form.Excluded) {
			return fmt.Errorf("Package Form %s has conflicting activation state", form.Name)
		}
		if seenFIN[form.FIN] {
			return fmt.Errorf("duplicate FIN %s", form.FIN)
		}
		identity := form.Dimension + "\x00" + strings.ToLower(form.Name)
		if seenName[identity] {
			return fmt.Errorf("duplicate Package Form %s in Dimension %s", form.Name, form.Dimension)
		}
		seenFIN[form.FIN], seenName[identity] = true, true
	}
	for index, entry := range state.Journal {
		if entry.Sequence == 0 || (index > 0 && entry.Sequence <= state.Journal[index-1].Sequence) {
			return errors.New("journal sequence is not monotonic")
		}
	}
	return nil
}

func (state *State) Find(identity, dimension string) (*PackageForm, error) {
	for index := range state.Packages {
		form := &state.Packages[index]
		if form.Dimension == dimension && (form.FIN == identity || strings.EqualFold(form.Name, identity)) {
			return form, nil
		}
	}
	return nil, fmt.Errorf("Package Form %q is not visible in Dimension %q", identity, dimension)
}

func (state *State) Sort() {
	sort.Slice(state.Packages, func(i, j int) bool {
		if state.Packages[i].Dimension != state.Packages[j].Dimension {
			return state.Packages[i].Dimension < state.Packages[j].Dimension
		}
		if state.Packages[i].Name == state.Packages[j].Name {
			return state.Packages[i].FIN < state.Packages[j].FIN
		}
		return state.Packages[i].Name < state.Packages[j].Name
	})
}
