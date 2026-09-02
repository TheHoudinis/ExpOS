package catalog

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"hexaos.dev/ayo/internal/model"
)

func TestBuiltinBrowserResolvesCompleteFormPlan(t *testing.T) {
	plan, err := Builtin().Resolve("Browser", "Stable", model.State{})
	if err != nil {
		t.Fatal(err)
	}
	want := []string{"CoreTools", "Network", "Terminal", "Browser"}
	if len(plan) != len(want) {
		t.Fatalf("unexpected plan: %#v", plan)
	}
	for index := range want {
		if plan[index].Name != want[index] {
			t.Fatalf("plan[%d]=%s want %s", index, plan[index].Name, want[index])
		}
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
