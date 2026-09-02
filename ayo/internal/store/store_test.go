package store

import (
	"os"
	"path/filepath"
	"testing"

	"hexaos.dev/ayo/internal/model"
)

func validState(name string) model.State {
	return model.State{Packages: []model.PackageForm{{FIN: "FIN-" + name, Name: name, Version: "1.0.0", Dimension: "Stable", Active: true, DesiredActive: true, Revision: 1}}}
}

func TestJSONBridgeRecoversLastCommittedState(t *testing.T) {
	bridge := JSONBridge{Path: filepath.Join(t.TempDir(), "ayo.json")}
	if err := bridge.Save(validState("Core")); err != nil {
		t.Fatal(err)
	}
	if err := bridge.Save(validState("Shell")); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(bridge.Path, []byte("interrupted"), 0o600); err != nil {
		t.Fatal(err)
	}
	state, err := bridge.Load()
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Packages) != 1 || state.Packages[0].Name != "Core" {
		t.Fatalf("recovery snapshot not loaded: %#v", state)
	}
}

func TestJSONBridgeUpdateCommitsAtomically(t *testing.T) {
	bridge := JSONBridge{Path: filepath.Join(t.TempDir(), "ayo.json")}
	if err := bridge.Update(func(state *model.State) error { *state = validState("Core"); return nil }); err != nil {
		t.Fatal(err)
	}
	state, err := bridge.Load()
	if err != nil {
		t.Fatal(err)
	}
	if len(state.Packages) != 1 || state.Packages[0].Name != "Core" {
		t.Fatalf("update not committed: %#v", state)
	}
	if _, err := os.Stat(bridge.Path + ".journal"); !os.IsNotExist(err) {
		t.Fatalf("pending journal survived commit: %v", err)
	}
}

func TestLegacyStateMigrationPreservesActivation(t *testing.T) {
	bridge := JSONBridge{Path: filepath.Join(t.TempDir(), "ayo.json")}
	legacy := `{"package_forms":[{"fin":"OLD","name":"Core","version":"1.0.0","dimension":"Stable","active":true,"revision":1}],"journal":[],"manifests":[],"sequence":0}`
	if err := os.WriteFile(bridge.Path, []byte(legacy), 0o600); err != nil {
		t.Fatal(err)
	}
	state, err := bridge.Load()
	if err != nil {
		t.Fatal(err)
	}
	if state.SchemaVersion != model.CurrentSchema || !state.Packages[0].DesiredActive {
		t.Fatalf("legacy activation was not migrated: %#v", state)
	}
}
