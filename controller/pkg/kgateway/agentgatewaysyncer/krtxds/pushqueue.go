// This file is derived from https://github.com/istio/istio/blob/master/pilot/pkg/xds/pushqueue.go (Apache 2.0)

package krtxds

import (
	"sync"
)

type PushQueue struct {
	cond *sync.Cond

	// pending stores all connections in the queue. If the same connection is enqueued again,
	// the PushRequest will be merged.
	pending map[*Connection]*PushRequest

	// queue maintains ordering of the queue
	queue []*Connection

	// processing stores all connections that have been Dequeue(), but not MarkDone().
	// The value stored will be initially be nil, but may be populated if the connection is Enqueue().
	// If model.PushRequest is not nil, it will be Enqueued again once MarkDone has been called.
	processing map[*Connection]*PushRequest

	shuttingDown bool
}

func NewPushQueue() *PushQueue {
	return &PushQueue{
		pending:    make(map[*Connection]*PushRequest),
		processing: make(map[*Connection]*PushRequest),
		cond:       sync.NewCond(&sync.Mutex{}),
	}
}

// Enqueue will mark a proxy as pending a push. If it is already pending, pushInfo will be merged.
// ServiceEntry updates will be added together, and full will be set if either were full
func (p *PushQueue) Enqueue(con *Connection, pushRequest *PushRequest) {
	p.cond.L.Lock()
	defer p.cond.L.Unlock()

	if p.shuttingDown {
		return
	}

	// If its already in progress, merge the info and return
	if request, f := p.processing[con]; f {
		p.processing[con] = request.Merge(pushRequest)
		return
	}

	if request, f := p.pending[con]; f {
		p.pending[con] = request.Merge(pushRequest)
		return
	}

	p.pending[con] = pushRequest
	p.queue = append(p.queue, con)
	// Signal waiters on Dequeue that a new item is available
	p.cond.Signal()
}

// Remove a proxy from the queue. If there are no proxies ready to be removed, this will block
func (p *PushQueue) Dequeue() (con *Connection, request *PushRequest, shutdown bool) {
	p.cond.L.Lock()
	defer p.cond.L.Unlock()

	// Block until there is one to remove. Enqueue will signal when one is added.
	for len(p.queue) == 0 && !p.shuttingDown {
		p.cond.Wait()
	}

	if len(p.queue) == 0 {
		// We must be shutting down.
		return nil, nil, true
	}

	con = p.queue[0]
	// The underlying array will still exist, despite the slice changing, so the object may not GC without this
	// See https://github.com/grpc/grpc-go/issues/4758
	p.queue[0] = nil
	p.queue = p.queue[1:]

	request = p.pending[con]
	delete(p.pending, con)

	// Mark the connection as in progress
	p.processing[con] = nil

	return con, request, false
}

func (p *PushQueue) MarkDone(con *Connection) {
	p.cond.L.Lock()
	defer p.cond.L.Unlock()
	request := p.processing[con]
	delete(p.processing, con)

	// If the info is present, that means Enqueue was called while connection was not yet marked done.
	// This means we need to add it back to the queue.
	if request != nil {
		p.pending[con] = request
		p.queue = append(p.queue, con)
		p.cond.Signal()
	}
}

// Get number of pending proxies
func (p *PushQueue) Pending() int {
	p.cond.L.Lock()
	defer p.cond.L.Unlock()
	return len(p.queue)
}

// ShutDown will cause queue to ignore all new items added to it. As soon as the
// worker goroutines have drained the existing items in the queue, they will be
// instructed to exit.
func (p *PushQueue) ShutDown() {
	p.cond.L.Lock()
	defer p.cond.L.Unlock()
	p.shuttingDown = true
	p.cond.Broadcast()
}
