package store

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"

	"hexaos.dev/ayo/internal/model"
)

// Store is the userspace development bridge for HexaFS transactions. The
// kernel integration will implement the same contract with Form Handles.
type Store interface {
	Load() (model.State, error)
	Save(model.State) error
}

type JSONBridge struct{ Path string }

func (bridge JSONBridge) Load() (model.State, error) {
	raw, err := os.ReadFile(bridge.Path)
	if errors.Is(err, os.ErrNotExist) {
		return model.State{}, nil
	}
	if err != nil {
		return model.State{}, err
	}
	var state model.State
	if err := json.Unmarshal(raw, &state); err != nil {
		return model.State{}, err
	}
	return state, state.Validate()
}

func (bridge JSONBridge) Save(state model.State) error {
	if err := state.Validate(); err != nil {
		return err
	}
	raw, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(bridge.Path), 0o755); err != nil {
		return err
	}
	temporary := bridge.Path + ".next"
	if err := os.WriteFile(temporary, append(raw, '\n'), 0o600); err != nil {
		return err
	}
	return os.Rename(temporary, bridge.Path)
}
