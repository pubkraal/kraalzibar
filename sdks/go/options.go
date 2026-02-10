package kraalzibar

import (
	"time"

	"google.golang.org/grpc"
)

type clientConfig struct {
	insecure    bool
	apiKey      string
	timeout     time.Duration
	dialOptions []grpc.DialOption
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
		c.apiKey = key
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
