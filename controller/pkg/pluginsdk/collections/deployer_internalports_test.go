package collections

import (
	"testing"
	"time"

	"istio.io/istio/pkg/slices"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	"github.com/agentgateway/agentgateway/controller/api/annotations"
)

func gw(annotation string, ports ...gwv1.PortNumber) *gwv1.Gateway {
	g := &gwv1.Gateway{}
	if annotation != "" {
		g.Annotations = map[string]string{annotations.InternalPorts: annotation}
	}
	for i, p := range ports {
		g.Spec.Listeners = append(g.Spec.Listeners, gwv1.Listener{
			Name:     gwv1.SectionName(string(rune('a' + i))),
			Protocol: gwv1.HTTPProtocolType,
			Port:     p,
		})
	}
	return g
}

func ls(name, annotation string, created time.Time, ports ...gwv1.PortNumber) *gwv1.ListenerSet {
	l := &gwv1.ListenerSet{ObjectMeta: metav1.ObjectMeta{
		Name:              name,
		CreationTimestamp: metav1.NewTime(created),
	}}
	if annotation != "" {
		l.Annotations = map[string]string{annotations.InternalPorts: annotation}
	}
	for i, p := range ports {
		l.Spec.Listeners = append(l.Spec.Listeners, gwv1.ListenerEntry{
			Name:     gwv1.SectionName(string(rune('a' + i))),
			Protocol: gwv1.HTTPProtocolType,
			Port:     p,
		})
	}
	return l
}

func TestComputeInternalPorts(t *testing.T) {
	now := time.Now()
	tests := []struct {
		name  string
		gw    *gwv1.Gateway
		lsets []*gwv1.ListenerSet
		want  []int32
	}{
		{
			name: "gateway annotation marks its port internal",
			gw:   gw("8080", 80, 8080),
			want: []int32{8080},
		},
		{
			name:  "listenerset can add an internal port",
			gw:    gw("", 80),
			lsets: []*gwv1.ListenerSet{ls("internal", "9090", now, 9090)},
			want:  []int32{9090},
		},
		{
			name:  "gateway internal mode wins over standard listenerset",
			gw:    gw("8080", 8080),
			lsets: []*gwv1.ListenerSet{ls("standard", "", now, 8080)},
			want:  []int32{8080},
		},
		{
			name:  "gateway standard mode wins over internal listenerset",
			gw:    gw("", 8080),
			lsets: []*gwv1.ListenerSet{ls("internal", "8080", now, 8080)},
			want:  nil,
		},
		{
			name: "older listenerset mode wins",
			gw:   gw("", 80),
			lsets: []*gwv1.ListenerSet{
				ls("newer-standard", "", now.Add(time.Minute), 9090),
				ls("older-internal", "9090", now, 9090),
			},
			want: []int32{9090},
		},
		{
			name: "invalid annotation is ignored",
			gw:   gw("9999", 80),
			want: nil,
		},
		{
			name: "no annotation",
			gw:   gw("", 80, 8080),
			want: nil,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := slices.Sort(ComputeInternalPorts(tt.gw, tt.lsets).List())
			want := slices.Sort(tt.want)
			if !slices.Equal(got, want) {
				t.Fatalf("ComputeInternalPorts = %v, want %v", got, want)
			}
		})
	}
}
