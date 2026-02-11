package krtxds

import (
	"fmt"
	"reflect"
	"sort"
	"strconv"
	"sync"
	"testing"
	"time"

	"istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/util/sets"
)

// Helper function to remove an item or timeout and return nil if there are no pending pushes
func getWithTimeout(p *PushQueue) *Connection {
	done := make(chan *Connection, 1)
	go func() {
		con, _, _ := p.Dequeue()
		done <- con
	}()
	select {
	case ret := <-done:
		return ret
	case <-time.After(time.Millisecond * 500):
		return nil
	}
}

func ExpectTimeout(t *testing.T, p *PushQueue) {
	t.Helper()
	done := make(chan struct{}, 1)
	go func() {
		p.Dequeue()
		done <- struct{}{}
	}()
	select {
	case <-done:
		t.Fatal("Expected timeout")
	case <-time.After(time.Millisecond * 500):
	}
}

func ExpectDequeue(t *testing.T, p *PushQueue, expected *Connection) {
	t.Helper()
	result := make(chan *Connection, 1)
	go func() {
		con, _, _ := p.Dequeue()
		result <- con
	}()
	select {
	case got := <-result:
		if got != expected {
			t.Fatalf("Expected proxy %v, got %v", expected, got)
		}
	case <-time.After(time.Millisecond * 500):
		t.Fatal("Timed out")
	}
}

func TestProxyQueue(t *testing.T) {
	proxies := make([]*Connection, 0, 100)
	for p := range 100 {
		conn := &Connection{}
		conn.SetID(fmt.Sprintf("proxy-%d", p))
		proxies = append(proxies, conn)
	}

	t.Run("simple add and remove", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()
		p.Enqueue(proxies[0], &PushRequest{})
		p.Enqueue(proxies[1], &PushRequest{})

		ExpectDequeue(t, p, proxies[0])
		ExpectDequeue(t, p, proxies[1])
	})

	t.Run("remove too many", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()

		p.Enqueue(proxies[0], &PushRequest{})

		ExpectDequeue(t, p, proxies[0])
		ExpectTimeout(t, p)
	})

	t.Run("add multiple times", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()

		p.Enqueue(proxies[0], &PushRequest{})
		p.Enqueue(proxies[1], &PushRequest{})
		p.Enqueue(proxies[0], &PushRequest{})

		ExpectDequeue(t, p, proxies[0])
		ExpectDequeue(t, p, proxies[1])
		ExpectTimeout(t, p)
	})

	t.Run("add and remove and markdone", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()

		p.Enqueue(proxies[0], &PushRequest{})
		ExpectDequeue(t, p, proxies[0])
		p.MarkDone(proxies[0])
		p.Enqueue(proxies[0], &PushRequest{})
		ExpectDequeue(t, p, proxies[0])
		ExpectTimeout(t, p)
	})

	t.Run("add and remove and add and markdone", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()

		p.Enqueue(proxies[0], &PushRequest{})
		ExpectDequeue(t, p, proxies[0])
		p.Enqueue(proxies[0], &PushRequest{})
		p.Enqueue(proxies[0], &PushRequest{})
		p.MarkDone(proxies[0])

		ExpectDequeue(t, p, proxies[0])
		ExpectTimeout(t, p)
	})

	t.Run("remove should block", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()

		wg := &sync.WaitGroup{}
		wg.Go(func() {
			ExpectDequeue(t, p, proxies[0])
		})
		time.Sleep(time.Millisecond * 50)
		p.Enqueue(proxies[0], &PushRequest{})
		wg.Wait()
	})

	t.Run("should merge PushRequest", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()

		p.Enqueue(proxies[0], &PushRequest{
			ConfigsUpdated: map[TypeUrl]sets.String{
				"type1": sets.New("foo"),
			},
		})

		p.Enqueue(proxies[0], &PushRequest{
			ConfigsUpdated: map[TypeUrl]sets.String{
				"type1": sets.New("bar"),
				"type2": sets.New("baz"),
			},
		})
		_, info, _ := p.Dequeue()

		assert.Equal(t, info.ConfigsUpdated, map[TypeUrl]sets.String{
			"type1": sets.New("bar", "foo"),
			"type2": sets.New("baz"),
		})
	})

	t.Run("two removes, one should block one should return", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()

		wg := &sync.WaitGroup{}
		wg.Add(2)
		respChannel := make(chan *Connection, 2)
		go func() {
			respChannel <- getWithTimeout(p)
			wg.Done()
		}()
		time.Sleep(time.Millisecond * 50)
		p.Enqueue(proxies[0], &PushRequest{})
		go func() {
			respChannel <- getWithTimeout(p)
			wg.Done()
		}()

		wg.Wait()
		timeouts := 0
		close(respChannel)
		for resp := range respChannel {
			if resp == nil {
				timeouts++
			}
		}
		if timeouts != 1 {
			t.Fatalf("Expected 1 timeout, got %v", timeouts)
		}
	})

	t.Run("concurrent", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()

		key := func(p *Connection, eds string) string { return fmt.Sprintf("%s~%s", p.ID(), eds) }

		// We will trigger many pushes for eds services to each proxy. In the end we will expect
		// all of these to be dequeue, but order is not deterministic.
		expected := sets.String{}
		for eds := range 100 {
			for _, pr := range proxies {
				expected.Insert(key(pr, fmt.Sprintf("%d", eds)))
			}
		}
		go func() {
			for eds := range 100 {
				for _, pr := range proxies {
					p.Enqueue(pr, &PushRequest{
						ConfigsUpdated: map[TypeUrl]sets.String{
							"type1": sets.New(fmt.Sprintf("%d", eds)),
						},
					})
				}
			}
		}()

		done := make(chan struct{})
		mu := sync.RWMutex{}
		go func() {
			for {
				con, info, shuttingdown := p.Dequeue()
				if shuttingdown {
					return
				}
				for eds := range info.ConfigsUpdated["type1"] {
					mu.Lock()
					delete(expected, key(con, eds))
					mu.Unlock()
				}
				p.MarkDone(con)
				if len(expected) == 0 {
					done <- struct{}{}
				}
			}
		}()

		select {
		case <-done:
		case <-time.After(time.Second * 10):
			mu.RLock()
			defer mu.RUnlock()
			t.Fatalf("failed to get all updates, still pending: %v", len(expected))
		}
	})

	t.Run("concurrent with deterministic order", func(t *testing.T) {
		t.Parallel()
		p := NewPushQueue()
		defer p.ShutDown()

		con := &Connection{}
		con.SetID("proxy-test")

		// We will trigger many pushes for eds services to the proxy. In the end we will expect
		// all of these to be dequeue, but order is deterministic.
		expected := make([]string, 100)
		for eds := range 100 {
			expected[eds] = fmt.Sprintf("%d", eds)
		}
		go func() {
			// send to pushQueue
			for eds := range 100 {
				p.Enqueue(con, &PushRequest{
					ConfigsUpdated: map[TypeUrl]sets.String{
						"type1": sets.New(fmt.Sprintf("%d", eds)),
					},
				})
			}
		}()

		processed := make([]string, 0, 100)
		done := make(chan struct{})
		pushChannel := make(chan *PushRequest)
		go func() {
			// dequeue pushQueue and send to pushChannel
			for {
				_, request, shuttingdown := p.Dequeue()
				if shuttingdown {
					close(pushChannel)
					return
				}
				pushChannel <- request
			}
		}()

		go func() {
			// recv from pushChannel and simulate push
			for {
				request := <-pushChannel
				if request == nil {
					return
				}
				updated := make([]string, 0, len(request.ConfigsUpdated))
				for _, configkeys := range request.ConfigsUpdated {
					for configkey := range configkeys {
						updated = append(updated, configkey)
					}
				}
				sort.Slice(updated, func(i, j int) bool {
					l, _ := strconv.Atoi(updated[i])
					r, _ := strconv.Atoi(updated[j])
					return l < r
				})
				processed = append(processed, updated...)
				if len(processed) == 100 {
					done <- struct{}{}
				}
				p.MarkDone(con)
			}
		}()

		select {
		case <-done:
		case <-time.After(time.Second * 10):
			t.Fatalf("failed to get all updates, still pending:  got %v", len(processed))
		}

		if !reflect.DeepEqual(expected, processed) {
			t.Fatalf("expected order %v, but got %v", expected, processed)
		}
	})
}
