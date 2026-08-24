// Package events defines the NDJSON stdout protocol between the sidecar and
// its Rust parent. One JSON object per line; optional fields are omitted when
// empty so the parent never has to distinguish absent from zero-value.
package events

import (
	"encoding/json"
	"io"
	"sync"
)

type Event struct {
	Type    string `json:"type"`
	URL     string `json:"url,omitempty"`
	FQDN    string `json:"fqdn,omitempty"`
	Code    string `json:"code,omitempty"`
	Message string `json:"message,omitempty"`
}

// Writer serializes events to an underlying writer, one JSON object per line,
// with a mutex so concurrent emitters never interleave lines.
type Writer struct {
	mu sync.Mutex
	w  io.Writer
}

func NewWriter(w io.Writer) *Writer {
	return &Writer{w: w}
}

func (w *Writer) Emit(event Event) error {
	w.mu.Lock()
	defer w.mu.Unlock()
	b, err := json.Marshal(event)
	if err != nil {
		return err
	}
	b = append(b, '\n')
	_, err = w.w.Write(b)
	return err
}
