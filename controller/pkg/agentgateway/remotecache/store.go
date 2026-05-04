package remotecache

import (
	"context"
	"fmt"
	"log/slog"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/util/sets"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// FetchKeyIndexCollectionOption is a krt CollectionOption that keys an
// index collection by FetchKey string. Both the JWKS and OIDC collection
// pipelines use this to collapse per-owner sources onto a shared request key.
var FetchKeyIndexCollectionOption = krt.WithIndexCollectionFromString(func(s string) remotehttp.FetchKey {
	return remotehttp.FetchKey(s)
})

// Hydrator loads previously persisted Results so the in-memory cache can
// serve last-known-good data before any remote fetches complete on startup.
type Hydrator[R Result] interface {
	LoadAll(ctx context.Context) ([]R, error)
}

// StoreOptions configures a generic Store.
type StoreOptions[S Request, R Result] struct {
	Fetcher  *Fetcher[S, R]
	Requests krt.Collection[S]
	Logger   *slog.Logger

	// Hydrator, if non-nil, populates Fetcher.Cache before request
	// registration. LoadAll errors are logged but never block Start so a
	// transient persistence failure cannot stall fresh fetches.
	Hydrator Hydrator[R]

	// RetireOnRequestKeyChange, if true, calls Fetcher.Retire when a request's
	// key changes. This stops fetching the old key but preserves its cache
	// entry so existing traffic can continue using last-known-good data
	// until a newer fetch for the same resource (under a different key)
	// or an orphan sweep removes it.
	RetireOnRequestKeyChange bool
}

// Store bridges KRT-derived shared fetch requests to a runtime Fetcher.
type Store[S Request, R Result] struct {
	Fetcher *Fetcher[S, R]
	opts    StoreOptions[S, R]
	ready   chan struct{}
}

func NewStore[S Request, R Result](opts StoreOptions[S, R]) *Store[S, R] {
	return &Store[S, R]{
		Fetcher: opts.Fetcher,
		opts:    opts,
		ready:   make(chan struct{}),
	}
}

func (s *Store[S, R]) Start(ctx context.Context) error {
	s.opts.Logger.Info("starting remote fetch store")

	if s.opts.Hydrator != nil {
		stored, err := s.opts.Hydrator.LoadAll(ctx)
		if err != nil {
			s.opts.Logger.Error("error hydrating remote cache from persistence", "error", err)
		}
		for _, result := range stored {
			s.opts.Fetcher.Cache.Put(result)
		}
	}

	registration := s.opts.Requests.Register(func(event krt.Event[S]) {
		switch event.Event {
		case controllers.EventAdd, controllers.EventUpdate:
			if event.New == nil {
				return
			}
			// Special handling for request key changes (retire old)
			if s.opts.RetireOnRequestKeyChange && event.Event == controllers.EventUpdate && event.Old != nil {
				oldKey := (*event.Old).RemoteRequestKey()
				newKey := (*event.New).RemoteRequestKey()
				if oldKey != newKey {
					s.opts.Logger.Debug("retiring stale record after request key change", "old_request_key", oldKey, "new_request_key", newKey)
					s.opts.Fetcher.Retire(oldKey)
				}
			}

			s.opts.Fetcher.AddOrUpdate(*event.New)
		case controllers.EventDelete:
			if event.Old == nil {
				return
			}
			s.opts.Fetcher.Remove((*event.Old).RemoteRequestKey())
		}
	})
	defer registration.UnregisterHandler()

	go s.opts.Fetcher.Run(ctx)

	if !registration.WaitUntilSynced(ctx.Done()) {
		// WaitUntilSynced returns false either because ctx was cancelled
		// (graceful shutdown — surface ctx.Err() so the supervisor sees the
		// real cause) or because sync failed for non-context reasons. In the
		// latter case return an explicit error so the supervisor doesn't mark
		// this Runnable healthy.
		if err := ctx.Err(); err != nil {
			return err
		}
		return fmt.Errorf("remote fetch store: request collection did not sync")
	}

	s.opts.Fetcher.SweepOrphans()

	close(s.ready)
	<-ctx.Done()
	return nil
}

func (s *Store[S, R]) HasSynced() bool {
	select {
	case <-s.ready:
		return true
	default:
		return false
	}
}

func (s *Store[S, R]) SubscribeToUpdates() <-chan sets.Set[remotehttp.FetchKey] {
	return s.opts.Fetcher.SubscribeToUpdates()
}

func (s *Store[S, R]) NeedLeaderElection() bool {
	return true
}
