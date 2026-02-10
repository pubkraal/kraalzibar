package kraalzibar

import (
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
