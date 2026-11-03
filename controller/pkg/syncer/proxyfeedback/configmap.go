package proxyfeedback

import (
	"context"
	"encoding/json"
	"os"
	"slices"
	"time"

	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/kclient"
	"istio.io/istio/pkg/kube/krt"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/apiclient"
	"github.com/agentgateway/agentgateway/controller/pkg/logging"
	"github.com/agentgateway/agentgateway/controller/pkg/pluginsdk/krtutil"
)

const (
	configMapPrefix = "agentgateway-xds-feedback-"
	componentLabel  = "app.kubernetes.io/component"
	componentValue  = "xds-feedback"
	dataKey         = "diagnostics"

	// maxPublishedDiagnostics keeps a replica's ConfigMap well under the 1MiB object limit.
	maxPublishedDiagnostics = 500

	// startupDelay gives proxies time to reconnect and report again before a restarted replica
	// replaces what its previous process published.
	startupDelay = 30 * time.Second
	// publishDelay coalesces changes, such as a proxy reconnecting, into one write.
	publishDelay = 2 * time.Second
)

var logger = logging.New("proxy_feedback")

// Writer publishes this replica's tracked diagnostics to a ConfigMap. It runs on every replica,
// since each one serves its own proxy connections.
type Writer struct {
	tracker   *Tracker
	client    kube.Client
	namespace string
	name      string
	podName   string

	startupDelay time.Duration
	publishDelay time.Duration
}

func NewWriter(tracker *Tracker, client kube.Client, namespace string) *Writer {
	podName, _ := os.Hostname()
	return &Writer{
		tracker:   tracker,
		client:    client,
		namespace: namespace,
		name:      configMapPrefix + podName,
		podName:   podName,

		startupDelay: startupDelay,
		publishDelay: publishDelay,
	}
}

func (w *Writer) NeedLeaderElection() bool {
	return false
}

func (w *Writer) Start(ctx context.Context) error {
	owner := w.podOwnerReference(ctx)
	timer := time.NewTimer(w.startupDelay)
	defer timer.Stop()
	scheduled := true
	published := ""
	for {
		select {
		case <-ctx.Done():
			if owner == nil {
				// Without a Pod to own the ConfigMap, nothing else removes it.
				w.delete()
			}
			return nil
		case <-w.tracker.Changed():
			if !scheduled {
				timer.Reset(w.publishDelay)
				scheduled = true
			}
		case <-timer.C:
			scheduled = false
			data, err := encode(w.tracker.Snapshot())
			if err != nil {
				logger.Error("failed to encode proxy diagnostics", "error", err)
				continue
			}
			if data == published {
				continue
			}
			if err := w.publish(ctx, data, owner); err != nil {
				logger.Warn("failed to publish proxy diagnostics; retrying", "configmap", w.name, "error", err)
				timer.Reset(w.publishDelay)
				scheduled = true
				continue
			}
			published = data
		}
	}
}

func (w *Writer) publish(ctx context.Context, data string, owner *metav1.OwnerReference) error {
	configMaps := w.client.Kube().CoreV1().ConfigMaps(w.namespace)
	existing, err := configMaps.Get(ctx, w.name, metav1.GetOptions{})
	if apierrors.IsNotFound(err) {
		cm := &corev1.ConfigMap{
			Name:      w.name,
			Namespace: w.namespace,
			Labels:    map[string]string{componentLabel: componentValue},
			Data:      map[string]string{dataKey: data},
		}
		if owner != nil {
			cm.OwnerReferences = []metav1.OwnerReference{*owner}
		}
		_, err = configMaps.Create(ctx, cm, metav1.CreateOptions{})
		return err
	}
	if err != nil {
		return err
	}
	if existing.Data[dataKey] == data {
		return nil
	}
	existing.Data = map[string]string{dataKey: data}
	_, err = configMaps.Update(ctx, existing, metav1.UpdateOptions{})
	return err
}

func (w *Writer) delete() {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	err := w.client.Kube().CoreV1().ConfigMaps(w.namespace).Delete(ctx, w.name, metav1.DeleteOptions{})
	if err != nil && !apierrors.IsNotFound(err) {
		logger.Warn("failed to delete proxy diagnostics", "configmap", w.name, "error", err)
	}
}

// podOwnerReference lets Kubernetes delete the ConfigMap when this replica's Pod goes away.
func (w *Writer) podOwnerReference(ctx context.Context) *metav1.OwnerReference {
	pod, err := w.client.Kube().CoreV1().Pods(w.namespace).Get(ctx, w.podName, metav1.GetOptions{})
	if err != nil {
		logger.Info("controller Pod not found; proxy diagnostics will be deleted on shutdown", "pod", w.podName, "error", err)
		return nil
	}
	return &metav1.OwnerReference{
		APIVersion: "v1",
		Kind:       "Pod",
		Name:       pod.Name,
		UID:        pod.UID,
	}
}

func encode(diagnostics []Diagnostic) (string, error) {
	if len(diagnostics) > maxPublishedDiagnostics {
		logger.Warn("too many proxy diagnostics to publish; dropping the rest", "count", len(diagnostics), "limit", maxPublishedDiagnostics)
		diagnostics = diagnostics[:maxPublishedDiagnostics]
	}
	data, err := json.Marshal(diagnostics)
	return string(data), err
}

// ReplicaDiagnostics is the set one controller replica published.
type ReplicaDiagnostics struct {
	Replica     string
	Diagnostics []Diagnostic
}

func (r ReplicaDiagnostics) ResourceName() string {
	return r.Replica
}

func (r ReplicaDiagnostics) Equals(other ReplicaDiagnostics) bool {
	return r.Replica == other.Replica && slices.Equal(r.Diagnostics, other.Diagnostics)
}

// Published reads the diagnostics every replica published, indexed by source object.
type Published struct {
	Collection krt.Collection[ReplicaDiagnostics]
	BySource   krt.Index[Source, ReplicaDiagnostics]
}

func NewPublished(client apiclient.Client, krtopts krtutil.KrtOptions, namespace string) Published {
	configMaps := krt.NewFilteredInformer[*corev1.ConfigMap](client, kclient.Filter{
		ObjectFilter:  client.ObjectFilter(),
		Namespace:     namespace,
		LabelSelector: componentLabel + "=" + componentValue,
	}, krtopts.ToOptions("informer/ProxyFeedback")...)
	return NewPublishedFromConfigMaps(configMaps, krtopts.ToOptions("proxyfeedback/Published")...)
}

func NewPublishedFromConfigMaps(configMaps krt.Collection[*corev1.ConfigMap], opts ...krt.CollectionOption) Published {
	replicas := krt.NewCollection(configMaps, func(_ krt.HandlerContext, cm *corev1.ConfigMap) *ReplicaDiagnostics {
		var diagnostics []Diagnostic
		if err := json.Unmarshal([]byte(cm.Data[dataKey]), &diagnostics); err != nil {
			logger.Warn("ignoring unreadable proxy diagnostics", "configmap", cm.Name, "error", err)
			return nil
		}
		return &ReplicaDiagnostics{Replica: cm.Name, Diagnostics: diagnostics}
	}, opts...)
	return Published{
		Collection: replicas,
		BySource: krt.NewIndex(replicas, "proxyfeedback/BySource", func(r ReplicaDiagnostics) []Source {
			var sources []Source
			for _, d := range r.Diagnostics {
				if d.Source != (Source{}) && !slices.Contains(sources, d.Source) {
					sources = append(sources, d.Source)
				}
			}
			return sources
		}),
	}
}
