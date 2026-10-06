package catalog

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"expos.dev/ayo/internal/model"
)

func TestBuiltinBrowserResolvesCompleteFormPlan(t *testing.T) {
	plan, err := Builtin().Resolve("Browser", "Stable", model.State{})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"CoreTools", "Network", "Terminal", "ExpDisplay", "Browser"}
	if len(plan) != len(want) {
		t.Fatalf("unexpected plan: %#v", plan)
	}
	for index := range want {
		if plan[index].Name != want[index] {
			t.Fatalf("plan[%d]=%s want %s", index, plan[index].Name, want[index])
		}
	}
}

func TestBuiltinSnakeResolvesPlayableGameStack(t *testing.T) {
	plan, err := Builtin().Resolve("Snake", "Stable", model.State{})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"CoreTools", "ExpDisplay", "InputKit", "GameHub", "Snake"}
	if len(plan) != len(want) {
		t.Fatalf("unexpected plan: %#v", plan)
	}
	for index := range want {
		if plan[index].Name != want[index] {
			t.Fatalf("plan[%d]=%s want %s", index, plan[index].Name, want[index])
		}
	}
}

func TestBuiltinCatalogHasEcosystemMetadataAndSearch(t *testing.T) {
	catalog := Builtin()
	if catalog.Schema != 3 || catalog.Trust != "built-in" {
		t.Fatalf("unexpected catalog identity: schema=%d trust=%q", catalog.Schema, catalog.Trust)
	}
	if len(catalog.Packages) < 57 {
		t.Fatalf("native ecosystem regressed to %d packages", len(catalog.Packages))
	}
	for _, name := range []string{"Calculator", "PixelPad", "NetScope", "MarkdownPad", "Weather", "Timer", "UUIDGen", "Breakout", "Memory"} {
		pkg, ok := catalog.Find(name)
		if !ok || len(pkg.ProvidedForms) == 0 || pkg.Artifact == nil {
			t.Fatalf("%s is not an installable Package Form: %#v", name, pkg)
		}
	}
	for _, pkg := range catalog.Packages {
		if pkg.Category == "" || len(pkg.Architectures) == 0 {
			t.Fatalf("%s lacks ecosystem metadata", pkg.Name)
		}
	}
	matches := catalog.Search("editor")
	if len(matches) < 2 || matches[0].Name != "ExpEdit" || matches[1].Name != "TextLab" {
		t.Fatalf("unexpected editor search: %#v", matches)
	}
}

func TestCatalogRejectsTamperedPackage(t *testing.T) {
	catalog := Builtin()
	catalog.Packages[0].Summary = "tampered"
	raw, err := json.Marshal(catalog)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(t.TempDir(), "catalog.json")
	if err := os.WriteFile(path, raw, 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := Load(context.Background(), path, ""); err == nil {
		t.Fatal("tampered Package Form was accepted")
	}
}

func TestCatalogEd25519Signature(t *testing.T) {
	publicKey, privateKey, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	catalog := Builtin()
	raw, err := json.Marshal(catalog)
	if err != nil {
		t.Fatal(err)
	}
	catalog.Signature = base64.StdEncoding.EncodeToString(ed25519.Sign(privateKey, raw))
	if err := verifySignature(catalog, base64.StdEncoding.EncodeToString(publicKey)); err != nil {
		t.Fatal(err)
	}
	catalog.Name = "tampered"
	if err := verifySignature(catalog, base64.StdEncoding.EncodeToString(publicKey)); err == nil {
		t.Fatal("tampered signed catalog was accepted")
	}
}

func TestRemoteCatalogRequiresPinnedKey(t *testing.T) {
	if err := requirePinnedRegistryKey(true, ""); err == nil {
		t.Fatal("remote catalog without a pinned key was accepted")
	}
	if err := requirePinnedRegistryKey(true, "  "); err == nil {
		t.Fatal("remote catalog with a blank pinned key was accepted")
	}
	if err := requirePinnedRegistryKey(true, "pinned-key"); err != nil {
		t.Fatalf("remote catalog with a key was rejected: %v", err)
	}
	if err := requirePinnedRegistryKey(false, ""); err != nil {
		t.Fatalf("local development catalog unexpectedly requires a key: %v", err)
	}
	if _, err := Load(context.Background(), "https://127.0.0.1:1/catalog.json", ""); err == nil ||
		!strings.Contains(err.Error(), "pinned Ed25519") {
		t.Fatalf("HTTPS source was contacted or accepted before key validation: %v", err)
	}
}
