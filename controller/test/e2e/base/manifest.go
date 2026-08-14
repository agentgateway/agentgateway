//go:build e2e

package base

import (
	"fmt"
	"os"
	"path/filepath"
	goruntime "runtime"
	"slices"
	"strings"

	"istio.io/istio/pkg/test"
	istioassert "istio.io/istio/pkg/test/util/assert"
	"istio.io/istio/pkg/test/util/yml"
	appsv1 "k8s.io/api/apps/v1"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/serializer/yaml"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"

	apitests "github.com/agentgateway/agentgateway/controller/api/tests"
	"github.com/agentgateway/agentgateway/controller/test/e2e/testutils/assertions"
	"github.com/agentgateway/agentgateway/controller/test/testutils"
)

var decUnstructured = yaml.NewDecodingSerializer(unstructured.UnstructuredJSONScheme)

func (s *Test) applyYAML(contents ...string) {
	yaml := strings.Join(contents, "\n---\n")
	done := func() {}
	if len(contents) > 0 {
		done = traceStep(s, "applied YAML %q", yaml)
	}
	err := s.TestInstallation.ClusterContext.Client.ApplyYAMLContents("", contents...)
	if err != nil {
		istioassert.NoError(s, fmt.Errorf("failed to apply YAML %q: %w", yaml, err))
	}
	done()

	s.waitForResources(s.loadYAMLResources(contents...))
}

func (s *Test) applyManifests(manifests ...string) {
	manifests = interceptManifestFiles(s, s.TestInstallation.GeneratedFiles.TempDir, manifests...)
	done := func() {}
	if len(manifests) > 0 {
		done = traceStep(s, "applied manifests %v", manifestNames(manifests))
	}
	err := s.TestInstallation.ClusterContext.Client.ApplyYAMLFiles("", manifests...)
	istioassert.NoError(s, err)
	done()

	s.waitForResources(s.loadManifestResources(manifests...))
}

func (s *Test) waitForResources(resources []client.Object) {
	dynamicResources := s.loadDynamicResources(resources)
	allResources := slices.Concat(resources, dynamicResources)
	for _, resource := range allResources {
		var ns, name string
		if pod, ok := resource.(*corev1.Pod); ok {
			ns = pod.Namespace
			name = pod.Name
		} else if deployment, ok := resource.(*appsv1.Deployment); ok {
			if deployment.Spec.Replicas != nil && *deployment.Spec.Replicas == 0 {
				continue
			}
			ns = deployment.Namespace
			name = deployment.Name
		} else {
			continue
		}
		done := traceStep(s, "pods ready %s/%s", ns, name)
		assertions.EventuallyPodsRunning(s, ns, metav1.ListOptions{
			LabelSelector: fmt.Sprintf("%s=%s", WellKnownAppLabel, name),
		})
		done()
	}
}

// Apply applies YAML manifests, waits for any declared Pods/Deployments to become ready,
// and registers cleanup for the end of the test. This is the common path for per-test config.
func (s *Test) Apply(manifests ...string) {
	s.Helper()
	if s.ShouldSkip() {
		s.Skip("Skipping test due to gateway API version requirements")
	}

	s.applyManifests(manifests...)
	s.Cleanup(func() {
		if testutils.ShouldSkipCleanup(s) {
			return
		}
		s.deleteManifests(manifests...)
	})
}

// ApplyYAML is just like Apply, but instead of reading from a file it takes the yaml string directly.
func (s *Test) ApplyYAML(contents ...string) {
	s.Helper()
	if s.ShouldSkip() {
		s.Skip("Skipping test due to gateway API version requirements")
	}
	s.applyYAML(contents...)
	s.Cleanup(func() {
		if testutils.ShouldSkipCleanup(s) {
			return
		}
		s.deleteYAML(contents...)
	})
}

// ApplyPersistent is like Apply, but leaves resources behind when -agw.persist/PERSIST_INSTALL is set.
// Use it for expensive shared dependencies that should be reused across local test reruns.
func (s *Test) ApplyPersistent(manifests ...string) {
	s.Helper()
	if s.ShouldSkip() {
		s.Skip("Skipping test due to gateway API version requirements")
	}

	s.applyManifests(manifests...)
	s.Cleanup(func() {
		if testutils.ShouldSkipCleanup(s) || testutils.ShouldPersistInstall() {
			return
		}
		s.deleteManifests(manifests...)
	})
}

// Delete removes resources from YAML manifests immediately.
// Most tests should rely on Apply cleanup; call Delete only when the test behavior needs removal mid-test.
func (s *Test) Delete(manifests ...string) {
	s.Helper()
	s.deleteManifests(manifests...)
}

// Manifest resolves a file under the caller package's testdata directory.
// For example, Manifest("rbac", "policy.yaml") resolves testdata/rbac/policy.yaml.
func Manifest(pathParts ...string) string {
	_, file, _, ok := goruntime.Caller(1)
	if !ok {
		panic("failed to resolve caller for test manifest")
	}
	return filepath.Join(append([]string{filepath.Dir(file), "testdata"}, pathParts...)...)
}

func manifestNames(manifests []string) []string {
	names := make([]string, 0, len(manifests))
	for _, manifest := range manifests {
		names = append(names, filepath.Base(manifest))
	}
	return names
}

func stripNamespaceResources(t test.Failer, contents ...string) string {
	cfgs := []string{}
	for _, data := range contents {
		for _, yml := range yml.SplitString(data) {
			obj := &unstructured.Unstructured{}
			_, gvk, err := decUnstructured.Decode([]byte(yml), nil, obj)
			if runtime.IsMissingKind(err) {
				continue
			}
			istioassert.NoError(t, err)
			if gvk.Kind != "Namespace" {
				cfgs = append(cfgs, yml)
			}
		}
	}

	return strings.Join(cfgs, "\n---\n")
}

func (s *Test) deleteManifests(manifests ...string) {
	manifests = interceptManifestFiles(s, s.TestInstallation.GeneratedFiles.TempDir, manifests...)
	contents := []string{}
	for _, manifest := range manifests {
		data, err := os.ReadFile(manifest)
		istioassert.NoError(s, err)
		contents = append(contents, string(data))
	}
	nf := stripNamespaceResources(s, contents...)
	fp := filepath.Join(s.TestInstallation.GeneratedFiles.TempDir, "delete_manifests.yaml")
	istioassert.NoError(s, os.WriteFile(fp, []byte(nf), 0o644)) //nolint:gosec // G306: Golden test file can be readable

	err := s.TestInstallation.ClusterContext.Client.DeleteYAMLFiles("", fp)
	istioassert.NoError(s, err)
}

func (s *Test) deleteYAML(contents ...string) {
	nf := stripNamespaceResources(s, contents...)
	fp := filepath.Join(s.TestInstallation.GeneratedFiles.TempDir, "delete_manifests.yaml")
	istioassert.NoError(s, os.WriteFile(fp, []byte(nf), 0o644)) //nolint:gosec // G306: Golden test file can be readable

	err := s.TestInstallation.ClusterContext.Client.DeleteYAMLFiles("", fp)
	istioassert.NoError(s, err)
}

func (s *Test) setupHelpers() {
	configureScheme(s, s.TestInstallation.ClusterContext.ControllerClient.Scheme())
	s.validator = apitests.NewAgentgatewayValidatorSkipMissing(s)
}

func (s *Test) loadManifestResources(manifests ...string) []client.Object {
	var resources []client.Object
	for _, manifest := range manifests {
		objs, err := testutils.LoadFromFiles(manifest, s.TestInstallation.ClusterContext.ControllerClient.Scheme(), s.validator)
		istioassert.NoError(s, err)
		resources = append(resources, objs...)
	}
	return resources
}

func (s *Test) loadYAMLResources(contents ...string) []client.Object {
	var resources []client.Object
	for _, data := range contents {
		objs, err := testutils.LoadFromBytes([]byte(data), s.TestInstallation.ClusterContext.ControllerClient.Scheme(), s.validator)
		istioassert.NoError(s, err)
		resources = append(resources, objs...)
	}
	return resources
}

func (s *Test) loadDynamicResources(manifestResources []client.Object) []client.Object {
	var dynamicResources []client.Object
	for _, obj := range manifestResources {
		if gw, ok := obj.(*gwv1.Gateway); ok {
			proxyObjectMeta := metav1.ObjectMeta{
				Name:      gw.GetName(),
				Namespace: gw.GetNamespace(),
			}
			proxyResources := []client.Object{
				&appsv1.Deployment{ObjectMeta: proxyObjectMeta},
				&corev1.Service{ObjectMeta: proxyObjectMeta},
			}
			dynamicResources = append(dynamicResources, proxyResources...)
		}
	}
	return dynamicResources
}
