package krtutil

import "istio.io/istio/pkg/kube/krt"

type KrtOptions struct {
	Stop     <-chan struct{}
	Debugger *krt.DebugHandler
	// namePrefix, if set, will prefix every name with the common prefix.
	// For example `<namePrefix>/<name>`.
	namePrefix string
}

func NewKrtOptions(stop <-chan struct{}, debugger *krt.DebugHandler) KrtOptions {
	return KrtOptions{
		Stop:     stop,
		Debugger: debugger,
	}
}

func (k KrtOptions) ToIstio() krt.OptionsBuilder {
	return krt.NewOptionsBuilder(k.Stop, k.namePrefix, k.Debugger)
}

func (k KrtOptions) WithPrefix(name string) KrtOptions {
	k.namePrefix = name
	return k
}

func (k KrtOptions) ToOptions(name string) []krt.CollectionOption {
	if k.namePrefix != "" {
		name = k.namePrefix + "/" + name
	}
	return []krt.CollectionOption{
		krt.WithName(name),
		krt.WithDebugging(k.Debugger),
		krt.WithStop(k.Stop),
	}
}
