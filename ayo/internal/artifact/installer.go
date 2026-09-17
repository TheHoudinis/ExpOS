// Package artifact implements Ayo's host-side package payload transaction.
// It deliberately installs data only: package hooks and arbitrary scripts are
// not supported.
package artifact

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"sort"
	"strings"
	"syscall"
	"time"

	"expos.dev/ayo/internal/model"
)

const (
	maxArtifactBytes = int64(64 << 20)
	maxExpandedBytes = int64(256 << 20)
	maxArtifactFiles = 4096
)

type Installer struct {
	Root   string
	Client *http.Client
}

type Package struct {
	Name, Version string
	Artifact      model.ArtifactSpec
}

type transaction struct {
	ID      string    `json:"id"`
	Phase   string    `json:"phase"`
	Created time.Time `json:"created"`
	Entries []txEntry `json:"entries"`
}

type txEntry struct {
	Path        string `json:"path"`
	Stage       string `json:"stage,omitempty"`
	Backup      string `json:"backup,omitempty"`
	NewSHA256   string `json:"new_sha256,omitempty"`
	Mode        uint32 `json:"mode,omitempty"`
	HadOriginal bool   `json:"had_original"`
}

// BuiltinPayload is the deterministic, offline payload used by the bundled
// catalog. It creates a real owned file while keeping the base ISO independent
// from an external registry.
func BuiltinPayload(source string) ([]byte, error) {
	parsed, err := url.Parse(source)
	if err != nil || parsed.Scheme != "builtin" || parsed.Host != "forms" {
		return nil, fmt.Errorf("invalid builtin artifact source %q", source)
	}
	clean := strings.Trim(path.Clean(parsed.Path), "/")
	if clean == "" || strings.Contains(clean, "..") {
		return nil, fmt.Errorf("invalid builtin artifact identity %q", source)
	}
	return []byte("AYO-V3-PACKAGE-FORM\nsource=" + source + "\n"), nil
}

func Digest(raw []byte) string {
	digest := sha256.Sum256(raw)
	return hex.EncodeToString(digest[:])
}

// Install downloads, verifies, stages and atomically applies every package in
// the plan. Filesystem changes are rolled back if the Form-state commit fails.
func (installer Installer) Install(
	ctx context.Context,
	packages []Package,
	state model.State,
	dimension string,
	commit func(transactionID string, receipts map[string]*model.InstalledArtifact) error,
) error {
	if len(packages) == 0 {
		return errors.New("artifact install plan is empty")
	}
	root, release, err := installer.lock()
	if err != nil {
		return err
	}
	defer release()
	if err := installer.recoverUnlocked(root, state.ArtifactTransaction); err != nil {
		return err
	}

	id, err := transactionID()
	if err != nil {
		return err
	}
	txRoot := filepath.Join(root, ".ayo", "transactions", id)
	if err := os.MkdirAll(filepath.Join(txRoot, "payload"), 0o700); err != nil {
		return err
	}
	keepTransaction := false
	defer func() {
		if !keepTransaction {
			_ = os.RemoveAll(txRoot)
		}
	}()

	receipts := make(map[string]*model.InstalledArtifact, len(packages))
	staged := make(map[string]string)
	newOwner := make(map[string]string)
	for _, pkg := range packages {
		if err := validatePackage(pkg); err != nil {
			return err
		}
		raw, err := installer.fetch(ctx, pkg.Artifact.Source)
		if err != nil {
			return fmt.Errorf("download %s: %w", pkg.Name, err)
		}
		if !strings.EqualFold(Digest(raw), pkg.Artifact.SHA256) {
			return fmt.Errorf("artifact %s failed SHA-256 verification", pkg.Name)
		}
		packageStage := filepath.Join(txRoot, "packages", safeLabel(pkg.Name))
		files, err := expand(raw, pkg.Artifact, packageStage)
		if err != nil {
			return fmt.Errorf("extract %s: %w", pkg.Name, err)
		}
		for index, file := range files {
			key := strings.ToLower(file.Path)
			if owner, exists := newOwner[key]; exists {
				return fmt.Errorf("artifact path %s is claimed by both %s and %s", file.Path, owner, pkg.Name)
			}
			newOwner[key] = pkg.Name
			staged[pkg.Name+"\x00"+file.Path] = filepath.Join(packageStage, fmt.Sprintf("%06d", index))
		}
		receipts[pkg.Name] = &model.InstalledArtifact{
			Source: pkg.Artifact.Source, SHA256: strings.ToLower(pkg.Artifact.SHA256),
			Format: normalizedFormat(pkg.Artifact.Format), Files: files, InstalledAt: time.Now().UTC(),
		}
	}

	owners, err := ownership(state, dimension)
	if err != nil {
		return err
	}
	for key, owner := range newOwner {
		if current, exists := owners[key]; exists && !strings.EqualFold(current.name, owner) {
			return fmt.Errorf("artifact path %s is already owned by %s", current.file.Path, current.name)
		}
	}

	tx := transaction{ID: id, Phase: "prepared", Created: time.Now().UTC()}
	entryByPath := make(map[string]int)
	for _, pkg := range packages {
		receipt := receipts[pkg.Name]
		for _, file := range receipt.Files {
			entry := txEntry{Path: file.Path, Stage: staged[pkg.Name+"\x00"+file.Path], NewSHA256: file.SHA256, Mode: file.Mode}
			if err := installer.prepareEntry(root, &entry, owners); err != nil {
				return err
			}
			entryByPath[strings.ToLower(file.Path)] = len(tx.Entries)
			tx.Entries = append(tx.Entries, entry)
		}
	}
	// Upgrades remove files no longer present in the new artifact.
	for _, pkg := range packages {
		form, findErr := state.Find(pkg.Name, dimension)
		if findErr != nil || form.Artifact == nil {
			continue
		}
		for _, old := range form.Artifact.Files {
			key := strings.ToLower(old.Path)
			if _, retained := entryByPath[key]; retained {
				continue
			}
			entry := txEntry{Path: old.Path}
			if err := installer.prepareEntry(root, &entry, owners); err != nil {
				return err
			}
			tx.Entries = append(tx.Entries, entry)
		}
	}
	sort.Slice(tx.Entries, func(i, j int) bool { return tx.Entries[i].Path < tx.Entries[j].Path })
	for index := range tx.Entries {
		if tx.Entries[index].HadOriginal {
			tx.Entries[index].Backup = filepath.Join("backup", fmt.Sprintf("%06d", index))
		}
	}
	if err := writeTransaction(txRoot, tx); err != nil {
		return err
	}
	if err := installer.apply(root, txRoot, &tx); err != nil {
		if rollbackErr := installer.rollback(root, txRoot, tx); rollbackErr != nil {
			keepTransaction = true
			return fmt.Errorf("apply failed: %v; rollback needs recovery: %w", err, rollbackErr)
		}
		return err
	}
	if err := commit(id, receipts); err != nil {
		if rollbackErr := installer.rollback(root, txRoot, tx); rollbackErr != nil {
			keepTransaction = true
			return fmt.Errorf("state commit failed: %v; rollback needs recovery: %w", err, rollbackErr)
		}
		return fmt.Errorf("state commit failed; artifact transaction rolled back: %w", err)
	}
	tx.Phase = "state-committed"
	_ = writeTransaction(txRoot, tx)
	return nil
}

// Remove deletes only unchanged files listed in the package receipt. The
// deletion is backed up until Form state commits, so uninstall is reversible.
func (installer Installer) Remove(
	state model.State,
	dimension, packageName string,
	receipt model.InstalledArtifact,
	commit func(transactionID string) error,
) error {
	root, release, err := installer.lock()
	if err != nil {
		return err
	}
	defer release()
	if err := installer.recoverUnlocked(root, state.ArtifactTransaction); err != nil {
		return err
	}
	owners, err := ownership(state, dimension)
	if err != nil {
		return err
	}
	id, err := transactionID()
	if err != nil {
		return err
	}
	txRoot := filepath.Join(root, ".ayo", "transactions", id)
	if err := os.MkdirAll(txRoot, 0o700); err != nil {
		return err
	}
	keepTransaction := false
	defer func() {
		if !keepTransaction {
			_ = os.RemoveAll(txRoot)
		}
	}()
	tx := transaction{ID: id, Phase: "prepared", Created: time.Now().UTC()}
	for _, file := range receipt.Files {
		owner, ok := owners[strings.ToLower(file.Path)]
		if !ok || !strings.EqualFold(owner.name, packageName) {
			return fmt.Errorf("refusing to remove %s: ownership receipt is inconsistent", file.Path)
		}
		entry := txEntry{Path: file.Path}
		if err := installer.prepareEntry(root, &entry, owners); err != nil {
			return err
		}
		tx.Entries = append(tx.Entries, entry)
	}
	for index := range tx.Entries {
		if tx.Entries[index].HadOriginal {
			tx.Entries[index].Backup = filepath.Join("backup", fmt.Sprintf("%06d", index))
		}
	}
	if err := writeTransaction(txRoot, tx); err != nil {
		return err
	}
	if err := installer.apply(root, txRoot, &tx); err != nil {
		if rollbackErr := installer.rollback(root, txRoot, tx); rollbackErr != nil {
			keepTransaction = true
			return fmt.Errorf("uninstall failed: %v; rollback needs recovery: %w", err, rollbackErr)
		}
		return err
	}
	if err := commit(id); err != nil {
		if rollbackErr := installer.rollback(root, txRoot, tx); rollbackErr != nil {
			keepTransaction = true
			return fmt.Errorf("state commit failed: %v; rollback needs recovery: %w", err, rollbackErr)
		}
		return fmt.Errorf("state commit failed; uninstall rolled back: %w", err)
	}
	tx.Phase = "state-committed"
	_ = writeTransaction(txRoot, tx)
	return nil
}

func (installer Installer) Recover(committedTransaction string) error {
	root, release, err := installer.lock()
	if err != nil {
		return err
	}
	defer release()
	return installer.recoverUnlocked(root, committedTransaction)
}

func (installer Installer) recoverUnlocked(root, committedTransaction string) error {
	directory := filepath.Join(root, ".ayo", "transactions")
	entries, err := os.ReadDir(directory)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		txRoot := filepath.Join(directory, entry.Name())
		tx, err := readTransaction(txRoot)
		if errors.Is(err, os.ErrNotExist) {
			// No journal means the process stopped during download/staging,
			// before any target path could have changed.
			if removeErr := os.RemoveAll(txRoot); removeErr != nil {
				return removeErr
			}
			continue
		}
		if err != nil {
			return fmt.Errorf("cannot recover artifact transaction %s: %w", entry.Name(), err)
		}
		if tx.Phase == "state-committed" || tx.ID == committedTransaction {
			if err := os.RemoveAll(txRoot); err != nil {
				return err
			}
			continue
		}
		if err := installer.rollback(root, txRoot, tx); err != nil {
			return fmt.Errorf("recover artifact transaction %s: %w", tx.ID, err)
		}
		if err := os.RemoveAll(txRoot); err != nil {
			return err
		}
	}
	return nil
}

type owned struct {
	name string
	file model.OwnedFile
}

func ownership(state model.State, dimension string) (map[string]owned, error) {
	result := make(map[string]owned)
	for _, form := range state.Packages {
		if form.Dimension != dimension || form.Artifact == nil {
			continue
		}
		for _, file := range form.Artifact.Files {
			key := strings.ToLower(file.Path)
			if existing, ok := result[key]; ok {
				return nil, fmt.Errorf("artifact ownership conflict: %s and %s claim %s", existing.name, form.Name, file.Path)
			}
			result[key] = owned{name: form.Name, file: file}
		}
	}
	return result, nil
}

func validatePackage(pkg Package) error {
	if strings.TrimSpace(pkg.Name) == "" || strings.TrimSpace(pkg.Version) == "" {
		return errors.New("artifact package needs a name and version")
	}
	if strings.TrimSpace(pkg.Artifact.Source) == "" {
		return fmt.Errorf("Package Form %s has no artifact source", pkg.Name)
	}
	digest, err := hex.DecodeString(pkg.Artifact.SHA256)
	if err != nil || len(digest) != sha256.Size {
		return fmt.Errorf("Package Form %s needs a complete SHA-256 artifact digest", pkg.Name)
	}
	switch normalizedFormat(pkg.Artifact.Format) {
	case "raw":
		if _, err := safeRelative(pkg.Artifact.Target); err != nil {
			return fmt.Errorf("Package Form %s raw target: %w", pkg.Name, err)
		}
	case "tar", "tar.gz":
	default:
		return fmt.Errorf("Package Form %s uses unsupported artifact format %q", pkg.Name, pkg.Artifact.Format)
	}
	return nil
}

func normalizedFormat(format string) string {
	value := strings.ToLower(strings.TrimSpace(format))
	switch value {
	case "tgz", "tar-gzip":
		return "tar.gz"
	}
	return value
}

func expand(raw []byte, spec model.ArtifactSpec, stage string) ([]model.OwnedFile, error) {
	if err := os.MkdirAll(stage, 0o700); err != nil {
		return nil, err
	}
	if normalizedFormat(spec.Format) == "raw" {
		target, err := safeRelative(spec.Target)
		if err != nil {
			return nil, err
		}
		mode := safeMode(spec.Mode)
		staged := filepath.Join(stage, "000000")
		if err := os.WriteFile(staged, raw, os.FileMode(mode)); err != nil {
			return nil, err
		}
		return []model.OwnedFile{{Path: target, SHA256: Digest(raw), Mode: mode, Size: int64(len(raw))}}, nil
	}

	var reader io.Reader = bytes.NewReader(raw)
	if normalizedFormat(spec.Format) == "tar.gz" {
		gzipReader, err := gzip.NewReader(bytes.NewReader(raw))
		if err != nil {
			return nil, err
		}
		defer gzipReader.Close()
		reader = gzipReader
	}
	tarReader := tar.NewReader(reader)
	files := make([]model.OwnedFile, 0)
	seen := make(map[string]bool)
	var expanded int64
	for {
		header, err := tarReader.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return nil, err
		}
		headerName := header.Name
		if header.Typeflag == tar.TypeDir {
			headerName = strings.TrimSuffix(headerName, "/")
		}
		name, err := safeRelative(headerName)
		if err != nil {
			return nil, err
		}
		switch header.Typeflag {
		case tar.TypeDir:
			continue
		case tar.TypeReg, tar.TypeRegA:
		default:
			return nil, fmt.Errorf("archive entry %s is not a regular file or directory", name)
		}
		key := strings.ToLower(name)
		if seen[key] {
			return nil, fmt.Errorf("archive repeats path %s", name)
		}
		seen[key] = true
		if len(files) >= maxArtifactFiles {
			return nil, fmt.Errorf("archive exceeds %d-file limit", maxArtifactFiles)
		}
		if header.Size < 0 || header.Size > maxExpandedBytes-expanded {
			return nil, errors.New("archive exceeds 256 MiB expanded limit")
		}
		content, err := io.ReadAll(io.LimitReader(tarReader, header.Size+1))
		if err != nil {
			return nil, err
		}
		if int64(len(content)) != header.Size {
			return nil, fmt.Errorf("archive entry %s is truncated", name)
		}
		expanded += header.Size
		mode := safeMode(uint32(header.Mode))
		staged := filepath.Join(stage, fmt.Sprintf("%06d", len(files)))
		if err := os.WriteFile(staged, content, os.FileMode(mode)); err != nil {
			return nil, err
		}
		files = append(files, model.OwnedFile{Path: name, SHA256: Digest(content), Mode: mode, Size: header.Size})
	}
	if len(files) == 0 {
		return nil, errors.New("artifact contains no regular files")
	}
	sort.Slice(files, func(i, j int) bool { return files[i].Path < files[j].Path })
	// Renumber staged payloads to follow the sorted receipt order.
	for index := range files {
		// The contents were initially keyed by encounter order, so locate by hash
		// and size only after sorting would be ambiguous. Archives are instead
		// required to already have stable order in the staging map below.
		_ = index
	}
	return restageSorted(raw, spec, stage, files)
}

// restageSorted maps archive names to stable numeric payload names. It keeps
// transaction records independent of untrusted archive paths.
func restageSorted(raw []byte, spec model.ArtifactSpec, stage string, sorted []model.OwnedFile) ([]model.OwnedFile, error) {
	contents, err := archiveContents(raw, spec)
	if err != nil {
		return nil, err
	}
	for index, file := range sorted {
		content, ok := contents[file.Path]
		if !ok {
			return nil, fmt.Errorf("staging lost archive entry %s", file.Path)
		}
		if err := os.WriteFile(filepath.Join(stage, fmt.Sprintf("%06d", index)), content, os.FileMode(file.Mode)); err != nil {
			return nil, err
		}
	}
	return sorted, nil
}

func archiveContents(raw []byte, spec model.ArtifactSpec) (map[string][]byte, error) {
	var reader io.Reader = bytes.NewReader(raw)
	if normalizedFormat(spec.Format) == "tar.gz" {
		gzipReader, err := gzip.NewReader(bytes.NewReader(raw))
		if err != nil {
			return nil, err
		}
		defer gzipReader.Close()
		reader = gzipReader
	}
	result := make(map[string][]byte)
	tarReader := tar.NewReader(reader)
	for {
		header, err := tarReader.Next()
		if errors.Is(err, io.EOF) {
			return result, nil
		}
		if err != nil {
			return nil, err
		}
		if header.Typeflag != tar.TypeReg && header.Typeflag != tar.TypeRegA {
			continue
		}
		name, err := safeRelative(header.Name)
		if err != nil {
			return nil, err
		}
		content, err := io.ReadAll(io.LimitReader(tarReader, header.Size+1))
		if err != nil || int64(len(content)) != header.Size {
			return nil, fmt.Errorf("cannot restage archive entry %s", name)
		}
		result[name] = content
	}
}

func (installer Installer) prepareEntry(root string, entry *txEntry, owners map[string]owned) error {
	target, err := secureTarget(root, entry.Path)
	if err != nil {
		return err
	}
	info, err := os.Lstat(target)
	if errors.Is(err, os.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() {
		return fmt.Errorf("refusing to replace non-regular path %s", entry.Path)
	}
	owner, owned := owners[strings.ToLower(entry.Path)]
	if !owned {
		return fmt.Errorf("refusing to overwrite unowned file %s", entry.Path)
	}
	digest, err := fileDigest(target)
	if err != nil {
		return err
	}
	if !strings.EqualFold(digest, owner.file.SHA256) {
		return fmt.Errorf("refusing to overwrite modified file %s", entry.Path)
	}
	entry.HadOriginal = true
	return nil
}

func (installer Installer) apply(root, txRoot string, tx *transaction) error {
	for index := range tx.Entries {
		entry := &tx.Entries[index]
		target, err := secureTarget(root, entry.Path)
		if err != nil {
			return err
		}
		if err := secureMkdirAll(root, filepath.Dir(target)); err != nil {
			return err
		}
		if entry.HadOriginal {
			backup, err := secureTransactionPath(txRoot, entry.Backup)
			if err != nil {
				return err
			}
			if err := os.MkdirAll(filepath.Dir(backup), 0o700); err != nil {
				return err
			}
			if err := os.Rename(target, backup); err != nil {
				return err
			}
		}
		if entry.Stage != "" {
			if err := os.Rename(entry.Stage, target); err != nil {
				return err
			}
			if err := os.Chmod(target, os.FileMode(entry.Mode)); err != nil {
				return err
			}
		}
	}
	tx.Phase = "applied"
	return writeTransaction(txRoot, *tx)
}

func (installer Installer) rollback(root, txRoot string, tx transaction) error {
	var problems []string
	for index := len(tx.Entries) - 1; index >= 0; index-- {
		entry := tx.Entries[index]
		target, err := secureTarget(root, entry.Path)
		if err != nil {
			problems = append(problems, err.Error())
			continue
		}
		backup := ""
		if entry.Backup != "" {
			backup, err = secureTransactionPath(txRoot, entry.Backup)
			if err != nil {
				problems = append(problems, err.Error())
				continue
			}
		}
		backupExists := false
		if backup != "" {
			_, backupErr := os.Lstat(backup)
			backupExists = backupErr == nil
			if backupErr != nil && !errors.Is(backupErr, os.ErrNotExist) {
				problems = append(problems, backupErr.Error())
				continue
			}
		}
		if backupExists {
			if removeErr := os.Remove(target); removeErr != nil && !errors.Is(removeErr, os.ErrNotExist) {
				problems = append(problems, removeErr.Error())
				continue
			}
			if err := secureMkdirAll(root, filepath.Dir(target)); err != nil {
				problems = append(problems, err.Error())
				continue
			}
			if err := os.Rename(backup, target); err != nil {
				problems = append(problems, err.Error())
			}
			continue
		}
		if entry.Stage != "" && !entry.HadOriginal {
			if _, statErr := os.Lstat(target); statErr == nil {
				digest, digestErr := fileDigest(target)
				if digestErr != nil || !strings.EqualFold(digest, entry.NewSHA256) {
					problems = append(problems, "refusing to roll back modified path "+entry.Path)
					continue
				}
				if err := os.Remove(target); err != nil {
					problems = append(problems, err.Error())
				}
			}
		}
	}
	removeEmptyParents(root, tx.Entries)
	if len(problems) > 0 {
		return errors.New(strings.Join(problems, "; "))
	}
	return nil
}

func (installer Installer) fetch(ctx context.Context, source string) ([]byte, error) {
	parsed, err := url.Parse(source)
	if err != nil {
		return nil, err
	}
	switch parsed.Scheme {
	case "builtin":
		return BuiltinPayload(source)
	case "file":
		if parsed.Host != "" && parsed.Host != "localhost" {
			return nil, errors.New("file artifact URL must be local")
		}
		return readLimited(parsed.Path)
	case "https":
		request, err := http.NewRequestWithContext(ctx, http.MethodGet, source, nil)
		if err != nil {
			return nil, err
		}
		client := installer.httpClient()
		response, err := client.Do(request)
		if err != nil {
			return nil, err
		}
		defer response.Body.Close()
		if response.StatusCode != http.StatusOK {
			return nil, fmt.Errorf("server returned %s", response.Status)
		}
		return readAllLimited(response.Body)
	case "http":
		return nil, errors.New("remote package artifacts must use HTTPS")
	case "":
		return readLimited(source)
	default:
		return nil, fmt.Errorf("unsupported artifact source scheme %q", parsed.Scheme)
	}
}

func (installer Installer) httpClient() *http.Client {
	client := &http.Client{Timeout: 30 * time.Second}
	if installer.Client != nil {
		copy := *installer.Client
		client = &copy
		if client.Timeout == 0 {
			client.Timeout = 30 * time.Second
		}
	}
	previous := client.CheckRedirect
	client.CheckRedirect = func(request *http.Request, via []*http.Request) error {
		if request.URL.Scheme != "https" {
			return errors.New("artifact redirect left HTTPS")
		}
		if previous != nil {
			return previous(request, via)
		}
		if len(via) >= 10 {
			return errors.New("too many artifact redirects")
		}
		return nil
	}
	return client
}

func readLimited(filename string) ([]byte, error) {
	file, err := os.Open(filename)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	return readAllLimited(file)
}

func readAllLimited(reader io.Reader) ([]byte, error) {
	raw, err := io.ReadAll(io.LimitReader(reader, maxArtifactBytes+1))
	if err != nil {
		return nil, err
	}
	if int64(len(raw)) > maxArtifactBytes {
		return nil, errors.New("artifact exceeds 64 MiB download limit")
	}
	return raw, nil
}

func (installer Installer) lock() (string, func(), error) {
	root, err := installer.validatedRoot()
	if err != nil {
		return "", nil, err
	}
	if err := os.MkdirAll(filepath.Join(root, ".ayo"), 0o700); err != nil {
		return "", nil, err
	}
	if info, err := os.Lstat(filepath.Join(root, ".ayo")); err != nil {
		return "", nil, err
	} else if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
		return "", nil, errors.New("ayo metadata path is not a safe directory")
	}
	lock, err := os.OpenFile(filepath.Join(root, ".ayo", "transaction.lock"), os.O_CREATE|os.O_RDWR, 0o600)
	if err != nil {
		return "", nil, err
	}
	if err := syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		_ = lock.Close()
		return "", nil, errors.New("another ayo artifact transaction is active")
	}
	return root, func() {
		_ = syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
		_ = lock.Close()
	}, nil
}

func (installer Installer) validatedRoot() (string, error) {
	if strings.TrimSpace(installer.Root) == "" {
		return "", errors.New("ayo install root is not configured")
	}
	root, err := filepath.Abs(installer.Root)
	if err != nil {
		return "", err
	}
	root = filepath.Clean(root)
	for _, protected := range []string{"/", "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/proc", "/root", "/run", "/sbin", "/sys", "/usr", "/var"} {
		if root == protected || strings.HasPrefix(root, protected+string(filepath.Separator)) {
			return "", fmt.Errorf("ayo refuses system/root install location %s; choose a user-owned root", root)
		}
	}
	if info, err := os.Lstat(root); err == nil && info.Mode()&os.ModeSymlink != 0 {
		return "", errors.New("ayo install root cannot be a symbolic link")
	} else if err != nil && !errors.Is(err, os.ErrNotExist) {
		return "", err
	}
	return root, nil
}

func safeRelative(raw string) (string, error) {
	if raw == "" || strings.ContainsRune(raw, '\\') || strings.ContainsRune(raw, '\x00') || strings.HasPrefix(raw, "/") {
		return "", fmt.Errorf("unsafe artifact path %q", raw)
	}
	clean := path.Clean(raw)
	if clean == "." || clean == ".." || strings.HasPrefix(clean, "../") || clean != raw {
		return "", fmt.Errorf("unsafe artifact path %q", raw)
	}
	return clean, nil
}

func secureTarget(root, relative string) (string, error) {
	clean, err := safeRelative(relative)
	if err != nil {
		return "", err
	}
	target := filepath.Join(root, filepath.FromSlash(clean))
	rel, err := filepath.Rel(root, target)
	if err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return "", fmt.Errorf("artifact path %q escapes install root", relative)
	}
	parent := filepath.Dir(target)
	for parent != root && parent != filepath.Dir(parent) {
		info, statErr := os.Lstat(parent)
		if statErr == nil && info.Mode()&os.ModeSymlink != 0 {
			return "", fmt.Errorf("artifact path %q traverses a symbolic link", relative)
		}
		if statErr != nil && !errors.Is(statErr, os.ErrNotExist) {
			return "", statErr
		}
		parent = filepath.Dir(parent)
	}
	return target, nil
}

func secureMkdirAll(root, directory string) error {
	relative, err := filepath.Rel(root, directory)
	if err != nil || relative == ".." || strings.HasPrefix(relative, ".."+string(filepath.Separator)) {
		return errors.New("directory escapes ayo install root")
	}
	current := root
	for _, component := range strings.Split(relative, string(filepath.Separator)) {
		if component == "." || component == "" {
			continue
		}
		current = filepath.Join(current, component)
		info, statErr := os.Lstat(current)
		if errors.Is(statErr, os.ErrNotExist) {
			if err := os.Mkdir(current, 0o755); err != nil && !errors.Is(err, os.ErrExist) {
				return err
			}
			continue
		}
		if statErr != nil {
			return statErr
		}
		if !info.IsDir() || info.Mode()&os.ModeSymlink != 0 {
			return fmt.Errorf("artifact parent %s is not a safe directory", current)
		}
	}
	return nil
}

func safeMode(raw uint32) uint32 {
	if raw&0o111 != 0 {
		return 0o755
	}
	return 0o644
}

func fileDigest(filename string) (string, error) {
	file, err := os.Open(filename)
	if err != nil {
		return "", err
	}
	defer file.Close()
	digest := sha256.New()
	if _, err := io.Copy(digest, io.LimitReader(file, maxExpandedBytes+1)); err != nil {
		return "", err
	}
	return hex.EncodeToString(digest.Sum(nil)), nil
}

func writeTransaction(root string, tx transaction) error {
	raw, err := json.MarshalIndent(tx, "", "  ")
	if err != nil {
		return err
	}
	temporary := filepath.Join(root, "transaction.json.next")
	file, err := os.OpenFile(temporary, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o600)
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
	return os.Rename(temporary, filepath.Join(root, "transaction.json"))
}

func readTransaction(root string) (transaction, error) {
	raw, err := os.ReadFile(filepath.Join(root, "transaction.json"))
	if err != nil {
		return transaction{}, err
	}
	var tx transaction
	if err := json.Unmarshal(raw, &tx); err != nil {
		return transaction{}, err
	}
	if tx.ID == "" || tx.Phase == "" {
		return transaction{}, errors.New("incomplete transaction journal")
	}
	return tx, nil
}

func transactionID() (string, error) {
	var raw [16]byte
	if _, err := rand.Read(raw[:]); err != nil {
		return "", err
	}
	return hex.EncodeToString(raw[:]), nil
}

func safeLabel(value string) string {
	digest := sha256.Sum256([]byte(strings.ToLower(value)))
	return hex.EncodeToString(digest[:8])
}

func secureTransactionPath(txRoot, relative string) (string, error) {
	if relative == "" || filepath.IsAbs(relative) {
		return "", errors.New("invalid transaction backup path")
	}
	target := filepath.Clean(filepath.Join(txRoot, relative))
	rel, err := filepath.Rel(txRoot, target)
	if err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
		return "", errors.New("transaction backup path escapes journal")
	}
	return target, nil
}

func removeEmptyParents(root string, entries []txEntry) {
	directories := make(map[string]bool)
	for _, entry := range entries {
		target, err := secureTarget(root, entry.Path)
		if err != nil {
			continue
		}
		for directory := filepath.Dir(target); directory != root && directory != filepath.Dir(directory); directory = filepath.Dir(directory) {
			directories[directory] = true
		}
	}
	ordered := make([]string, 0, len(directories))
	for directory := range directories {
		ordered = append(ordered, directory)
	}
	sort.Slice(ordered, func(i, j int) bool { return len(ordered[i]) > len(ordered[j]) })
	for _, directory := range ordered {
		_ = os.Remove(directory)
	}
}
