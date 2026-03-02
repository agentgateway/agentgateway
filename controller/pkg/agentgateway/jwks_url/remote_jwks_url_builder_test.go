package jwks_url_test

import (
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"path/filepath"
	"testing"

	"github.com/stretchr/testify/assert"
	"istio.io/istio/pkg/kube/krt"
	"istio.io/istio/pkg/ptr"
	"istio.io/istio/pkg/test/util/file"
	corev1 "k8s.io/api/core/v1"
	"k8s.io/apimachinery/pkg/types"

	apitests "github.com/agentgateway/agentgateway/controller/api/tests"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/jwks_url"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/plugins"
	"github.com/agentgateway/agentgateway/controller/pkg/agentgateway/testutils"
	"github.com/agentgateway/agentgateway/controller/pkg/utils/fsutils"
)

func TestRemoteJwksUrlBuilder(t *testing.T) {
	var mockCtx plugins.PolicyCtx

	var tests = []struct {
		name          string
		ctx           plugins.PolicyCtx
		expectedError error
		expectedUrl   string
		expectedTls   *tls.Config
	}{
		{
			name:        "jwksPath prefixed with a '/'",
			ctx:         setup(t, []string{getTestFile("jwkspath-starts-with-slash.yaml")}),
			expectedUrl: "http://dummy-idp.default.svc.cluster.local:8443/org-one/keys",
		},
		{
			name:        "service with no tls",
			ctx:         setup(t, []string{getTestFile("svc-clear-text.yaml")}),
			expectedUrl: "http://dummy-idp.default.svc.cluster.local:8443/org-one/keys",
		},
		{
			name:        "default tls",
			ctx:         setup(t, []string{getTestFile("svc-with-default-tls.yaml")}),
			expectedUrl: "https://dummy-idp.default.svc.cluster.local:8443/org-one/keys",
			expectedTls: &tls.Config{}, //nolint:gosec
		},
		{
			name:        "tls with InsecureSkipVerify",
			ctx:         setup(t, []string{getTestFile("svc-insecure-skip-verify.yaml")}),
			expectedUrl: "https://dummy-idp.default.svc.cluster.local:8443/org-one/keys",
			expectedTls: &tls.Config{
				InsecureSkipVerify: true, //nolint:gosec
			},
		},
		{
			name: "tls with all configurable fields",
			ctx: func() plugins.PolicyCtx {
				mockCtx = setup(t, []string{getTestFile("svc-with-all-configurable-fields.yaml")})
				return mockCtx
			}(),
			expectedUrl: "https://dummy-idp.default.svc.cluster.local:8443/org-one/keys",
			expectedTls: &tls.Config{ //nolint:gosec
				ServerName: "test.testns",
				NextProtos: []string{"test1", "test2"},
				RootCAs:    caFromConfigMap(t, mockCtx),
			},
		},
		{
			name:          "non-static backend",
			ctx:           setup(t, []string{getTestFile("gw-with-non-static-backend.yaml")}),
			expectedError: fmt.Errorf("only static backends are supported; backend: default/fail, policy: default/gw-policy"),
		},
		{
			name:          "missing backend",
			ctx:           setup(t, []string{getTestFile("gw-with-missing-backend.yaml")}),
			expectedError: fmt.Errorf("backend default/fail not found, policy default/gw-policy"),
		},
		{
			name:        "backend with no tls",
			ctx:         setup(t, []string{getTestFile("gw-with-backend-clear-text.yaml")}),
			expectedUrl: "http://dummy-idp.default:8443/org-one/keys",
		},
		{
			name:        "backend with tls with InsecureSkipVerify",
			ctx:         setup(t, []string{getTestFile("gw-with-backend-insecure-skip-verify.yaml")}),
			expectedUrl: "https://dummy-idp.default:8443/org-one/keys",
			expectedTls: &tls.Config{
				InsecureSkipVerify: true, //nolint:gosec
			},
		},
		{
			name:        "backend with default tls",
			ctx:         setup(t, []string{getTestFile("gw-with-backend-default-tls.yaml")}),
			expectedUrl: "https://dummy-idp.default:8443/org-one/keys",
			expectedTls: &tls.Config{}, //nolint:gosec
		},
		{
			name: "backend with tls with all configurable fields",
			ctx: func() plugins.PolicyCtx {
				mockCtx = setup(t, []string{getTestFile("gw-with-backend-all-configurable-fields.yaml")})
				return mockCtx
			}(),
			expectedUrl: "https://dummy-idp.default:8443/org-one/keys",
			expectedTls: &tls.Config{ //nolint:gosec
				ServerName: "test.testns",
				NextProtos: []string{"test1", "test2"},
				RootCAs:    caFromConfigMap(t, mockCtx),
			},
		},
		{
			name:        "backend with a ref to a policy with default tls",
			ctx:         setup(t, []string{getTestFile("gw-with-backend-with-policy-ref-with-default-tls.yaml")}),
			expectedUrl: "https://dummy-idp.default:8443/org-one/keys",
			expectedTls: &tls.Config{}, //nolint:gosec
		},
		{
			name:        "backend with a ref to a policy with tls with InsecureSkipVerify",
			ctx:         setup(t, []string{getTestFile("gw-with-backend-with-policy-ref-with-insecure-skip-verify.yaml")}),
			expectedUrl: "https://dummy-idp.default:8443/org-one/keys",
			expectedTls: &tls.Config{
				InsecureSkipVerify: true, //nolint:gosec
			},
		},
		{
			name: "backend with a ref to a policy with tls with all configurable fields",
			ctx: func() plugins.PolicyCtx {
				mockCtx = setup(t, []string{getTestFile("gw-with-backend-with-policy-ref-with-all-configurable-fields.yaml")})
				return mockCtx
			}(),
			expectedUrl: "https://dummy-idp.default:8443/org-one/keys",
			expectedTls: &tls.Config{ //nolint:gosec
				ServerName: "test.testns",
				NextProtos: []string{"test1", "test2"},
				RootCAs:    caFromConfigMap(t, mockCtx),
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			f := jwks_url.NewJwksUrlFactory(tt.ctx.Collections.ConfigMaps, tt.ctx.Collections.Backends, tt.ctx.Collections.AgentgatewayPolicies)
			pol := ptr.Flatten(krt.FetchOne(tt.ctx.Krt, tt.ctx.Collections.AgentgatewayPolicies, krt.FilterObjectName(types.NamespacedName{Name: "gw-policy", Namespace: "default"})))

			assert.NotNil(t, pol)
			assert.NotNil(t, pol.Spec.Traffic)
			assert.NotNil(t, pol.Spec.Traffic.JWTAuthentication)
			assert.Len(t, pol.Spec.Traffic.JWTAuthentication.Providers, 1)
			assert.NotNil(t, pol.Spec.Traffic.JWTAuthentication.Providers[0].JWKS.Remote)
			url, tlsConfig, err := f.BuildJwksUrlAndTlsConfig(tt.ctx.Krt, "gw-policy", "default", pol.Spec.Traffic.JWTAuthentication.Providers[0].JWKS.Remote)
			assert.Equal(t, tt.expectedUrl, url)
			if tt.expectedTls != nil {
				assert.Equal(t, tt.expectedTls.ServerName, tlsConfig.ServerName)
				assert.Equal(t, tt.expectedTls.NextProtos, tlsConfig.NextProtos)
				assert.Equal(t, tt.expectedTls.InsecureSkipVerify, tlsConfig.InsecureSkipVerify)
				assert.True(t, tt.expectedTls.RootCAs.Equal(tlsConfig.RootCAs)) // must use CertPool.Equal() for equality check
			} else {
				assert.Nil(t, tlsConfig)
			}
			assert.Equal(t, tt.expectedError, err)
		})
	}
}

func setup(t *testing.T, paths []string) plugins.PolicyCtx {
	val := apitests.NewKgatewayValidator(t)
	val.SkipMissing = true
	mockObjs := []any{}
	for _, f := range paths {
		data := file.AsStringOrFail(t, f)
		assert.NoError(t, val.ValidateCustomResourceYAML(data, nil))
		mockObjs = append(mockObjs, data)
	}
	return testutils.BuildMockPolicyContext(t, mockObjs)
}

func getTestFile(filename string) string {
	return filepath.Join(fsutils.MustGetThisDir(), "testdata", filename)
}

func caFromConfigMap(t *testing.T, ctx plugins.PolicyCtx) *x509.CertPool {
	certPool := x509.NewCertPool()
	cfgmap := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.ConfigMaps, krt.FilterObjectName(types.NamespacedName{Namespace: "default", Name: "ca"})))
	assert.NotNil(t, cfgmap)

	assert.True(t, jwks_url.AppendPoolWithCertsFromConfigMap(certPool, cfgmap))
	return certPool
}

func TestAppendPoolWithCertsFromConfigMapBinaryData(t *testing.T) {
	ctx := setup(t, []string{getTestFile("svc-with-all-configurable-fields.yaml")})
	source := ptr.Flatten(krt.FetchOne(ctx.Krt, ctx.Collections.ConfigMaps, krt.FilterObjectName(types.NamespacedName{Namespace: "default", Name: "ca"})))
	assert.NotNil(t, source)

	certPool := x509.NewCertPool()
	cfgmap := &corev1.ConfigMap{
		BinaryData: map[string][]byte{
			"ca.crt": []byte(source.Data["ca.crt"]),
		},
	}

	assert.True(t, jwks_url.AppendPoolWithCertsFromConfigMap(certPool, cfgmap))
}
