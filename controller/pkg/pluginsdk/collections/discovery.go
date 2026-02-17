// Parts of this file are borrowed from https://github.com/istio/istio/blob/1.26.0/pkg/kube/namespace/filter.go
//
// Copyright Istio Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

package collections

import (
	"encoding/json"
	"fmt"
	"sync"

	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/controllers"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/kubetypes"
	"istio.io/istio/pkg/slices"
	"istio.io/istio/pkg/util/sets"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/labels"
)

type discoveryNamespacesFilter struct {
	lock                sync.RWMutex
	namespaces          kclient.Client[*corev1.Namespace]
	discoveryNamespaces sets.String
	discoverySelectors  []labels.Selector // nil if discovery selectors are not specified, permits all namespaces for discovery
	handlers            []func(added, removed sets.String)
}

// NewDiscoveryNamespacesFilter creates a new DynamicObjectFilter that filters namespaced objects based
// on the given discovery namespace selector config JSON (cfgJSON).
func NewDiscoveryNamespacesFilter(
	namespaces kclient.Client[*corev1.Namespace],
	cfgJSON string,
	stop <-chan struct{},
) (kubetypes.DynamicObjectFilter, error) {
	// If the config is unset, default to empty JSON array which selects all namespaces.
	// This prevents needing to pass an empty JSON array from tests, which would otherwise result
	// in an unmarshalling error.
	if cfgJSON == "" {
		cfgJSON = "[]"
	}

	// convert LabelSelectors to Selectors
	var labelSelectors []metav1.LabelSelector
	err := json.Unmarshal([]byte(cfgJSON), &labelSelectors)
	if err != nil {
		return nil, fmt.Errorf("error parsing discovery selectors: %v; %w", cfgJSON, err)
	}
	selectors, err := toSelectors(labelSelectors)
	if err != nil {
		return nil, fmt.Errorf("error parsing discovery selectors: %v; %w", cfgJSON, err)
	}
	f := &discoveryNamespacesFilter{
		namespaces:          namespaces,
		discoveryNamespaces: sets.New[string](),
		discoverySelectors:  selectors,
	}

	namespaces.AddEventHandler(controllers.EventHandler[*corev1.Namespace]{
		AddFunc: func(ns *corev1.Namespace) {
			f.lock.Lock()
			created := f.namespaceCreatedLocked(ns.ObjectMeta)
			f.lock.Unlock()
			// In rare cases, a namespace may be created after objects in the namespace, because there is no synchronization between watches
			// So we need to notify if we started selecting namespace
			if created {
				f.notifyHandlers(sets.New(ns.Name), nil)
			}
		},
		UpdateFunc: func(oldObj, newObj *corev1.Namespace) {
			f.lock.Lock()
			membershipChanged, namespaceAdded := f.namespaceUpdatedLocked(oldObj.ObjectMeta, newObj.ObjectMeta)
			f.lock.Unlock()
			if membershipChanged {
				added := sets.New(newObj.Name)
				var removed sets.String
				if !namespaceAdded {
					removed = added
					added = nil
				}
				f.notifyHandlers(added, removed)
			}
		},
		DeleteFunc: func(ns *corev1.Namespace) {
			f.lock.Lock()
			defer f.lock.Unlock()
			// No need to notify handlers for deletes. The namespace was deleted, so the object will be as well (and a delete could not de-select).
			// Note that specifically for the edge case of a Namespace watcher that is filtering, this will ignore deletes we should
			// otherwise send.
			// See kclient.applyDynamicFilter for rationale.
			f.namespaceDeletedLocked(ns.ObjectMeta)
		},
	})

	// Start namespaces and wait for it to be ready now.
	// The filter is required by other watchers, so we want to block.
	namespaces.Start(stop)
	kube.WaitForCacheSync("discovery namespace filter", stop, namespaces.HasSynced)
	return f, nil
}

// Filter implements kubetypes.DynamicObjectFilter's Filter interface.
// It returns true if the object is in a namespace selected for discovery.
func (d *discoveryNamespacesFilter) Filter(obj any) bool {
	// When an object is deleted, obj could be a DeletionFinalStateUnknown marker item.
	ns, ok := extractObjectNamespace(obj)
	if !ok {
		return false
	}
	if ns == "" {
		// Cluster scoped resources. Always included
		return true
	}

	d.lock.RLock()
	defer d.lock.RUnlock()
	// permit all objects if discovery selectors are not specified
	if len(d.discoverySelectors) == 0 {
		return true
	}

	// permit if object resides in a namespace labeled for discovery
	return d.discoveryNamespaces.Contains(ns)
}

// AddHandler implements kubetypes.DynamicObjectFilter's AddHandler interface.
// It registers a handler on namespace, which will be triggered when namespace selected or deselected.
// If the namespaces have been synced, trigger the new added handler.
func (d *discoveryNamespacesFilter) AddHandler(f func(added, removed sets.String)) {
	d.lock.Lock()
	defer d.lock.Unlock()
	d.handlers = append(d.handlers, f)
}

func toSelectors(selectors []metav1.LabelSelector) ([]labels.Selector, error) {
	out := make([]labels.Selector, 0, len(selectors))
	for _, selector := range selectors {
		sel, err := metav1.LabelSelectorAsSelector(&selector)
		if err != nil {
			return nil, err
		}
		out = append(out, sel)
	}
	return out, nil
}

func (d *discoveryNamespacesFilter) notifyHandlers(added sets.Set[string], removed sets.String) {
	// Clone handlers; we handle dynamic handlers so they can change after the filter has started.
	// Important: handlers are not called under the lock. If they are, then handlers which eventually call discoveryNamespacesFilter.Filter
	// (as some do in the codebase currently, via kclient.List), will deadlock.
	d.lock.RLock()
	handlers := slices.Clone(d.handlers)
	d.lock.RUnlock()
	for _, h := range handlers {
		h(added, removed)
	}
}

func extractObjectNamespace(obj any) (string, bool) {
	if ns, ok := obj.(string); ok {
		return ns, true
	}
	object := controllers.ExtractObject(obj)
	if object == nil {
		// When an object is deleted, obj could be a DeletionFinalStateUnknown marker item.
		return "", false
	}
	if _, ok := object.(*corev1.Namespace); ok {
		return object.GetName(), true
	}
	return object.GetNamespace(), true
}

// namespaceCreated: if newly created namespace is selected, update namespace membership
func (d *discoveryNamespacesFilter) namespaceCreatedLocked(ns metav1.ObjectMeta) (membershipChanged bool) {
	if d.isSelectedLocked(ns.Labels) {
		d.discoveryNamespaces.Insert(ns.Name)
		// Do not trigger update when there are no selectors. This avoids possibility of double namespace ADDs
		return len(d.discoverySelectors) != 0
	}
	return false
}

// namespaceUpdatedLocked : if updated namespace was a member and no longer selected, or was not a member and now selected, update namespace membership
func (d *discoveryNamespacesFilter) namespaceUpdatedLocked(oldNs, newNs metav1.ObjectMeta) (membershipChanged bool, namespaceAdded bool) {
	if d.discoveryNamespaces.Contains(oldNs.Name) && !d.isSelectedLocked(newNs.Labels) {
		d.discoveryNamespaces.Delete(oldNs.Name)
		return true, false
	}
	if !d.discoveryNamespaces.Contains(oldNs.Name) && d.isSelectedLocked(newNs.Labels) {
		d.discoveryNamespaces.Insert(oldNs.Name)
		return true, true
	}
	return false, false
}

// namespaceDeletedLocked : if deleted namespace was a member, remove it
func (d *discoveryNamespacesFilter) namespaceDeletedLocked(ns metav1.ObjectMeta) {
	d.discoveryNamespaces.Delete(ns.Name)
}

func (d *discoveryNamespacesFilter) isSelectedLocked(labels labels.Set) bool {
	// permit all objects if discovery selectors are not specified
	if len(d.discoverySelectors) == 0 {
		return true
	}

	for _, selector := range d.discoverySelectors {
		if selector.Matches(labels) {
			return true
		}
	}

	return false
}
