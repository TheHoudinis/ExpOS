package artifact

import (
	"archive/tar"
	"bytes"
	"context"
	"errors"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"expos.dev/ayo/internal/model"
)

func TestHTTPSVerifiedInstallOwnershipAndUninstall(t *testing.T) {
	payload := []byte("#!/bin/sh\necho real-artifact\n")
	client := &http.Client{Transport: roundTripFunc(func(request *http.Request) (*http.Response, error) {
		if request.URL.Scheme != "https" || request.URL.Host != "packages.example" || request.URL.Path != "/demo" {
			return nil, errors.New("unexpected artifact request")
		}
		return &http.Response{
			StatusCode: http.StatusOK,
			Status:     "200 OK",
			Header:     make(http.Header),
			Body:       io.NopCloser(bytes.NewReader(payload)),
			Request:    request,
		}, nil
	})}

	root := t.TempDir()
	installer := Installer{Root: root, Client: client}
	state := model.State{SchemaVersion: model.CurrentSchema}
	spec := model.ArtifactSpec{Source: "https://packages.example/demo", SHA256: Digest(payload), Format: "raw", Target: "bin/demo", Mode: 0o755}
	var receipt *model.InstalledArtifact
	var transaction string
	err := installer.Install(context.Background(), []Package{{Name: "Demo", Version: "1.0.0", Artifact: spec}}, state, "Stable", func(id string, receipts map[string]*model.InstalledArtifact) error {
		transaction = id
		receipt = receipts["Demo"]
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	installed, err := os.ReadFile(filepath.Join(root, "bin", "demo"))
	if err != nil || !bytes.Equal(installed, payload) {
		t.Fatalf("verified payload was not installed: %q, %v", installed, err)
	}
	if receipt == nil || len(receipt.Files) != 1 || receipt.Files[0].Path != "bin/demo" || receipt.Files[0].SHA256 != Digest(payload) {
		t.Fatalf("ownership receipt is incomplete: %#v", receipt)
	}
	info, err := os.Stat(filepath.Join(root, "bin", "demo"))
	if err != nil || info.Mode().Perm() != 0o755 {
		t.Fatalf("safe executable mode not installed: %v, %v", info, err)
	}

	state = installedState("Demo", *receipt)
	state.ArtifactTransaction = transaction
	if err := installer.Remove(state, "Stable", "Demo", *receipt, func(string) error { return nil }); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(root, "bin", "demo")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("owned file survived uninstall: %v", err)
	}
}

type roundTripFunc func(*http.Request) (*http.Response, error)

func (roundTrip roundTripFunc) RoundTrip(request *http.Request) (*http.Response, error) {
	return roundTrip(request)
}

func TestChecksumFailureLeavesNoPayloadOrStateCommit(t *testing.T) {
	root := t.TempDir()
	source := filepath.Join(t.TempDir(), "payload")
	if err := os.WriteFile(source, []byte("not trusted"), 0o600); err != nil {
		t.Fatal(err)
	}
	committed := false
	err := (Installer{Root: root}).Install(context.Background(), []Package{{
		Name: "Bad", Version: "1.0.0",
		Artifact: model.ArtifactSpec{Source: source, SHA256: strings.Repeat("0", 64), Format: "raw", Target: "bin/bad"},
	}}, model.State{SchemaVersion: model.CurrentSchema}, "Stable", func(string, map[string]*model.InstalledArtifact) error {
		committed = true
		return nil
	})
	if err == nil || !strings.Contains(err.Error(), "SHA-256") {
		t.Fatalf("checksum mismatch was accepted: %v", err)
	}
	if committed {
		t.Fatal("state callback ran after checksum failure")
	}
	if _, err := os.Stat(filepath.Join(root, "bin", "bad")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("checksum failure leaked payload: %v", err)
	}
}

func TestArchiveRejectsTraversalAndLinks(t *testing.T) {
	for _, test := range []struct {
		name     string
		entry    string
		typeflag byte
	}{
		{name: "traversal", entry: "../escape", typeflag: tar.TypeReg},
		{name: "absolute", entry: "/escape", typeflag: tar.TypeReg},
		{name: "symlink", entry: "bin/link", typeflag: tar.TypeSymlink},
		{name: "hardlink", entry: "bin/link", typeflag: tar.TypeLink},
	} {
		t.Run(test.name, func(t *testing.T) {
			payload := tarPayload(t, test.entry, test.typeflag, []byte("blocked"))
			source := filepath.Join(t.TempDir(), "bad.tar")
			if err := os.WriteFile(source, payload, 0o600); err != nil {
				t.Fatal(err)
			}
			err := (Installer{Root: t.TempDir()}).Install(context.Background(), []Package{{
				Name: "Unsafe", Version: "1.0.0",
				Artifact: model.ArtifactSpec{Source: source, SHA256: Digest(payload), Format: "tar"},
			}}, model.State{SchemaVersion: model.CurrentSchema}, "Stable", func(string, map[string]*model.InstalledArtifact) error { return nil })
			if err == nil {
				t.Fatal("unsafe archive was accepted")
			}
		})
	}
}

func TestStateFailureRollsFilesystemBack(t *testing.T) {
	root := t.TempDir()
	payload := []byte("staged")
	source := filepath.Join(t.TempDir(), "payload")
	if err := os.WriteFile(source, payload, 0o600); err != nil {
		t.Fatal(err)
	}
	err := (Installer{Root: root}).Install(context.Background(), []Package{{
		Name: "Rollback", Version: "1.0.0",
		Artifact: model.ArtifactSpec{Source: source, SHA256: Digest(payload), Format: "raw", Target: "share/rollback"},
	}}, model.State{SchemaVersion: model.CurrentSchema}, "Stable", func(string, map[string]*model.InstalledArtifact) error {
		return errors.New("simulated state write failure")
	})
	if err == nil || !strings.Contains(err.Error(), "rolled back") {
		t.Fatalf("missing rollback diagnostic: %v", err)
	}
	if _, err := os.Stat(filepath.Join(root, "share", "rollback")); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("failed transaction leaked installed file: %v", err)
	}
}

func TestInterruptedTransactionRecoveryRestoresBackup(t *testing.T) {
	root := t.TempDir()
	target := filepath.Join(root, "bin", "tool")
	if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(target, []byte("new"), 0o644); err != nil {
		t.Fatal(err)
	}
	txRoot := filepath.Join(root, ".ayo", "transactions", "interrupted")
	if err := os.MkdirAll(filepath.Join(txRoot, "backup"), 0o700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(txRoot, "backup", "000000"), []byte("old"), 0o644); err != nil {
		t.Fatal(err)
	}
	tx := transaction{
		ID: "interrupted", Phase: "applied", Created: time.Unix(1, 0).UTC(),
		Entries: []txEntry{{Path: "bin/tool", Stage: filepath.Join(txRoot, "payload", "000000"), Backup: filepath.Join("backup", "000000"), NewSHA256: Digest([]byte("new")), HadOriginal: true}},
	}
	if err := writeTransaction(txRoot, tx); err != nil {
		t.Fatal(err)
	}
	if err := (Installer{Root: root}).Recover(""); err != nil {
		t.Fatal(err)
	}
	recovered, err := os.ReadFile(target)
	if err != nil || string(recovered) != "old" {
		t.Fatalf("recovery did not restore backup: %q, %v", recovered, err)
	}
	if _, err := os.Stat(txRoot); !errors.Is(err, os.ErrNotExist) {
		t.Fatalf("recovered journal was not cleaned: %v", err)
	}
}

func TestOwnershipPreventsCrossPackageOverwrite(t *testing.T) {
	root := t.TempDir()
	target := filepath.Join(root, "bin", "owned")
	if err := os.MkdirAll(filepath.Dir(target), 0o755); err != nil {
		t.Fatal(err)
	}
	old := []byte("owner-a")
	if err := os.WriteFile(target, old, 0o644); err != nil {
		t.Fatal(err)
	}
	receipt := model.InstalledArtifact{Source: "file:///old", SHA256: Digest(old), Format: "raw", Files: []model.OwnedFile{{Path: "bin/owned", SHA256: Digest(old), Mode: 0o644, Size: int64(len(old))}}}
	state := installedState("OwnerA", receipt)
	next := []byte("owner-b")
	source := filepath.Join(t.TempDir(), "next")
	if err := os.WriteFile(source, next, 0o600); err != nil {
		t.Fatal(err)
	}
	err := (Installer{Root: root}).Install(context.Background(), []Package{{
		Name: "OwnerB", Version: "1.0.0",
		Artifact: model.ArtifactSpec{Source: source, SHA256: Digest(next), Format: "raw", Target: "bin/owned"},
	}}, state, "Stable", func(string, map[string]*model.InstalledArtifact) error { return nil })
	if err == nil || !strings.Contains(err.Error(), "already owned") {
		t.Fatalf("ownership collision was accepted: %v", err)
	}
	content, _ := os.ReadFile(target)
	if !bytes.Equal(content, old) {
		t.Fatalf("ownership collision changed file: %q", content)
	}
}

func installedState(name string, receipt model.InstalledArtifact) model.State {
	return model.State{SchemaVersion: model.CurrentSchema, Packages: []model.PackageForm{{
		FIN: "FIN-" + name, Name: name, Version: "1.0.0", Dimension: "Stable",
		Active: true, DesiredActive: true, Revision: 1, Artifact: &receipt,
	}}}
}

func tarPayload(t *testing.T, name string, typeflag byte, content []byte) []byte {
	t.Helper()
	var buffer bytes.Buffer
	writer := tar.NewWriter(&buffer)
	header := &tar.Header{Name: name, Typeflag: typeflag, Mode: 0o755}
	if typeflag == tar.TypeReg {
		header.Size = int64(len(content))
	}
	if typeflag == tar.TypeSymlink || typeflag == tar.TypeLink {
		header.Linkname = "../../escape"
	}
	if err := writer.WriteHeader(header); err != nil {
		t.Fatal(err)
	}
	if typeflag == tar.TypeReg {
		if _, err := writer.Write(content); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return buffer.Bytes()
}
