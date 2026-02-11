package threadsafe

import (
	"bytes"
	"sync"
)

// Following code has been extracted from: https://github.com/solo-io/go-utils/blob/baaf9bb972468c16c64f955e41bf246081348281/threadsafe/buffer.go to avoid
// importing the whole package.

// Buffer provides a thread-safe wrapper around bytes.Buffer
type Buffer struct {
	buffer bytes.Buffer
	mutex  sync.Mutex
}

// Write appends the contents of p to the buffer, growing the buffer as needed.
// It returns the number of bytes written and is thread-safe.
func (b *Buffer) Write(p []byte) (n int, err error) {
	b.mutex.Lock()
	defer b.mutex.Unlock()
	return b.buffer.Write(p)
}

// String returns the contents of the buffer as a string in a thread-safe manner.
func (b *Buffer) String() string {
	b.mutex.Lock()
	defer b.mutex.Unlock()
	return b.buffer.String()
}

// Bytes returns a slice of length b.Len() holding the unread portion of the buffer.
// The slice is valid for use only until the next buffer modification.
func (b *Buffer) Bytes() []byte {
	b.mutex.Lock()
	defer b.mutex.Unlock()
	return b.buffer.Bytes()
}

// Len returns the number of bytes of the unread portion of the buffer.
func (b *Buffer) Len() int {
	b.mutex.Lock()
	defer b.mutex.Unlock()
	return b.buffer.Len()
}

// Reset resets the buffer to be empty, but it retains the underlying storage for use by future writes.
func (b *Buffer) Reset() {
	b.mutex.Lock()
	defer b.mutex.Unlock()
	b.buffer.Reset()
}

// Read reads the next len(p) bytes from the buffer or until the buffer
// is drained. The return value n is the number of bytes read.
func (b *Buffer) Read(p []byte) (n int, err error) {
	b.mutex.Lock()
	defer b.mutex.Unlock()
	return b.buffer.Read(p)
}
