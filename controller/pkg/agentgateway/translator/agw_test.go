package translator

import (
	"testing"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/ptr"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

func TestCreateAgwRewriteFilterFullPath(t *testing.T) {
	cases := []struct {
		name string
		path string
		want string
	}{
		{"trailing slash preserved", "/app/", "/app/"},
		{"no trailing slash", "/app", "/app"},
		{"root", "/", "/"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			spec := CreateAgwRewriteFilter(&gwv1.HTTPURLRewriteFilter{
				Path: &gwv1.HTTPPathModifier{
					Type:            gwv1.FullPathHTTPPathModifier,
					ReplaceFullPath: ptr.Of(tc.path),
				},
			})
			rewrite := spec.GetUrlRewrite()
			assert.NotNil(t, rewrite)
			assert.Equal(t, tc.want, rewrite.GetFull())
		})
	}
}

func TestCreateAgwRewriteFilterPrefix(t *testing.T) {
	spec := CreateAgwRewriteFilter(&gwv1.HTTPURLRewriteFilter{
		Path: &gwv1.HTTPPathModifier{
			Type:               gwv1.PrefixMatchHTTPPathModifier,
			ReplacePrefixMatch: ptr.Of("/app/"),
		},
	})
	rewrite := spec.GetUrlRewrite()
	assert.NotNil(t, rewrite)
	assert.Equal(t, "/app", rewrite.GetPrefix())
}

func TestCreateAgwRedirectFilterFullPath(t *testing.T) {
	cases := []struct {
		name string
		path string
		want string
	}{
		{"trailing slash preserved", "/app/", "/app/"},
		{"no trailing slash", "/app", "/app"},
		{"root", "/", "/"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			redirect := CreateAgwRedirectFilter(&gwv1.HTTPRequestRedirectFilter{
				Path: &gwv1.HTTPPathModifier{
					Type:            gwv1.FullPathHTTPPathModifier,
					ReplaceFullPath: ptr.Of(tc.path),
				},
			})
			assert.Equal(t, tc.want, redirect.GetFull())
		})
	}
}
