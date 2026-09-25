package expos

import (
	"context"
	"testing"
	"unsafe"
)

func TestGoFormSurfaceCommitIsAtomic(t *testing.T) {
	caller := FIN{15: 7}
	kernel := NewEmulator(caller, 9)
	client := Client{Caller: caller, Handle: 9, Transport: kernel}
	surface, err := client.CreateSurface(context.Background(), Rect{X: 20, Y: 20, Width: 640, Height: 480}, 1)
	if err != nil {
		t.Fatal(err)
	}
	if err := client.AttachBuffer(context.Background(), surface, 44, 640, 480); err != nil {
		t.Fatal(err)
	}
	if kernel.Surfaces[surface].CurrentBuffer != 0 {
		t.Fatal("pending buffer became visible before commit")
	}
	sequence, err := client.Commit(context.Background(), surface)
	if err != nil {
		t.Fatal(err)
	}
	if sequence != 1 || kernel.Surfaces[surface].CurrentBuffer != 44 {
		t.Fatalf("commit failed: %#v", kernel.Surfaces[surface])
	}
}

func TestGoFormCannotBorrowAnotherHandle(t *testing.T) {
	caller := FIN{15: 7}
	kernel := NewEmulator(caller, 9)
	client := Client{Caller: caller, Handle: 10, Transport: kernel}
	if _, err := client.CreateSurface(context.Background(), Rect{Width: 100, Height: 100}, 1); err == nil {
		t.Fatal("foreign Handle was accepted")
	}
}

func TestABILayoutMatchesKernelContract(t *testing.T) {
	if unsafe.Sizeof(Request{}) != 72 || unsafe.Sizeof(Response{}) != 40 {
		t.Fatalf("ABI layout drifted: request=%d response=%d", unsafe.Sizeof(Request{}), unsafe.Sizeof(Response{}))
	}
	if CallTimeNow != 4 || CallSurfaceCreate != 16 || CallStorageRead != 33 || CallNetworkReceive != 36 || CallPackageTransaction != 48 {
		t.Fatal("Form ABI v1 call numbers drifted")
	}
}
