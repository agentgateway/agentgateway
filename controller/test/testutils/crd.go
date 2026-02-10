package testutils

import (
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"

	"istio.io/istio/pkg/config/schema/gvr"
	"k8s.io/apiextensions-apiserver/pkg/apis/apiextensions"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	apiserverschema "k8s.io/apiextensions-apiserver/pkg/apiserver/schema"
	structuraldefaulting "k8s.io/apiextensions-apiserver/pkg/apiserver/schema/defaulting"
	structuralpruning "k8s.io/apiextensions-apiserver/pkg/apiserver/schema/pruning"
	"k8s.io/apimachinery/pkg/apis/meta/v1/unstructured"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	utilyaml "k8s.io/apimachinery/pkg/util/yaml"
	"sigs.k8s.io/yaml"

	"github.com/kgateway-dev/kgateway/v2/pkg/kgateway/wellknown"
)

var AllCRDs = []schema.GroupVersionResource{
	// Gateway API
	gvr.KubernetesGateway,
	gvr.GatewayClass,
	gvr.HTTPRoute,
	gvr.GRPCRoute,
	gvr.TCPRoute,
	gvr.TLSRoute,
	gvr.ReferenceGrant,
	gvr.BackendTLSPolicy,
	gvr.XListenerSet,
	wellknown.InferencePoolGVR,
	wellknown.BackendTLSPolicyGVR,
	// K8s API
	gvr.Service,
	gvr.Pod,
	// Istio API
	gvr.ServiceEntry,
	gvr.WorkloadEntry,
	gvr.AuthorizationPolicy,
	// agentgateway api
	wellknown.AgentgatewayBackendGVR,
	wellknown.AgentgatewayParametersGVR,
	wellknown.AgentgatewayPolicyGVR,
}

const (
	AgwCRDPath = "install/helm/agentgateway-crds/templates"
)

// GetStructuralSchemas returns a map of GroupVersionKind to Structural schemas for all CRDs in the given directories.
// Deprecated: Use GetStructuralSchemasForBothCharts instead to load both kgateway and agentgateway CRDs.
func GetStructuralSchemas(
	crdDir string,
) (map[schema.GroupVersionKind]*apiserverschema.Structural, error) {
	crds, err := getCRDs(crdDir)
	if err != nil {
		return nil, err
	}
	return buildStructuralSchemaMap(crds)
}

// GetStructuralSchemasForBothCharts returns a map of GroupVersionKind to Structural schemas for all CRDs
// from both kgateway-crds and agentgateway-crds charts.
func GetStructuralSchemasForBothCharts() (map[schema.GroupVersionKind]*apiserverschema.Structural, error) {
	gitRoot := GitRootDirectory()
	agwCRDDir := filepath.Join(gitRoot, AgwCRDPath)

	agwCRDs, err := getCRDs(agwCRDDir)
	if err != nil {
		return nil, fmt.Errorf("error loading agentgateway CRDs: %w", err)
	}

	return buildStructuralSchemaMap(agwCRDs)
}

// buildStructuralSchemaMap converts a list of CRDs to a map of GVK to structural schemas
func buildStructuralSchemaMap(crds []*apiextensions.CustomResourceDefinition) (map[schema.GroupVersionKind]*apiserverschema.Structural, error) {
	gvkToStructuralSchema := map[schema.GroupVersionKind]*apiserverschema.Structural{}

	for _, crd := range crds {
		versions := crd.Spec.Versions
		if len(versions) == 0 {
			return nil, fmt.Errorf("spec.versions not set for CRD %s.%s", crd.Kind, crd.Spec.Group)
		}

		for _, ver := range versions {
			crd.Status.StoredVersions = append(crd.Status.StoredVersions, ver.Name)

			gvk := schema.GroupVersionKind{
				Group:   crd.Spec.Group,
				Version: ver.Name,
				Kind:    crd.Spec.Names.Kind,
			}
			validationSchema, err := apiextensions.GetSchemaForVersion(crd, ver.Name)
			if err != nil {
				return nil, err
			}
			structuralSchema, err := apiserverschema.NewStructural(validationSchema.OpenAPIV3Schema)
			if err != nil {
				return nil, err
			}
			gvkToStructuralSchema[gvk] = structuralSchema
		}
	}
	return gvkToStructuralSchema, nil
}

// ApplyDefaults applies default values to the given object using the provided structural schema.
// The API defaults are a part of the structural schema.
func ApplyDefaults(
	objYAML []byte,
	structuralSchema *apiserverschema.Structural,
) (*unstructured.Unstructured, []byte, error) {
	// Convert YAML to map without losing any fields (using the Go type with omitempty will drop zero-value fields)
	raw := make(map[string]any)
	err := yaml.Unmarshal(objYAML, &raw)
	if err != nil {
		return nil, nil, err
	}
	u := &unstructured.Unstructured{
		Object: raw,
	}

	// Pruning:
	// 1. Detect unknown fields
	// 2. Drop null values for non-nullable and non-defaultable fields values
	pruneOpts := apiserverschema.UnknownFieldPathOptions{
		TrackUnknownFieldPaths: true,
	}
	unknownFields := structuralpruning.PruneWithOptions(u.Object, structuralSchema, true, pruneOpts)
	if len(unknownFields) > 0 {
		return nil, nil, fmt.Errorf("got unknown fields: %v", unknownFields)
	}
	structuraldefaulting.PruneNonNullableNullsWithoutDefaults(u.Object, structuralSchema)

	// Apply defaults
	structuraldefaulting.Default(u.UnstructuredContent(), structuralSchema)
	objYAML, err = yaml.Marshal(u.Object)
	if err != nil {
		return nil, nil, err
	}
	return u, objYAML, nil
}

func parseCRDs(path string) ([]*apiextensions.CustomResourceDefinition, error) {
	f, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer f.Close()

	decoder := utilyaml.NewYAMLOrJSONDecoder(f, 4096)

	// There could be multiple CRDs per file (e.g., for testing)
	var crds []*apiextensions.CustomResourceDefinition
	for {
		raw := new(unstructured.Unstructured)
		err := decoder.Decode(raw)
		if errors.Is(err, io.EOF) {
			break
		} else if err != nil {
			return nil, err
		}

		// Assume all our CRDs are apiextensions.k8s.io/v1
		crd := new(apiextensions.CustomResourceDefinition)
		crdv1 := new(apiextensionsv1.CustomResourceDefinition)
		if err := runtime.DefaultUnstructuredConverter.
			FromUnstructured(raw.UnstructuredContent(), crdv1); err != nil {
			return nil, err
		}
		if err := apiextensionsv1.Convert_v1_CustomResourceDefinition_To_apiextensions_CustomResourceDefinition(crdv1, crd, nil); err != nil {
			return nil, err
		}

		crds = append(crds, crd)
	}

	return crds, nil
}

func getCRDs(crdDir string) ([]*apiextensions.CustomResourceDefinition, error) {
	var crds []*apiextensions.CustomResourceDefinition
	files, err := os.ReadDir(crdDir)
	if err != nil {
		return nil, err
	}
	for _, f := range files {
		if f.IsDir() || !strings.HasSuffix(f.Name(), ".yaml") {
			continue
		}

		filePath := filepath.Join(crdDir, f.Name())
		specs, err := parseCRDs(filePath)
		if err != nil {
			if errors.As(err, &utilyaml.JSONSyntaxError{}) {
				// If there is a parsing error, ignore the CRD as it is templated
				continue
			}
			return nil, err
		}
		crds = append(crds, specs...)
	}

	return crds, nil
}
