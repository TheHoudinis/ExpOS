package manager

import (
	"testing"

	"hexaos.dev/ayo/internal/model"
)

type memoryStore struct{ state model.State }

func (store *memoryStore) Load() (model.State, error)   { return store.state.Clone(), nil }
func (store *memoryStore) Save(state model.State) error { store.state = state.Clone(); return nil }

func TestPackageLifecycleIsTransactionalAndDimensionScoped(t *testing.T) {
	storage := &memoryStore{}
	manager := Manager{Store: storage, Authority: model.Operator, Dimension: "Stable"}
	if err := manager.Slap("Browser", "1.0.0", []string{"network", "render"}, nil); err != nil {
		t.Fatal(err)
	}
	if err := manager.Ghost("Browser"); err != nil {
		t.Fatal(err)
	}
	state, _ := storage.Load()
	if len(state.Packages) != 1 || !state.Packages[0].Hidden || state.Packages[0].Active {
		t.Fatalf("unexpected state: %#v", state.Packages)
	}
	if len(state.Journal) != 2 || state.Sequence != 2 {
		t.Fatalf("journal did not commit atomically: %#v", state.Journal)
	}
}

func TestDIESERejectsMutationWithoutOperatorAuthority(t *testing.T) {
	storage := &memoryStore{}
	manager := Manager{Store: storage, Authority: model.Power, Dimension: "Stable"}
	if err := manager.Slap("Browser", "1.0.0", nil, nil); err == nil {
		t.Fatal("Power authority unexpectedly changed package state")
	}
}

func TestVibecheckExplainsMissingDependency(t *testing.T) {
	state := model.State{Packages: []model.PackageForm{{FIN: "A", Name: "Browser", Version: "1", Dimension: "Stable", Active: true, Revision: 1, Dependencies: []string{"Network"}}}}
	if err := Vibecheck(state); err == nil {
		t.Fatal("missing dependency was not detected")
	}
}
