package kraalzibar

import (
	"fmt"
	"strings"
	"testing"
	"time"
)

func TestDefaultOptions_Sensible(t *testing.T) {
	cfg := defaultConfig()

	if cfg.insecure {
		t.Error("expected insecure to be false by default")
	}
	if cfg.apiKey != "" {
		t.Error("expected apiKey to be empty by default")
	}
	if cfg.timeout != 30*time.Second {
		t.Errorf("expected default timeout 30s, got %v", cfg.timeout)
	}
}

func TestWithInsecure_SetsFlag(t *testing.T) {
	cfg := defaultConfig()
	WithInsecure()(&cfg)

	if !cfg.insecure {
		t.Error("expected insecure to be true after WithInsecure()")
	}
}

func TestWithAPIKey_SetsCredentials(t *testing.T) {
	cfg := defaultConfig()
	WithAPIKey("test-key")(&cfg)

	if cfg.apiKey != "test-key" {
		t.Errorf("expected apiKey %q, got %q", "test-key", cfg.apiKey)
	}
}

func TestWithTimeout_SetsDeadline(t *testing.T) {
	cfg := defaultConfig()
	WithTimeout(5 * time.Second)(&cfg)

	if cfg.timeout != 5*time.Second {
		t.Errorf("expected timeout 5s, got %v", cfg.timeout)
	}
}

func TestClientConfig_StringRedactsAPIKey(t *testing.T) {
	cfg := clientConfig{apiKey: "kraalzibar_abc_secret123"}

	s := fmt.Sprintf("%v", cfg)

	if strings.Contains(s, "secret123") {
		t.Errorf("String() should not contain API key, got: %s", s)
	}

	if !strings.Contains(s, "[REDACTED]") {
		t.Errorf("String() should contain [REDACTED], got: %s", s)
	}
}

func TestClientConfig_GoStringRedactsAPIKey(t *testing.T) {
	cfg := clientConfig{apiKey: "kraalzibar_abc_secret123"}

	s := fmt.Sprintf("%#v", cfg)

	if strings.Contains(s, "secret123") {
		t.Errorf("GoString() should not contain API key, got: %s", s)
	}

	if !strings.Contains(s, "[REDACTED]") {
		t.Errorf("GoString() should contain [REDACTED], got: %s", s)
	}
}
