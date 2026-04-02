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

	if string(cfg.apiKey) != "test-key" {
		t.Errorf("expected apiKey %q, got %q", "test-key", string(cfg.apiKey))
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

func TestClientConfig_PlusVRedactsAPIKey(t *testing.T) {
	cfg := clientConfig{apiKey: "kraalzibar_abc_secret123"}

	s := fmt.Sprintf("%+v", cfg)

	if strings.Contains(s, "secret123") {
		t.Errorf("%%+v should not contain API key, got: %s", s)
	}

	if !strings.Contains(s, "[REDACTED]") {
		t.Errorf("%%+v should contain [REDACTED], got: %s", s)
	}
}

func TestRedactedString_TypeVerb(t *testing.T) {
	got := fmt.Sprintf("%T", redactedString("key"))

	want := "kraalzibar.redactedString"
	if got != want {
		t.Errorf("%%T = %q, want %q", got, want)
	}
}

func TestClientConfig_TypeVerb(t *testing.T) {
	cfg := clientConfig{apiKey: "secret"}

	got := fmt.Sprintf("%T", cfg)

	want := "kraalzibar.clientConfig"
	if got != want {
		t.Errorf("%%T = %q, want %q", got, want)
	}
}

func TestRedactedString_AllFormatVerbs(t *testing.T) {
	secret := redactedString("super-secret-key")

	verbs := []struct {
		name   string
		format string
	}{
		{"percent-s", "%s"},
		{"percent-v", "%v"},
		{"percent-plus-v", "%+v"},
		{"percent-hash-v", "%#v"},
		{"percent-q", "%q"},
	}

	for _, tt := range verbs {
		t.Run(tt.name, func(t *testing.T) {
			got := fmt.Sprintf(tt.format, secret)
			if strings.Contains(got, "super-secret-key") {
				t.Errorf("format %s leaked secret: %s", tt.format, got)
			}
			if !strings.Contains(got, "REDACTED") {
				t.Errorf("format %s missing REDACTED: %s", tt.format, got)
			}
		})
	}
}
