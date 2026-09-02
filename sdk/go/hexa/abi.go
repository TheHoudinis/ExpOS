package hexa

import (
	"context"
	"errors"
	"fmt"
)

const ABIVersion uint16 = 1

type FIN [16]byte

func (fin FIN) IsZero() bool {
	for _, value := range fin {
		if value != 0 {
			return false
		}
	}
	return true
}

type Call uint16

const (
	CallLog                Call = 1
	CallFormResolve        Call = 2
	CallHandleAuthorize    Call = 3
	CallSurfaceCreate      Call = 16
	CallBufferAttach       Call = 17
	CallSurfaceDamage      Call = 18
	CallSurfaceCommit      Call = 19
	CallEventPoll          Call = 20
	CallBrowserNavigate    Call = 32
	CallPackageTransaction Call = 48
)

type Status uint16

const (
	StatusOK Status = iota
	StatusInvalid
	StatusDenied
	StatusUnsupported
	StatusWouldBlock
)

type Request struct {
	Version   uint16
	Call      Call
	Caller    FIN
	Handle    uint32
	Arguments [6]uint64
}

type Response struct {
	Status Status
	Values [4]uint64
}

type Transport interface {
	Call(context.Context, Request) (Response, error)
}

type Client struct {
	Caller    FIN
	Handle    uint32
	Transport Transport
}

type Rect struct {
	X, Y          int16
	Width, Height uint16
}

func (client Client) CreateSurface(ctx context.Context, rect Rect, role uint16) (uint32, error) {
	response, err := client.invoke(ctx, CallSurfaceCreate, [6]uint64{uint64(uint16(rect.X)), uint64(uint16(rect.Y)), uint64(rect.Width), uint64(rect.Height), uint64(role)})
	return uint32(response.Values[0]), err
}

func (client Client) AttachBuffer(ctx context.Context, surface, buffer uint32, width, height uint16) error {
	_, err := client.invoke(ctx, CallBufferAttach, [6]uint64{uint64(surface), uint64(buffer), uint64(width), uint64(height)})
	return err
}

func (client Client) Damage(ctx context.Context, surface uint32, rect Rect) error {
	_, err := client.invoke(ctx, CallSurfaceDamage, [6]uint64{uint64(surface), uint64(uint16(rect.X)), uint64(uint16(rect.Y)), uint64(rect.Width), uint64(rect.Height)})
	return err
}

func (client Client) Commit(ctx context.Context, surface uint32) (uint64, error) {
	response, err := client.invoke(ctx, CallSurfaceCommit, [6]uint64{uint64(surface)})
	return response.Values[0], err
}

func (client Client) Navigate(ctx context.Context, resource uint64) error {
	_, err := client.invoke(ctx, CallBrowserNavigate, [6]uint64{resource})
	return err
}

func (client Client) invoke(ctx context.Context, call Call, arguments [6]uint64) (Response, error) {
	if client.Transport == nil || client.Caller.IsZero() {
		return Response{}, errors.New("hexa: invalid client identity or transport")
	}
	response, err := client.Transport.Call(ctx, Request{Version: ABIVersion, Call: call, Caller: client.Caller, Handle: client.Handle, Arguments: arguments})
	if err != nil {
		return Response{}, err
	}
	if response.Status != StatusOK {
		return response, fmt.Errorf("hexa: ABI call %d returned status %d", call, response.Status)
	}
	return response, nil
}

// Emulator provides deterministic host tests for Go Forms. The native
// transport will replace it when the execution-context loader is connected.
type Emulator struct {
	AllowedCaller  FIN
	AllowedHandle  uint32
	NextSurface    uint32
	CommitSequence uint64
	Surfaces       map[uint32]EmulatedSurface
}

type EmulatedSurface struct {
	Owner         FIN
	PendingBuffer uint32
	CurrentBuffer uint32
	Width, Height uint16
}

func NewEmulator(caller FIN, handle uint32) *Emulator {
	return &Emulator{AllowedCaller: caller, AllowedHandle: handle, NextSurface: 1, Surfaces: make(map[uint32]EmulatedSurface)}
}

func (emulator *Emulator) Call(_ context.Context, request Request) (Response, error) {
	if request.Version != ABIVersion || request.Caller != emulator.AllowedCaller {
		return Response{Status: StatusInvalid}, nil
	}
	if request.Call != CallLog && request.Handle != emulator.AllowedHandle {
		return Response{Status: StatusDenied}, nil
	}
	switch request.Call {
	case CallSurfaceCreate:
		id := emulator.NextSurface
		emulator.NextSurface++
		emulator.Surfaces[id] = EmulatedSurface{Owner: request.Caller, Width: uint16(request.Arguments[2]), Height: uint16(request.Arguments[3])}
		return Response{Status: StatusOK, Values: [4]uint64{uint64(id)}}, nil
	case CallBufferAttach:
		id := uint32(request.Arguments[0])
		surface, ok := emulator.Surfaces[id]
		if !ok {
			return Response{Status: StatusInvalid}, nil
		}
		surface.PendingBuffer = uint32(request.Arguments[1])
		emulator.Surfaces[id] = surface
		return Response{Status: StatusOK}, nil
	case CallSurfaceDamage:
		if _, ok := emulator.Surfaces[uint32(request.Arguments[0])]; !ok {
			return Response{Status: StatusInvalid}, nil
		}
		return Response{Status: StatusOK}, nil
	case CallSurfaceCommit:
		id := uint32(request.Arguments[0])
		surface, ok := emulator.Surfaces[id]
		if !ok {
			return Response{Status: StatusInvalid}, nil
		}
		surface.CurrentBuffer = surface.PendingBuffer
		emulator.Surfaces[id] = surface
		emulator.CommitSequence++
		return Response{Status: StatusOK, Values: [4]uint64{emulator.CommitSequence}}, nil
	case CallBrowserNavigate, CallLog, CallFormResolve, CallHandleAuthorize, CallEventPoll, CallPackageTransaction:
		return Response{Status: StatusOK}, nil
	default:
		return Response{Status: StatusUnsupported}, nil
	}
}
