package threadsafe

import (
	"io"
	"sync"
)

// WriterWrapper wraps an io.Writer with mutex protection for concurrent writes
type WriterWrapper struct {
	W  io.Writer
	mu sync.Mutex
}

func (tsw *WriterWrapper) Write(p []byte) (n int, err error) {
	tsw.mu.Lock()
	defer tsw.mu.Unlock()
	return tsw.W.Write(p)
}
