package kraalzibar

import (
	"fmt"
	"time"

	"google.golang.org/grpc"
)

type redactedString string

func (r redactedString) String() string   { return "[REDACTED]" }
func (r redactedString) GoString() string { return `redactedString("[REDACTED]")` }
func (r redactedString) Format(f fmt.State, verb rune) {
	if verb == 'T' {
		fmt.Fprint(f, "kraalzibar.redactedString")

		return
	}

	fmt.Fprint(f, "[REDACTED]")
}

type clientConfig struct {
	insecure    bool
	apiKey      redactedString
	timeout     time.Duration
	dialOptions []grpc.DialOption
}

// String returns a human-readable representation with the API key redacted.
func (c clientConfig) String() string {
	return fmt.Sprintf("{insecure:%v apiKey:[REDACTED] timeout:%v}", c.insecure, c.timeout)
}

// GoString returns a Go-syntax representation with the API key redacted.
func (c clientConfig) GoString() string {
	return c.String()
}

// Format implements fmt.Formatter to ensure all format verbs redact the API key.
func (c clientConfig) Format(f fmt.State, verb rune) {
	if verb == 'T' {
		fmt.Fprint(f, "kraalzibar.clientConfig")

		return
	}

	fmt.Fprint(f, c.String())
}

func defaultConfig() clientConfig {
	return clientConfig{
		timeout: 30 * time.Second,
	}
}

// ClientOption configures the Client.
type ClientOption func(*clientConfig)

// WithInsecure disables TLS for the gRPC connection.
func WithInsecure() ClientOption {
	return func(c *clientConfig) {
		c.insecure = true
	}
}

// WithAPIKey sets the Bearer token for authentication.
func WithAPIKey(key string) ClientOption {
	return func(c *clientConfig) {
		c.apiKey = redactedString(key)
	}
}

// WithTimeout sets the per-request deadline.
func WithTimeout(d time.Duration) ClientOption {
	return func(c *clientConfig) {
		c.timeout = d
	}
}

// WithDialOptions appends additional gRPC dial options.
func WithDialOptions(opts ...grpc.DialOption) ClientOption {
	return func(c *clientConfig) {
		c.dialOptions = append(c.dialOptions, opts...)
	}
}
