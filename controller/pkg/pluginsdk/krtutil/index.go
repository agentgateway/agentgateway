package krtutil

import (
	"fmt"

	"istio.io/istio/pkg/kube/krt"
)

func FetchIndexObjects[K comparable, O any](ctx krt.HandlerContext, index krt.IndexCollection[K, O], name K) []O {
	res := krt.FetchOne(ctx, index, krt.FilterKey(toString(name)))
	if res == nil {
		return nil
	}
	return res.Objects
}

func toString(rk any) string {
	tk, ok := rk.(string)
	if !ok {
		return rk.(fmt.Stringer).String()
	}
	return tk
}
