package testutils

import (
	"bytes"
	"errors"
	"fmt"
	"io/fs"
	"log/slog"
	"os"
	"path/filepath"
	"strings"

	"github.com/ghodss/yaml"
	apiserverschema "k8s.io/apiextensions-apiserver/pkg/apiserver/schema"
	apiextensionsvalidation "k8s.io/apiextensions-apiserver/pkg/apiserver/validation"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/apimachinery/pkg/runtime/schema"
	"sigs.k8s.io/controller-runtime/pkg/client"
	gwv1 "sigs.k8s.io/gateway-api/apis/v1"
)

var ErrNoFilesFound = errors.New("no k8s files found")

// FileContentTransformer is a function that transforms a file's contents
type FileContentTransformer func(content string) string

func LoadFromFiles(
	filename string,
	scheme *runtime.Scheme,
	gvkToStructuralSchema map[schema.GroupVersionKind]*apiserverschema.Structural,
) ([]client.Object, error) {
	return LoadFromFileWithTransform(filename, scheme, gvkToStructuralSchema, nil)
}

func LoadFromFileWithTransform(
	filename string,
	scheme *runtime.Scheme,
	gvkToStructuralSchema map[schema.GroupVersionKind]*apiserverschema.Structural,
	transformer FileContentTransformer,
) ([]client.Object, error) {
	fileOrDir, err := os.Stat(filename)
	if err != nil {
		return nil, err
	}

	var yamlFiles []string
	if fileOrDir.IsDir() {
		slog.Debug("looking for YAML files", "path", fileOrDir.Name())
		err := filepath.WalkDir(filename, func(path string, d fs.DirEntry, _ error) error {
			if strings.HasSuffix(path, ".yml") || strings.HasSuffix(path, ".yaml") {
				yamlFiles = append(yamlFiles, path)
			}
			return nil
		})
		if err != nil {
			return nil, err
		}
	} else {
		yamlFiles = append(yamlFiles, filename)
	}

	if len(yamlFiles) == 0 {
		return nil, ErrNoFilesFound
	}

	slog.Debug("user configuration YAML files found", "files", yamlFiles)

	var resources []client.Object
	for _, file := range yamlFiles {
		objs, err := parseFile(file, scheme, gvkToStructuralSchema, transformer)
		if err != nil {
			return nil, err
		}
		for _, obj := range objs {
			clientObj, ok := obj.(client.Object)
			if !ok {
				return nil, fmt.Errorf("cannot convert runtime.Object to client.Object: %+v", obj)
			}

			_, isGwc := clientObj.(*gwv1.GatewayClass)
			if !isGwc && clientObj.GetNamespace() == "" {
				// fill in default namespace
				clientObj.SetNamespace(GetDefaultNamespace())
			}
			resources = append(resources, clientObj)
		}
	}

	return resources, nil
}

func parseFile(
	filename string,
	scheme *runtime.Scheme,
	gvkToStructuralSchema map[schema.GroupVersionKind]*apiserverschema.Structural,
	transformer FileContentTransformer,
) ([]runtime.Object, error) {
	file, err := os.ReadFile(filename)
	if err != nil {
		return nil, err
	}
	if transformer != nil {
		file = []byte(transformer(string(file)))
	}

	type metaOnly struct {
		metav1.TypeMeta   `json:",inline"`
		metav1.ObjectMeta `json:"metadata,omitempty" protobuf:"bytes,1,opt,name=metadata"`
	}

	// Split into individual YAML documents
	resourceYamlStrings := bytes.Split(file, []byte("\n---\n"))

	// Create resources from YAML documents
	var genericResources []runtime.Object
	for _, objYaml := range resourceYamlStrings {
		// Skip empty documents
		if len(bytes.TrimSpace(objYaml)) == 0 {
			continue
		}

		var meta metaOnly
		if err := yaml.Unmarshal(objYaml, &meta); err != nil {
			slog.Warn("failed to parse resource metadata, skipping YAML document",
				"filename", filename,
				"data", truncateString(string(objYaml), 100),
			)
			continue
		}

		gvk := schema.FromAPIVersionAndKind(meta.APIVersion, meta.Kind)
		obj, err := scheme.New(gvk)
		if err != nil {
			slog.Warn("unknown resource kind",
				"filename", filename,
				"gvk", gvk.String(),
				"data", truncateString(string(objYaml), 100),
			)
			continue
		}

		if err := yaml.Unmarshal(objYaml, obj); err != nil {
			slog.Warn("failed to parse resource YAML",
				"error", err,
				"filename", filename,
				"gvk", gvk.String(),
				"resource_id", obj.(client.Object).GetName()+"."+obj.(client.Object).GetNamespace(),
				"data", truncateString(string(objYaml), 100),
			)
			continue
		}

		if structuralSchema, ok := gvkToStructuralSchema[gvk]; ok {
			unstructuredObj, objYamlWithDefaults, err := ApplyDefaults(objYaml, structuralSchema)
			if err != nil {
				return nil, fmt.Errorf("failed to apply defaults for %s: %w", gvk, err)
			}
			validator := apiextensionsvalidation.NewSchemaValidatorFromOpenAPI(structuralSchema.ToKubeOpenAPI())
			validationErrs := apiextensionsvalidation.ValidateCustomResource(nil, unstructuredObj.UnstructuredContent(), validator)
			if len(validationErrs) > 0 {
				agg := validationErrs.ToAggregate()
				return nil, fmt.Errorf("failed to validate %s: %w", gvk, agg)
			}
			if err := yaml.Unmarshal(objYamlWithDefaults, obj); err != nil {
				return nil, fmt.Errorf("failed to unmarshal object with defaults for %s: %w", gvk, err)
			}
		}

		genericResources = append(genericResources, obj)
	}

	return genericResources, err
}

func truncateString(str string, num int) string {
	result := str
	if len(str) > num {
		result = str[0:num] + "..."
	}
	return result
}

func ToRuntimeObjects(objs ...client.Object) []runtime.Object {
	res := make([]runtime.Object, len(objs))
	for i, obj := range objs {
		res[i] = obj
	}
	return res
}
