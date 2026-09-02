package store

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"syscall"

	"hexaos.dev/ayo/internal/model"
)

// Store is the development HexaFS bridge contract. AtomicStore mirrors one
// serialized HexaFS transaction and prevents lost concurrent updates.
type Store interface {
	Load() (model.State, error)
	Save(model.State) error
}

type AtomicStore interface {
	Update(func(*model.State) error) error
}

type JSONBridge struct{ Path string }

func (bridge JSONBridge) Load() (model.State, error) {
	state, err := bridge.loadPath(bridge.Path)
	if err == nil {
		return state, nil
	}
	recovered, recoveryErr := bridge.loadPath(bridge.Path + ".recovery")
	if recoveryErr == nil {
		return recovered, nil
	}
	pending, pendingErr := bridge.loadPath(bridge.Path + ".journal")
	if pendingErr == nil {
		return pending, nil
	}
	if errors.Is(err, os.ErrNotExist) {
		return model.State{SchemaVersion: model.CurrentSchema}, nil
	}
	return model.State{}, fmt.Errorf("HexaFS bridge state is invalid (%v); recovery failed (%v); journal replay failed (%v)", err, recoveryErr, pendingErr)
}

func (bridge JSONBridge) Save(state model.State) error {
	release, err := bridge.lock()
	if err != nil {
		return err
	}
	defer release()
	return bridge.saveUnlocked(state)
}

func (bridge JSONBridge) Update(change func(*model.State) error) error {
	release, err := bridge.lock()
	if err != nil {
		return err
	}
	defer release()
	state, err := bridge.Load()
	if err != nil {
		return err
	}
	if err := change(&state); err != nil {
		return err
	}
	return bridge.saveUnlocked(state)
}

func (bridge JSONBridge) loadPath(path string) (model.State, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return model.State{}, err
	}
	var state model.State
	if err := json.Unmarshal(raw, &state); err != nil {
		return model.State{}, err
	}
	return state, state.Migrate()
}

func (bridge JSONBridge) saveUnlocked(state model.State) error {
	if err := state.Migrate(); err != nil {
		return err
	}
	raw, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	directory := filepath.Dir(bridge.Path)
	if err := os.MkdirAll(directory, 0o755); err != nil {
		return err
	}
	if current, err := bridge.loadPath(bridge.Path); err == nil {
		currentRaw, marshalErr := json.MarshalIndent(current, "", "  ")
		if marshalErr != nil {
			return marshalErr
		}
		if err := os.WriteFile(bridge.Path+".recovery", append(currentRaw, '\n'), 0o600); err != nil {
			return err
		}
	}
	pending := bridge.Path + ".journal"
	if err := os.WriteFile(pending, append(raw, '\n'), 0o600); err != nil {
		return err
	}
	file, err := os.OpenFile(bridge.Path+".next", os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	if _, err = file.Write(append(raw, '\n')); err == nil {
		err = file.Sync()
	}
	if closeErr := file.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	if err := os.Rename(bridge.Path+".next", bridge.Path); err != nil {
		return err
	}
	if directoryHandle, err := os.Open(directory); err == nil {
		_ = directoryHandle.Sync()
		_ = directoryHandle.Close()
	}
	return os.Remove(pending)
}

func (bridge JSONBridge) lock() (func(), error) {
	if err := os.MkdirAll(filepath.Dir(bridge.Path), 0o755); err != nil {
		return nil, err
	}
	lockPath := bridge.Path + ".lock"
	file, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(file.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		_ = file.Close()
		return nil, errors.New("another ayo transaction is active; chill and retry")
	}
	return func() {
		_ = syscall.Flock(int(file.Fd()), syscall.LOCK_UN)
		_ = file.Close()
	}, nil
}
