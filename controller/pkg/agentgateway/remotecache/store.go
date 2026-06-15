package remotecache

import (
	"context"
	"fmt"
	"log/slog"

	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// FetchKeyIndexCollectionOption keys an index collection by FetchKey.
var FetchKeyIndexCollectionOption = krt.WithIndexCollectionFromString(func(s string) remotehttp.FetchKey {
	return remotehttp.FetchKey(s)
})

type StoreOptions[S Request, R Result[R]] struct {
	Fetcher  *Fetcher[S, R]
	Requests krt.Collection[S]
	Logger   *slog.Logger

	// Hydrate, if non-nil, populates Fetcher.Results before request
	// registration. Errors are logged but never block Start so a transient
	// persistence failure cannot stall fresh fetches.
	Hydrate func(ctx context.Context) ([]R, error)
}

// Store bridges KRT-derived shared fetch requests to a runtime Fetcher.
type Store[S Request, R Result[R]] struct {
	Fetcher  *Fetcher[S, R]
	requests krt.Collection[S]
	logger   *slog.Logger
	hydrate  func(ctx context.Context) ([]R, error)
	ready    chan struct{}
}

func NewStore[S Request, R Result[R]](opts StoreOptions[S, R]) *Store[S, R] {
	return &Store[S, R]{
		Fetcher:  opts.Fetcher,
		requests: opts.Requests,
		logger:   opts.Logger,
		hydrate:  opts.Hydrate,
		ready:    make(chan struct{}),
	}
}

func (s *Store[S, R]) Start(ctx context.Context) error {
	s.logger.Info("starting remote fetch store")

	if s.hydrate != nil {
		stored, err := s.hydrate(ctx)
		if err != nil {
			s.logger.Error("error hydrating remote results from persistence", "error", err)
		}
		s.Fetcher.Results.Reset(stored)
	}

	registration := s.requests.Register(func(event krt.Event[S]) {
		switch event.Event {
		case controllers.EventAdd, controllers.EventUpdate:
			if event.New == nil {
				return
			}
			// Requests are keyed by RequestKey, so a key change arrives as
			// Delete(old)+Add(new), never as an Update with differing keys.
			s.Fetcher.AddOrUpdate(*event.New)
		case controllers.EventDelete:
			if event.Old == nil {
				return
			}
			s.Fetcher.Remove((*event.Old).RemoteRequestKey())
		}
	})
	defer registration.UnregisterHandler()

	go s.Fetcher.Run(ctx)

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

	s.Fetcher.SweepOrphans()

	close(s.ready)
	<-ctx.Done()
	return nil
}

// FetchedResults exposes the fetcher's result set; satisfies ResultStore for
// the subsystem Stores that embed this one.
func (s *Store[S, R]) FetchedResults() *FetchedResults[R] {
	return s.Fetcher.Results
}

func (s *Store[S, R]) HasSynced() bool {
	select {
	case <-s.ready:
		return true
	default:
		return false
	}
}

func (s *Store[S, R]) NeedLeaderElection() bool {
	return true
}
