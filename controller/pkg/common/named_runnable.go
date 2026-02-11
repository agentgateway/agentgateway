package common

import "sigs.k8s.io/controller-runtime/pkg/manager"

type NamedRunnable interface {
	manager.Runnable
	RunnableName() string
}
