package remotecache

import (
	"sync"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"istio.io/istio/pkg/util/sets"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

const (
	keyA remotehttp.FetchKey = "a"
	keyB remotehttp.FetchKey = "b"
	keyC remotehttp.FetchKey = "c"
)

func TestSchedulePopDueOrdersByTime(t *testing.T) {
	s := NewSchedule()
	now := time.Unix(1_000_000, 0)

	s.Schedule(keyA, 1, now.Add(30*time.Second), 0)
	s.Schedule(keyB, 1, now.Add(10*time.Second), 0)
	s.Schedule(keyC, 1, now.Add(20*time.Second), 0)

	require.Equal(t, 3, s.Len())

	due := s.PopDue(now.Add(25 * time.Second))
	require.Len(t, due, 2)
	require.Equal(t, keyB, due[0].RequestKey)
	require.Equal(t, keyC, due[1].RequestKey)
	require.Equal(t, 1, s.Len())

	next, ok := s.Peek()
	require.True(t, ok)
	require.Equal(t, keyA, next.RequestKey)
}

func TestSchedulePeekReturnsCopyNotPointer(t *testing.T) {
	s := NewSchedule()
	now := time.Now()
	s.Schedule(keyA, 1, now, 0)

	first, ok := s.Peek()
	require.True(t, ok)
	first.RequestKey = "mutated"
	first.Generation = 999

	second, ok := s.Peek()
	require.True(t, ok)
	require.Equal(t, keyA, second.RequestKey, "Peek must not expose internal pointer")
	require.Equal(t, uint64(1), second.Generation)
}

func TestScheduleReplaceExistingEntryUpdatesInPlace(t *testing.T) {
	s := NewSchedule()
	now := time.Now()
	s.Schedule(keyA, 1, now.Add(time.Hour), 0)
	s.Schedule(keyA, 2, now.Add(time.Second), 5)

	require.Equal(t, 1, s.Len())
	next, ok := s.Peek()
	require.True(t, ok)
	require.Equal(t, uint64(2), next.Generation)
	require.Equal(t, 5, next.RetryAttempt)
	require.Equal(t, now.Add(time.Second), next.At)
}

func TestScheduleRemoveDropsEntry(t *testing.T) {
	s := NewSchedule()
	now := time.Now()
	s.Schedule(keyA, 1, now, 0)
	s.Schedule(keyB, 1, now, 0)

	s.Remove(keyA)

	require.Equal(t, 1, s.Len())
	next, ok := s.Peek()
	require.True(t, ok)
	require.Equal(t, keyB, next.RequestKey)

	s.Remove(keyA)
	require.Equal(t, 1, s.Len())
}

func TestSchedulePeekEmpty(t *testing.T) {
	s := NewSchedule()
	_, ok := s.Peek()
	require.False(t, ok)
	require.Equal(t, 0, s.Len())
}

func TestNextRetryDelayCapsWithoutOverflow(t *testing.T) {
	require.Equal(t, 200*time.Millisecond, NextRetryDelay(0))
	require.Equal(t, MaxRetryDelay, NextRetryDelay(7))
	require.Equal(t, MaxRetryDelay, NextRetryDelay(36))
	require.Equal(t, MaxRetryDelay, NextRetryDelay(MaxRetryShift+100))
}

func TestNextRetryDelayMonotonicUntilCap(t *testing.T) {
	prev := time.Duration(0)
	hitCap := false
	for i := range 10 {
		d := NextRetryDelay(i)
		if d == MaxRetryDelay {
			hitCap = true
			break
		}
		require.Greater(t, d, prev, "delay should grow until cap")
		prev = d
	}
	require.True(t, hitCap, "should reach cap within 10 attempts")
}

func TestUpdateFanoutNotifyDeliversToEachSubscriber(t *testing.T) {
	f := NewUpdateFanout()
	a := f.Subscribe()
	b := f.Subscribe()

	f.Notify(sets.New(keyA))

	require.True(t, (<-a).Contains(keyA))
	require.True(t, (<-b).Contains(keyA))
}

func TestUpdateFanoutMergesPendingUpdates(t *testing.T) {
	f := NewUpdateFanout()
	updates := f.Subscribe()

	f.Notify(sets.New(keyA))
	f.Notify(sets.New(keyB))

	merged := <-updates
	require.True(t, merged.Contains(keyA))
	require.True(t, merged.Contains(keyB))
}

func TestUpdateFanoutNotifyConcurrentSafe(t *testing.T) {
	f := NewUpdateFanout()
	updates := f.Subscribe()

	const goroutines = 50
	var wg sync.WaitGroup
	wg.Add(goroutines)
	for i := range goroutines {
		go func(i int) {
			defer wg.Done()
			f.Notify(sets.New(remotehttp.FetchKey(string(rune('A' + i%26)))))
		}(i)
	}
	wg.Wait()

	drained := false
	for !drained {
		select {
		case <-updates:
		case <-time.After(50 * time.Millisecond):
			drained = true
		}
	}
}

func TestUpdateFanoutSubscribeDuringNotifyDoesNotPanic(t *testing.T) {
	f := NewUpdateFanout()
	f.Subscribe()

	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		for range 100 {
			f.Notify(sets.New(keyA))
		}
	}()
	go func() {
		defer wg.Done()
		for range 100 {
			f.Subscribe()
		}
	}()
	wg.Wait()
}

func TestSignalWakeNonBlocking(t *testing.T) {
	wake := make(chan struct{}, 1)
	SignalWake(wake)
	SignalWake(wake)
	SignalWake(wake)

	select {
	case <-wake:
	default:
		t.Fatal("expected wake to receive signal")
	}
	select {
	case <-wake:
		t.Fatal("buffer should hold at most one signal")
	default:
	}
}

func TestDrainTimerStopped(t *testing.T) {
	timer := time.NewTimer(time.Hour)
	DrainTimer(timer)
	select {
	case <-timer.C:
		t.Fatal("drained timer should not fire")
	case <-time.After(50 * time.Millisecond):
	}
}

func TestDrainTimerExpired(t *testing.T) {
	timer := time.NewTimer(time.Microsecond)
	time.Sleep(10 * time.Millisecond)
	DrainTimer(timer)
	select {
	case <-timer.C:
		t.Fatal("expired-then-drained timer should not deliver")
	case <-time.After(20 * time.Millisecond):
	}
}
