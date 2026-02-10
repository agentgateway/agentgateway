// Derived from https://github.com/istio/istio/blob/master/pilot/pkg/status/resourcelock.go# (Apache 2.0)
// Minor changes to reduce Istio-isms.

package status

import (
	"context"
	"sync"

	"k8s.io/apimachinery/pkg/runtime/schema"
	"k8s.io/apimachinery/pkg/types"
)

type Resource struct {
	schema.GroupVersionKind
	types.NamespacedName
	ResourceVersion string
}

// WorkerQueue implements an expandable goroutine pool which executes at most one concurrent routine per target
// resource.  Multiple calls to Push() will not schedule multiple executions per target resource, but will ensure that
// the single execution uses the latest value.
type WorkerQueue interface {
	// Push a task.
	Push(target Resource, data any)
	// Run the loop until a signal on the context
	Run(ctx context.Context)
}

type WorkQueue struct {
	// a lock to govern access to data in the cache
	mu sync.Mutex
	// queue maintains all pending items awaiting processing
	queue []Resource
	// pending stores information about each item in the queue
	pending map[Resource]any

	// processing stores all resources that have been Dequeue(), but not MarkDone().
	// The value stored will be initially be nil, but may be populated if the connection is Enqueue().
	// If the value is not nil, it will be Enqueued again once MarkDone has been called.
	// This lets us build up pending data while ensuring we don't process the same key concurrently
	processing map[Resource]any

	shuttingDown bool
}

// Enqueue will mark a proxy as pending a push. If it is already pending, pushInfo will be merged.
// ServiceEntry updates will be added together, and full will be set if either were full
func (p *WorkQueue) Enqueue(con Resource, data any) {
	p.mu.Lock()
	defer p.mu.Unlock()

	if p.shuttingDown {
		return
	}

	// If its already in progress, replace the info and return
	if _, f := p.processing[con]; f {
		p.processing[con] = data
		return
	}

	// We already have this item waiting, replace it with the latest data
	if _, f := p.pending[con]; f {
		p.pending[con] = data
		return
	}

	p.pending[con] = data
	p.queue = append(p.queue, con)
}

// Remove a proxy from the queue. If there are no proxies ready to be removed, this will block
func (p *WorkQueue) Dequeue() (r Resource, d any, ok bool) {
	p.mu.Lock()
	defer p.mu.Unlock()

	// Block until there is one to remove. Enqueue will signal when one is added.
	for len(p.queue) == 0 && !p.shuttingDown {
		return Resource{}, nil, false
	}

	if len(p.queue) == 0 {
		// We must be shutting down.
		return Resource{}, nil, false
	}

	con := p.queue[0]
	// The underlying array will still exist, despite the slice changing, so the object may not GC without this
	p.queue = p.queue[1:]

	data := p.pending[con]
	delete(p.pending, con)

	// Mark the connection as in progress
	p.processing[con] = nil

	return con, data, true
}

func (p *WorkQueue) MarkDone(con Resource) {
	p.mu.Lock()
	defer p.mu.Unlock()
	request := p.processing[con]
	delete(p.processing, con)

	// If the info is present, that means Enqueue was called while connection was not yet marked done.
	// This means we need to add it back to the queue.
	if request != nil {
		p.pending[con] = request
		p.queue = append(p.queue, con)
	}
}

func (p *WorkQueue) ShutDown() {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.shuttingDown = true
}

// Get number of pending proxies
func (p *WorkQueue) Length() int {
	p.mu.Lock()
	defer p.mu.Unlock()
	return len(p.queue)
}

func NewWorkerPool(ctx context.Context, work func(ctx context.Context, resource Resource, data any), maxWorkers uint) *WorkerPool {
	wp := &WorkerPool{
		work:       work,
		maxWorkers: maxWorkers,
		ctx:        ctx,
		q: WorkQueue{
			pending:    make(map[Resource]any),
			processing: make(map[Resource]any),
		},
	}
	context.AfterFunc(ctx, func() {
		wp.lock.Lock()
		wp.closing = true
		wp.lock.Unlock()
	})
	return wp
}

type WorkerPool struct {
	q WorkQueue
	// indicates the queue is closing
	closing bool
	// the function which will be run for each task in queue
	work func(ctx context.Context, resource Resource, data any)
	// current worker routine count
	workerCount uint
	// maximum worker routine count
	maxWorkers uint
	lock       sync.Mutex
	ctx        context.Context
}

func (wp *WorkerPool) Push(target Resource, data any) {
	wp.q.Enqueue(target, data)
	wp.maybeAddWorker()
}

func (wp *WorkerPool) Run(ctx context.Context) {
	context.AfterFunc(ctx, func() {
		wp.lock.Lock()
		wp.closing = true
		wp.lock.Unlock()
	})
}

// maybeAddWorker adds a worker unless we are at maxWorkers.  Workers exit when there are no more tasks, except for the
// last worker, which stays alive indefinitely.
func (wp *WorkerPool) maybeAddWorker() {
	wp.lock.Lock()
	if wp.workerCount >= wp.maxWorkers || wp.q.Length() == 0 {
		wp.lock.Unlock()
		return
	}
	wp.workerCount++
	wp.lock.Unlock()
	go func() {
		for {
			wp.lock.Lock()
			if wp.closing || wp.q.Length() == 0 {
				wp.workerCount--
				wp.lock.Unlock()
				return
			}

			res, data, ok := wp.q.Dequeue()
			if !ok {
				// No items in queue not currently worked on
				wp.lock.Unlock()
				return
			}
			wp.lock.Unlock()

			// work should be done without holding the lock
			wp.work(wp.ctx, res, data)
			wp.q.MarkDone(res)
		}
	}()
}
