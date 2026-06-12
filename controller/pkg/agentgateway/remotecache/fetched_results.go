package remotecache

import (
	"istio.io/istio/pkg/kube/krt"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/remotehttp"
)

// FetchedResults is the KRT-visible store of successfully fetched remote artifacts.
type FetchedResults[R Result[R]] struct {
	collection krt.StaticCollection[R]
}

func NewFetchedResults[R Result[R]](opts ...krt.CollectionOption) *FetchedResults[R] {
	return &FetchedResults[R]{
		collection: krt.NewStaticCollection[R](nil, nil, opts...),
	}
}

func (r *FetchedResults[R]) Collection() krt.Collection[R] {
	return r.collection
}

func (r *FetchedResults[R]) Get(key remotehttp.FetchKey) (R, bool) {
	var zero R
	obj := r.collection.GetKey(string(key))
	if obj == nil {
		return zero, false
	}
	return *obj, true
}

func (r *FetchedResults[R]) Put(result R) {
	r.collection.UpdateObject(result)
}

func (r *FetchedResults[R]) Delete(key remotehttp.FetchKey) {
	r.collection.DeleteObject(string(key))
}

func (r *FetchedResults[R]) DeleteObjects(filter func(R) bool) {
	r.collection.DeleteObjects(filter)
}

func (r *FetchedResults[R]) Reset(results []R) {
	r.collection.Reset(results)
}
