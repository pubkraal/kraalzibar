package kraalzibar

import (
	"fmt"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// ErrorCode represents the category of client error.
type ErrorCode int

const (
	CodeUnknown           ErrorCode = iota
	CodeInvalidArgument
	CodeNotFound
	CodePermissionDenied
	CodeFailedPrecondition
	CodeInternal
	CodeTimeout
	CodeUnavailable
)

// Error is the SDK error type wrapping gRPC status details.
type Error struct {
	Code    ErrorCode
	Message string
}

func (e *Error) Error() string {
	return fmt.Sprintf("kraalzibar: %s", e.Message)
}

func errorFromStatus(err error) error {
	if err == nil {
		return nil
	}

	st, ok := status.FromError(err)
	if !ok {
		return &Error{Code: CodeUnknown, Message: err.Error()}
	}

	code := mapGRPCCode(st.Code())
	return &Error{Code: code, Message: st.Message()}
}

func mapGRPCCode(c codes.Code) ErrorCode {
	switch c {
	case codes.InvalidArgument:
		return CodeInvalidArgument
	case codes.NotFound:
		return CodeNotFound
	case codes.PermissionDenied:
		return CodePermissionDenied
	case codes.FailedPrecondition:
		return CodeFailedPrecondition
	case codes.Internal:
		return CodeInternal
	case codes.DeadlineExceeded:
		return CodeTimeout
	case codes.Unavailable:
		return CodeUnavailable
	default:
		return CodeUnknown
	}
}
