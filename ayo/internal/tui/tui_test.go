package tui

import (
	"bytes"
	"strings"
	"testing"

	"hexaos.dev/ayo/internal/catalog"
	"hexaos.dev/ayo/internal/manager"
	"hexaos.dev/ayo/internal/model"
)

type memoryStore struct{ state model.State }

func (store *memoryStore) Load() (model.State, error)   { return store.state.Clone(), nil }
func (store *memoryStore) Save(state model.State) error { store.state = state.Clone(); return nil }

func TestTUIInstallsSelectionAndDependencies(t *testing.T) {
	storage := &memoryStore{}
	input := strings.NewReader("4\ny\nq\n")
	output := &bytes.Buffer{}
	app := App{Manager: manager.Manager{Store: storage, Authority: model.Operator, Dimension: "Stable"}, Catalog: catalog.Builtin(), Input: input, Output: output, Plain: true}
	if err := app.Run(); err != nil {
		t.Fatal(err)
	}
	if len(storage.state.Packages) != 4 || storage.state.Sequence != 1 {
		t.Fatalf("TUI plan did not commit atomically: %#v", storage.state)
	}
	if !strings.Contains(output.String(), "Installed Browser with 4 Form(s)") {
		t.Fatalf("missing success feedback: %s", output.String())
	}
}
