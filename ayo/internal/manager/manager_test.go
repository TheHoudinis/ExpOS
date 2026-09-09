package manager

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"hexaos.dev/ayo/internal/artifact"
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

func TestVersionedDependenciesAndDependentProtection(t *testing.T) {
	storage := &memoryStore{}
	ayo := Manager{Store: storage, Authority: model.Operator, Dimension: "Stable"}
	if err := ayo.Slap("Network", "2.1.0", []string{"socket"}, nil); err != nil {
		t.Fatal(err)
	}
	if err := ayo.Slap("Browser", "1.0.0", []string{"render"}, []string{"Network@>=2.0.0"}); err != nil {
		t.Fatal(err)
	}
	if err := ayo.Yeet("Network"); err == nil || !strings.Contains(err.Error(), "Browser") {
		t.Fatalf("expected dependent protection, got %v", err)
	}
	if err := ayo.YeetForce("Network", true); err != nil {
		t.Fatal(err)
	}
	state, _ := storage.Load()
	browser, _ := state.Find("Browser", "Stable")
	if browser.Active || browser.LastError == "" {
		t.Fatalf("dependent was not reconciled off: %#v", browser)
	}
}

func TestSlapRejectsUnsatisfiedConstraintWithoutCommitting(t *testing.T) {
	storage := &memoryStore{}
	ayo := Manager{Store: storage, Authority: model.Operator, Dimension: "Stable"}
	if err := ayo.Slap("Core", "1.5.0", nil, nil); err != nil {
		t.Fatal(err)
	}
	if err := ayo.Slap("UI", "1.0.0", nil, []string{"Core@^2.0.0"}); err == nil {
		t.Fatal("incompatible dependency was accepted")
	}
	state, _ := storage.Load()
	if len(state.Packages) != 1 || state.Sequence != 1 {
		t.Fatalf("failed transaction leaked state: %#v", state)
	}
}

func TestManifestAndHighfivePreserveFormHistory(t *testing.T) {
	storage := &memoryStore{}
	ayo := Manager{Store: storage, Authority: model.Operator, Dimension: "Stable"}
	if err := ayo.SlapSpec(InstallSpec{Name: "Shell", Version: "1.0.0", Capabilities: []string{"execute"}, ProvidedForms: []string{"Terminal"}}); err != nil {
		t.Fatal(err)
	}
	if err := ayo.SlapSpec(InstallSpec{Name: "Theme", Version: "1.0.0", Capabilities: []string{"render"}, ProvidedForms: []string{"Palette"}}); err != nil {
		t.Fatal(err)
	}
	if err := ayo.Highfive("Shell", "Theme"); err != nil {
		t.Fatal(err)
	}
	if err := ayo.ManifestNamed("known-good"); err != nil {
		t.Fatal(err)
	}
	state, _ := storage.Load()
	shell, _ := state.Find("Shell", "Stable")
	if len(shell.MergedFrom) != 1 || len(shell.Capabilities) != 2 || len(shell.ProvidedForms) != 2 {
		t.Fatalf("controlled merge is incomplete: %#v", shell)
	}
	if len(state.Manifests) != 1 || state.Manifests[0].Checksum == "" || len(state.Manifests[0].Snapshot) != 2 {
		t.Fatalf("checkpoint is incomplete: %#v", state.Manifests)
	}
	state.Manifests[0].Snapshot[0].Version = "corrupted"
	if report := InspectHealth(state, "Stable"); len(report.Issues) == 0 {
		t.Fatal("manifest corruption was not detected")
	}
}

func TestCompatibilityAndPIMPPolicyAreEnforced(t *testing.T) {
	storage := &memoryStore{}
	ayo := Manager{Store: storage, Authority: model.Operator, Dimension: "Stable"}
	if err := ayo.SlapSpec(InstallSpec{Name: "Future", Version: "1.0.0", Compatibility: []string{"hexaos>=9.0.0"}}); err == nil {
		t.Fatal("incompatible HexaOS requirement was accepted")
	}
	if err := ayo.SlapSpec(InstallSpec{Name: "Denied", Version: "1.0.0", PIMP: map[string]string{"activation": "denied"}}); err == nil {
		t.Fatal("PIMP activation denial was ignored")
	}
	if err := ayo.SlapSpec(InstallSpec{Name: "Native", Version: "1.0.0", Compatibility: []string{"dimension=Stable", "hexaos>=8.0.0"}}); err != nil {
		t.Fatal(err)
	}
}

func TestInstallPlanResolvesUnorderedDependenciesAtomically(t *testing.T) {
	storage := &memoryStore{}
	ayo := Manager{Store: storage, Authority: model.Operator, Dimension: "Stable"}
	plan := []InstallSpec{
		{Name: "Browser", Version: "1.0.0", Dependencies: []string{"Network@>=2.0.0"}},
		{Name: "Network", Version: "2.1.0"},
	}
	if err := ayo.InstallPlan(plan); err != nil {
		t.Fatal(err)
	}
	state, _ := storage.Load()
	if len(state.Packages) != 2 || state.Sequence != 1 {
		t.Fatalf("plan was not one transaction: %#v", state)
	}
	broken := []InstallSpec{{Name: "BadUI", Version: "1.0.0", Dependencies: []string{"Missing@>=1.0.0"}}}
	if err := ayo.InstallPlan(broken); err == nil {
		t.Fatal("unresolved plan was committed")
	}
	state, _ = storage.Load()
	if len(state.Packages) != 2 || state.Sequence != 1 {
		t.Fatalf("failed plan leaked state: %#v", state)
	}
}

func TestArtifactPlanPersistsOwnershipAndYeetRemovesFiles(t *testing.T) {
	storage := &memoryStore{}
	root := t.TempDir()
	source := filepath.Join(t.TempDir(), "notes.bin")
	payload := []byte("actual package bytes")
	if err := os.WriteFile(source, payload, 0o600); err != nil {
		t.Fatal(err)
	}
	ayo := Manager{Store: storage, Authority: model.Operator, Dimension: "Stable", InstallRoot: root}
	spec := InstallSpec{
		Name: "Notes", Version: "3.0.0",
		Artifact: &model.ArtifactSpec{Source: source, SHA256: artifact.Digest(payload), Format: "raw", Target: "share/notes/data"},
	}
	if err := ayo.InstallPlan([]InstallSpec{spec}); err != nil {
		t.Fatal(err)
	}
	state, _ := storage.Load()
	form, err := state.Find("Notes", "Stable")
	if err != nil || form.Artifact == nil || len(form.Artifact.Files) != 1 {
		t.Fatalf("artifact ownership was not committed: %#v, %v", form, err)
	}
	if content, err := os.ReadFile(filepath.Join(root, "share", "notes", "data")); err != nil || string(content) != string(payload) {
		t.Fatalf("artifact payload missing: %q, %v", content, err)
	}
	if err := ayo.Yeet("Notes"); err != nil {
		t.Fatal(err)
	}
	state, _ = storage.Load()
	form, _ = state.Find("Notes", "Stable")
	if form.Active || form.Artifact != nil {
		t.Fatalf("uninstall did not clear activation receipt: %#v", form)
	}
	if _, err := os.Stat(filepath.Join(root, "share", "notes", "data")); !os.IsNotExist(err) {
		t.Fatalf("owned payload survived yeet: %v", err)
	}
}
