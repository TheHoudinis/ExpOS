package catalog

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"

	"hexaos.dev/ayo/internal/manager"
	"hexaos.dev/ayo/internal/model"
)

const maxCatalogBytes = 2 << 20

type Package struct {
	Name          string            `json:"name"`
	Version       string            `json:"version"`
	Summary       string            `json:"summary"`
	Capabilities  []string          `json:"capabilities,omitempty"`
	ProvidedForms []string          `json:"provided_forms,omitempty"`
	Dependencies  []string          `json:"dependencies,omitempty"`
	Compatibility []string          `json:"compatibility,omitempty"`
	PIMP          map[string]string `json:"pimp,omitempty"`
	Checksum      string            `json:"checksum"`
}

type Catalog struct {
	Schema      uint32    `json:"schema"`
	Name        string    `json:"name"`
	GeneratedAt time.Time `json:"generated_at"`
	Packages    []Package `json:"packages"`
	Signature   string    `json:"signature,omitempty"`
}

func Builtin() Catalog {
	catalog := Catalog{Schema: 1, Name: "ExpOS Prism Forms", GeneratedAt: time.Date(2026, 9, 8, 0, 0, 0, 0, time.UTC), Packages: []Package{
		{Name: "CoreTools", Version: "1.0.0", Summary: "Form-native diagnostics and repair tools", Capabilities: []string{"inspect", "repair"}, ProvidedForms: []string{"Diagnostics"}, Compatibility: []string{"hexaos>=8.0.0"}},
		{Name: "Network", Version: "2.1.0", Summary: "Network service Form and socket capability", Capabilities: []string{"network", "socket"}, ProvidedForms: []string{"NetworkService"}, Dependencies: []string{"CoreTools@>=1.0.0"}},
		{Name: "Terminal", Version: "1.2.0", Summary: "Interactive command Interface Form", Capabilities: []string{"execute", "render"}, ProvidedForms: []string{"TerminalInterface"}, Dependencies: []string{"CoreTools@>=1.0.0"}},
		{Name: "Browser", Version: "1.0.0", Summary: "Web Interface Form for the Network service", Capabilities: []string{"network", "render"}, ProvidedForms: []string{"BrowserInterface"}, Dependencies: []string{"Network@>=2.0.0", "Terminal@>=1.0.0", "HexaDisplay@>=1.0.0"}},
		{Name: "HexaEdit", Version: "0.8.0", Summary: "Revision-aware Data Form editor", Capabilities: []string{"read", "configure"}, ProvidedForms: []string{"EditorInterface"}, Dependencies: []string{"Terminal@>=1.0.0"}},
		{Name: "SystemScope", Version: "1.0.0", Summary: "Live Forms, Handles, and relationship viewer", Capabilities: []string{"inspect", "relate"}, ProvidedForms: []string{"SystemScopeInterface"}, Dependencies: []string{"Terminal@>=1.0.0"}},
		{Name: "HexaDisplay", Version: "1.0.0", Summary: "Native surface composition and input service", Capabilities: []string{"render", "relate"}, ProvidedForms: []string{"DisplayService"}, Dependencies: []string{"CoreTools@>=1.0.0"}},
		{Name: "RenderKit", Version: "1.0.0", Summary: "Dark UI gradients alpha blending rounded shapes and lines", Capabilities: []string{"render"}, ProvidedForms: []string{"GraphicsPrimitives"}, Dependencies: []string{"HexaDisplay@>=1.0.0"}},
		{Name: "MouseKit", Version: "1.0.0", Summary: "PS2 pointer packets cursor and surface hit testing", Capabilities: []string{"input", "inspect"}, ProvidedForms: []string{"PointerService"}, Dependencies: []string{"InputKit@>=1.0.0", "HexaDisplay@>=1.0.0"}},
		{Name: "SessionManager", Version: "1.0.0", Summary: "Login identity and capability authority sessions", Capabilities: []string{"inspect", "configure"}, ProvidedForms: []string{"SessionService"}, Dependencies: []string{"CoreTools@>=1.0.0"}},
		{Name: "GoSDK", Version: "1.0.0", Summary: "HexaOS Go ABI bindings and Form test emulator", Capabilities: []string{"execute", "relate"}, ProvidedForms: []string{"GoABIClient"}, Dependencies: []string{"CoreTools@>=1.0.0"}},
		{Name: "PrismDE", Version: "1.2.0", Summary: "Dark pointer-driven capability-native desktop environment", Capabilities: []string{"render", "input", "configure"}, ProvidedForms: []string{"PrismDesktop"}, Dependencies: []string{"HexaDisplay@>=1.0.0", "RenderKit@>=1.0.0", "Terminal@>=1.2.0", "MouseKit@>=1.0.0", "SessionManager@>=1.0.0"}},
		{Name: "PrismTheme", Version: "1.1.0", Summary: "Black Prism desktop palette and original chrome", Capabilities: []string{"render"}, ProvidedForms: []string{"PrismPalette"}, Dependencies: []string{"PrismDE@>=1.2.0"}},
		{Name: "InputKit", Version: "1.0.0", Summary: "PS2 and serial key event adapters", Capabilities: []string{"input", "inspect"}, ProvidedForms: []string{"InputService"}, Dependencies: []string{"CoreTools@>=1.0.0"}},
		{Name: "GameHub", Version: "1.0.0", Summary: "Native arcade launcher and game runtime", Capabilities: []string{"render", "input", "execute"}, ProvidedForms: []string{"GameHubInterface"}, Dependencies: []string{"HexaDisplay@>=1.0.0", "InputKit@>=1.0.0"}},
		{Name: "Snake", Version: "1.0.0", Summary: "Signal Garden native snake game", Capabilities: []string{"render", "input"}, ProvidedForms: []string{"SnakeGame"}, Dependencies: []string{"GameHub@>=1.0.0"}},
		{Name: "Pong", Version: "1.0.0", Summary: "Form Duel native pong game", Capabilities: []string{"render", "input"}, ProvidedForms: []string{"PongGame"}, Dependencies: []string{"GameHub@>=1.0.0"}},
		{Name: "VirtioBlock", Version: "0.4.0", Summary: "Experimental virtio block Driver Form", Capabilities: []string{"read", "write", "inspect"}, ProvidedForms: []string{"BlockDriver"}, Dependencies: []string{"CoreTools@>=1.0.0"}},
		{Name: "AudioKit", Version: "0.3.0", Summary: "Experimental audio service interfaces", Capabilities: []string{"read", "configure"}, ProvidedForms: []string{"AudioService"}, Dependencies: []string{"CoreTools@>=1.0.0"}},
		{Name: "TextLab", Version: "1.0.0", Summary: "Form-native notes and text workspace", Capabilities: []string{"read", "configure"}, ProvidedForms: []string{"TextLabInterface"}, Dependencies: []string{"HexaEdit@>=0.8.0", "PrismDE@>=1.0.0"}},
		{Name: "DeveloperKit", Version: "1.0.0", Summary: "Go SDK terminal and editor development deck", Capabilities: []string{"execute", "read", "configure"}, ProvidedForms: []string{"DeveloperWorkspace"}, Dependencies: []string{"GoSDK@>=1.0.0", "HexaEdit@>=0.8.0", "PrismDE@>=1.0.0"}},
	}}
	for index := range catalog.Packages {
		catalog.Packages[index].Checksum = packageChecksum(catalog.Packages[index])
	}
	return catalog
}

func Load(ctx context.Context, source, publicKey string) (Catalog, error) {
	if strings.TrimSpace(source) == "" {
		return Builtin(), nil
	}
	raw, remote, err := readSource(ctx, source)
	if err != nil {
		return Catalog{}, err
	}
	var catalog Catalog
	if err := json.Unmarshal(raw, &catalog); err != nil {
		return Catalog{}, fmt.Errorf("registry catalog is not valid JSON: %w", err)
	}
	if err := catalog.Validate(remote); err != nil {
		return Catalog{}, err
	}
	if publicKey != "" {
		if err := verifySignature(catalog, publicKey); err != nil {
			return Catalog{}, err
		}
	}
	return catalog, nil
}

func (catalog Catalog) Validate(requireChecksums bool) error {
	if catalog.Schema != 1 || catalog.Name == "" {
		return errors.New("unsupported or unnamed ayo registry catalog")
	}
	seen := make(map[string]bool)
	for _, pkg := range catalog.Packages {
		if pkg.Name == "" || pkg.Version == "" || pkg.Summary == "" {
			return errors.New("registry contains an incomplete Package Form")
		}
		key := strings.ToLower(pkg.Name)
		if seen[key] {
			return fmt.Errorf("registry contains duplicate Package Form %s", pkg.Name)
		}
		seen[key] = true
		if requireChecksums && pkg.Checksum == "" {
			return fmt.Errorf("remote Package Form %s has no checksum", pkg.Name)
		}
		if pkg.Checksum != "" && !strings.EqualFold(pkg.Checksum, packageChecksum(pkg)) {
			return fmt.Errorf("Package Form %s failed checksum validation", pkg.Name)
		}
	}
	return nil
}

func (catalog Catalog) Find(name string) (Package, bool) {
	for _, pkg := range catalog.Packages {
		if strings.EqualFold(pkg.Name, name) {
			return pkg, true
		}
	}
	return Package{}, false
}

func (catalog Catalog) Resolve(name, dimension string, state model.State) ([]manager.InstallSpec, error) {
	visiting, resolved := make(map[string]bool), make(map[string]bool)
	plan := make([]manager.InstallSpec, 0)
	var visit func(string, string) error
	visit = func(packageName, constraint string) error {
		if installed, err := state.Find(packageName, dimension); err == nil && installed.Active {
			ok, versionErr := manager.VersionSatisfies(installed.Version, constraint)
			if versionErr != nil {
				return versionErr
			}
			if ok {
				return nil
			}
		}
		pkg, ok := catalog.Find(packageName)
		if !ok {
			return fmt.Errorf("registry has no Package Form %s", packageName)
		}
		matches, err := manager.VersionSatisfies(pkg.Version, constraint)
		if err != nil {
			return err
		}
		if !matches {
			return fmt.Errorf("registry has %s@%s but %s is required", pkg.Name, pkg.Version, constraint)
		}
		key := strings.ToLower(pkg.Name)
		if visiting[key] {
			return fmt.Errorf("dependency cycle includes %s", pkg.Name)
		}
		if resolved[key] {
			return nil
		}
		visiting[key] = true
		for _, raw := range pkg.Dependencies {
			dependency, requirement := splitDependency(raw)
			if err := visit(dependency, requirement); err != nil {
				return err
			}
		}
		delete(visiting, key)
		resolved[key] = true
		plan = append(plan, pkg.InstallSpec())
		return nil
	}
	if err := visit(name, "*"); err != nil {
		return nil, err
	}
	return plan, nil
}

func (pkg Package) InstallSpec() manager.InstallSpec {
	return manager.InstallSpec{Name: pkg.Name, Version: pkg.Version, Capabilities: pkg.Capabilities, ProvidedForms: pkg.ProvidedForms, Dependencies: pkg.Dependencies, Compatibility: pkg.Compatibility, PIMP: pkg.PIMP}
}

func readSource(ctx context.Context, source string) ([]byte, bool, error) {
	parsed, err := url.Parse(source)
	if err == nil && (parsed.Scheme == "https" || parsed.Scheme == "http") {
		if parsed.Scheme != "https" {
			return nil, true, errors.New("remote ayo registries must use HTTPS")
		}
		request, err := http.NewRequestWithContext(ctx, http.MethodGet, source, nil)
		if err != nil {
			return nil, true, err
		}
		client := &http.Client{Timeout: 15 * time.Second}
		response, err := client.Do(request)
		if err != nil {
			return nil, true, fmt.Errorf("registry download failed: %w", err)
		}
		defer response.Body.Close()
		if response.StatusCode != http.StatusOK {
			return nil, true, fmt.Errorf("registry returned %s", response.Status)
		}
		raw, err := io.ReadAll(io.LimitReader(response.Body, maxCatalogBytes+1))
		if err != nil {
			return nil, true, err
		}
		if len(raw) > maxCatalogBytes {
			return nil, true, errors.New("registry catalog exceeds 2 MiB limit")
		}
		return raw, true, nil
	}
	raw, err := os.ReadFile(source)
	return raw, false, err
}

func packageChecksum(pkg Package) string {
	pkg.Checksum = ""
	raw, _ := json.Marshal(pkg)
	digest := sha256.Sum256(raw)
	return hex.EncodeToString(digest[:])
}

func verifySignature(catalog Catalog, encodedKey string) error {
	key, err := base64.StdEncoding.DecodeString(encodedKey)
	if err != nil || len(key) != ed25519.PublicKeySize {
		return errors.New("registry public key is not valid base64 Ed25519")
	}
	signature, err := base64.StdEncoding.DecodeString(catalog.Signature)
	if err != nil || len(signature) != ed25519.SignatureSize {
		return errors.New("registry signature is missing or invalid")
	}
	catalog.Signature = ""
	raw, err := json.Marshal(catalog)
	if err != nil {
		return err
	}
	if !ed25519.Verify(ed25519.PublicKey(key), raw, signature) {
		return errors.New("registry Ed25519 signature verification failed")
	}
	return nil
}

func splitDependency(raw string) (string, string) {
	parts := strings.SplitN(strings.TrimSpace(raw), "@", 2)
	if len(parts) == 1 {
		return parts[0], "*"
	}
	return parts[0], parts[1]
}
