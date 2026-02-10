package kraalzibar

import (
	"errors"
	"testing"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

func TestErrorFromStatus_NotFound(t *testing.T) {
	st := status.Error(codes.NotFound, "resource missing")
	err := errorFromStatus(st)

	var kErr *Error
	if !errors.As(err, &kErr) {
		t.Fatal("expected *Error")
	}
	if kErr.Code != CodeNotFound {
		t.Errorf("expected CodeNotFound, got %d", kErr.Code)
	}
	if kErr.Message != "resource missing" {
		t.Errorf("expected message %q, got %q", "resource missing", kErr.Message)
	}
}

func TestErrorFromStatus_InvalidArgument(t *testing.T) {
	st := status.Error(codes.InvalidArgument, "bad input")
	err := errorFromStatus(st)

	var kErr *Error
	if !errors.As(err, &kErr) {
		t.Fatal("expected *Error")
	}
	if kErr.Code != CodeInvalidArgument {
		t.Errorf("expected CodeInvalidArgument, got %d", kErr.Code)
	}
}

func TestErrorFromStatus_PermissionDenied(t *testing.T) {
	st := status.Error(codes.PermissionDenied, "no access")
	err := errorFromStatus(st)

	var kErr *Error
	if !errors.As(err, &kErr) {
		t.Fatal("expected *Error")
	}
	if kErr.Code != CodePermissionDenied {
		t.Errorf("expected CodePermissionDenied, got %d", kErr.Code)
	}
}

func TestErrorFromStatus_Internal(t *testing.T) {
	st := status.Error(codes.Internal, "server broke")
	err := errorFromStatus(st)

	var kErr *Error
	if !errors.As(err, &kErr) {
		t.Fatal("expected *Error")
	}
	if kErr.Code != CodeInternal {
		t.Errorf("expected CodeInternal, got %d", kErr.Code)
	}
}
