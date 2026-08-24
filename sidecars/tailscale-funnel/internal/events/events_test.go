package events

import (
	"bytes"
	"strings"
	"testing"
)

func TestWriterEmitsOneJSONObjectPerLine(t *testing.T) {
	var out bytes.Buffer
	w := NewWriter(&out)
	if err := w.Emit(Event{Type: "starting"}); err != nil {
		t.Fatal(err)
	}
	if got, want := out.String(), "{\"type\":\"starting\"}\n"; got != want {
		t.Fatalf("got %q, want %q", got, want)
	}
}

func TestWriterNeverIncludesEmptyOptionalFields(t *testing.T) {
	var out bytes.Buffer
	w := NewWriter(&out)
	if err := w.Emit(Event{Type: "error", Code: "FUNNEL_NOT_ALLOWED", Message: "denied"}); err != nil {
		t.Fatal(err)
	}
	got := out.String()
	if strings.Contains(got, "\"url\"") || strings.Contains(got, "\"fqdn\"") {
		t.Fatalf("empty optional fields leaked: %s", got)
	}
}
