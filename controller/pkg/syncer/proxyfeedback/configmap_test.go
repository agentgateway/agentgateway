package proxyfeedback

import (
	"context"
	"testing"
	"time"

	"istio.io/istio/pkg/kube"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/test"
	"istio.io/istio/pkg/test/util/assert"
	corev1 "k8s.io/api/core/v1"
	apierrors "k8s.io/apimachinery/pkg/api/errors"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
)

const testNamespace = "agentgateway-system"

type alwaysSynced struct{}

func (alwaysSynced) WaitUntilSynced(<-chan struct{}) bool { return true }
func (alwaysSynced) HasSynced() bool                      { return true }

func testWriter(tracker *Tracker, client kube.Client) *Writer {
	w := NewWriter(tracker, client, testNamespace)
	w.name = configMapPrefix + "replica-a"
	w.podName = "replica-a"
	w.startupDelay = 10 * time.Millisecond
	w.publishDelay = 10 * time.Millisecond
	return w
}

func reportBadKey(tracker *Tracker) *Connection {
	conn := tracker.NewConnection(testGateway, policyLookup())
	conn.Sent(testType, "n1", resources("policy/auth"), nil)
	conn.Replied(testType, "n1", []Report{badKey()})
	return conn
}

func TestWriterPublishesAndLeaderReads(t *testing.T) {
	client := kube.NewFakeClient()
	tracker := NewTracker()
	w := testWriter(tracker, client)
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		_ = w.Start(ctx)
		close(done)
	}()

	conn := reportBadKey(tracker)
	configMaps := client.Kube().CoreV1().ConfigMaps(testNamespace)
	var published *corev1.ConfigMap
	assert.EventuallyEqual(t, func() bool {
		cm, err := configMaps.Get(ctx, w.name, metav1.GetOptions{})
		if err != nil {
			return false
		}
		published = cm
		return cm.Data[dataKey] != "[]"
	}, true)
	assert.Equal(t, published.Labels[componentLabel], componentValue)

	stop := test.NewStop(t)
	read := NewPublishedFromConfigMaps(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{published}, krt.WithStop(stop)),
		krt.WithStop(stop),
	)
	read.Collection.WaitUntilSynced(stop)
	got := read.BySource.Lookup(testSource)
	assert.Equal(t, len(got), 1)
	assert.Equal(t, got[0].Diagnostics, tracker.Snapshot())

	// Clearing the report publishes the empty set.
	conn.Close()
	assert.EventuallyEqual(t, func() string {
		cm, err := configMaps.Get(ctx, w.name, metav1.GetOptions{})
		if err != nil {
			return err.Error()
		}
		return cm.Data[dataKey]
	}, "[]")

	// Without a controller Pod to own it, shutting down removes the ConfigMap.
	cancel()
	<-done
	_, err := configMaps.Get(context.Background(), w.name, metav1.GetOptions{})
	assert.Equal(t, apierrors.IsNotFound(err), true)
}

func TestWriterOwnedByPod(t *testing.T) {
	pod := &corev1.Pod{Namespace: testNamespace, Name: "replica-a", UID: "pod-uid"}
	client := kube.NewFakeClient(pod)
	w := testWriter(NewTracker(), client)
	owner := w.podOwnerReference(context.Background())
	assert.Equal(t, owner != nil, true)

	assert.NoError(t, w.publish(context.Background(), "[]", owner))
	cm, err := client.Kube().CoreV1().ConfigMaps(testNamespace).Get(context.Background(), w.name, metav1.GetOptions{})
	assert.NoError(t, err)
	assert.Equal(t, cm.OwnerReferences, []metav1.OwnerReference{{APIVersion: "v1", Kind: "Pod", Name: "replica-a", UID: "pod-uid"}})
}

func TestPublishedIgnoresUnreadableConfigMap(t *testing.T) {
	stop := test.NewStop(t)
	cm := &corev1.ConfigMap{
		Namespace: testNamespace,
		Name:      configMapPrefix + "replica-b",
		Data:      map[string]string{dataKey: "not json"},
	}
	read := NewPublishedFromConfigMaps(
		krt.NewStaticCollection[*corev1.ConfigMap](alwaysSynced{}, []*corev1.ConfigMap{cm}, krt.WithStop(stop)),
		krt.WithStop(stop),
	)
	read.Collection.WaitUntilSynced(stop)
	assert.Equal(t, len(read.Collection.List()), 0)
}
