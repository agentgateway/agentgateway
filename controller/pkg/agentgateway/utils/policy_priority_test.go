package utils_test

import (
	"testing"

	"github.com/stretchr/testify/require"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"

	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/utils"
)

func TestHigherPriorityPolicy(t *testing.T) {
	tests := []struct {
		name string
		a    metav1.Object
		b    metav1.Object
		want bool
	}{
		{
			name: "older creation timestamp wins",
			a: &metav1.PartialObjectMetadata{
				ObjectMeta: metav1.ObjectMeta{
					Namespace:         "default",
					Name:              "a",
					CreationTimestamp: metav1.Unix(10, 0),
				},
			},
			b: &metav1.PartialObjectMetadata{
				ObjectMeta: metav1.ObjectMeta{
					Namespace:         "default",
					Name:              "b",
					CreationTimestamp: metav1.Unix(20, 0),
				},
			},
			want: true,
		},
		{
			name: "namespace breaks ties",
			a: &metav1.PartialObjectMetadata{
				ObjectMeta: metav1.ObjectMeta{
					Namespace:         "alpha",
					Name:              "same",
					CreationTimestamp: metav1.Unix(10, 0),
				},
			},
			b: &metav1.PartialObjectMetadata{
				ObjectMeta: metav1.ObjectMeta{
					Namespace:         "beta",
					Name:              "same",
					CreationTimestamp: metav1.Unix(10, 0),
				},
			},
			want: true,
		},
		{
			name: "name breaks remaining ties",
			a: &metav1.PartialObjectMetadata{
				ObjectMeta: metav1.ObjectMeta{
					Namespace:         "default",
					Name:              "a",
					CreationTimestamp: metav1.Unix(10, 0),
				},
			},
			b: &metav1.PartialObjectMetadata{
				ObjectMeta: metav1.ObjectMeta{
					Namespace:         "default",
					Name:              "b",
					CreationTimestamp: metav1.Unix(10, 0),
				},
			},
			want: true,
		},
		{
			name: "higher priority is directional",
			a: &metav1.PartialObjectMetadata{
				ObjectMeta: metav1.ObjectMeta{
					Namespace:         "default",
					Name:              "z",
					CreationTimestamp: metav1.Unix(20, 0),
				},
			},
			b: &metav1.PartialObjectMetadata{
				ObjectMeta: metav1.ObjectMeta{
					Namespace:         "default",
					Name:              "a",
					CreationTimestamp: metav1.Unix(10, 0),
				},
			},
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			require.Equal(t, tt.want, utils.HigherPriorityPolicy(tt.a, tt.b))
		})
	}
}
