package main

import (
	"testing"

	"github.com/stretchr/testify/require"
	apiextensionsv1 "k8s.io/apiextensions-apiserver/pkg/apis/apiextensions/v1"
	"sigs.k8s.io/controller-tools/pkg/crd"
	"sigs.k8s.io/controller-tools/pkg/loader"
	"sigs.k8s.io/controller-tools/pkg/markers"
)

func newTestParser(t *testing.T, rootPath string) ([]*loader.Package, *crd.Parser) {
	t.Helper()

	roots, err := loader.LoadRoots(rootPath)
	require.NoError(t, err)
	require.Len(t, roots, 1)

	generator := &crd.Generator{}
	parser := &crd.Parser{
		Collector: &markers.Collector{Registry: &markers.Registry{}},
		Checker: &loader.TypeChecker{
			NodeFilters: []loader.NodeFilter{generator.CheckFilter()},
		},
	}
	require.NoError(t, generator.RegisterMarkers(parser.Collector.Registry))
	require.NoError(t, registerCustomMarkers(parser.Collector.Registry))
	crd.AddKnownTypes(parser)
	parser.NeedPackage(roots[0])

	return roots, parser
}

func TestAllJSONFieldNamesForTypeIncludesImportedInlineFields(t *testing.T) {
	const rootPath = "github.com/agentgateway/agentgateway/controller/hack/crdgen/testdata/root"

	roots, parser := newTestParser(t, rootPath)

	typ := crd.TypeIdent{
		Package: roots[0],
		Name:    "Wrapper",
	}
	info := parser.LookupType(typ.Package, typ.Name)
	require.NotNil(t, info)

	fields, err := allJSONFieldNamesForType(parser, typ, info, map[crd.TypeIdent][]string{}, map[crd.TypeIdent]bool{})
	require.NoError(t, err)
	require.Equal(t, []string{"bar", "baz", "foo"}, fields)
}

func TestOverrideXValidationReplacesExactlyOneMatch(t *testing.T) {
	schema := &apiextensionsv1.JSONSchemaProps{
		XValidations: []apiextensionsv1.ValidationRule{
			{
				Rule:    "old-rule",
				Message: "phase PreRouting only supports extAuth",
			},
			{
				Rule:    "keep-rule",
				Message: "some other validation",
			},
		},
	}

	err := applyOverrideXValidation(schema, OverrideXValidation{
		MessageContains: "phase PreRouting only",
		Rule:            "new-rule",
	})
	require.NoError(t, err)
	require.Equal(t, apiextensionsv1.ValidationRules{
		{
			Rule:    "new-rule",
			Message: "phase PreRouting only supports extAuth",
		},
		{
			Rule:    "keep-rule",
			Message: "some other validation",
		},
	}, schema.XValidations)
}

func TestOverrideXValidationRemovesMatchWhenRuleEmpty(t *testing.T) {
	schema := &apiextensionsv1.JSONSchemaProps{
		XValidations: []apiextensionsv1.ValidationRule{
			{
				Rule:    "old-rule",
				Message: "phase PreRouting only supports extAuth",
			},
			{
				Rule:    "keep-rule",
				Message: "some other validation",
			},
		},
	}

	err := applyOverrideXValidation(schema, OverrideXValidation{
		MessageContains: "phase PreRouting only",
	})
	require.NoError(t, err)
	require.Equal(t, apiextensionsv1.ValidationRules{
		{
			Rule:    "keep-rule",
			Message: "some other validation",
		},
	}, schema.XValidations)
}

func TestOverrideXValidationErrorsWithoutExactSingleMatch(t *testing.T) {
	t.Run("no matches", func(t *testing.T) {
		schema := &apiextensionsv1.JSONSchemaProps{
			XValidations: []apiextensionsv1.ValidationRule{
				{Rule: "keep-rule", Message: "some other validation"},
			},
		}

		err := applyOverrideXValidation(schema, OverrideXValidation{
			MessageContains: "phase PreRouting only",
			Rule:            "new-rule",
		})
		require.EqualError(t, err, "OverrideXValidation matched 0 rules for messageContains \"phase PreRouting only\", expected exactly 1")
	})

	t.Run("multiple matches", func(t *testing.T) {
		schema := &apiextensionsv1.JSONSchemaProps{
			XValidations: []apiextensionsv1.ValidationRule{
				{Rule: "old-rule-1", Message: "phase PreRouting only supports extAuth"},
				{Rule: "old-rule-2", Message: "phase PreRouting only supports rateLimit"},
			},
		}

		err := applyOverrideXValidation(schema, OverrideXValidation{
			MessageContains: "phase PreRouting only",
			Rule:            "new-rule",
		})
		require.EqualError(t, err, "OverrideXValidation matched 2 rules for messageContains \"phase PreRouting only\", expected exactly 1")
	})
}

func TestApplyIfThenOnlyFieldsUsesVisibleSchemaFields(t *testing.T) {
	schema := &apiextensionsv1.JSONSchemaProps{
		Properties: map[string]apiextensionsv1.JSONSchemaProps{
			"foo": {},
			"bar": {},
			"baz": {},
		},
	}

	err := applyIfThenOnlyFields(schema, sortedPropertyNames(schema), IfThenOnlyFields{
		If:     "has(self.baz)",
		Fields: []string{"foo", "baz"},
	})
	require.NoError(t, err)
	require.Len(t, schema.XValidations, 1)
	require.Equal(t, "has(self.baz) ? [has(self.bar)].filter(x,x==true).size() == 0 : true", schema.XValidations[0].Rule)
	require.Equal(t, "only fields in [baz foo] may be set when has(self.baz)", schema.XValidations[0].Message)
}

func TestApplyPostSchemaMarkersToCRDIfThenOnlyFieldsIncludesImportedInlineFields(t *testing.T) {
	const rootPath = "github.com/agentgateway/agentgateway/controller/hack/crdgen/testdata/ifthenembedded/api/v1"

	roots, parser := newTestParser(t, rootPath)
	metav1Pkg := crd.FindMetav1(roots)
	require.NotNil(t, metav1Pkg)

	kubeKinds := crd.FindKubeKinds(parser, metav1Pkg)
	require.Len(t, kubeKinds, 1)

	groupKind := kubeKinds[0]
	maxDescLen := 0
	parser.NeedCRDFor(groupKind, &maxDescLen)

	crdObj := parser.CustomResourceDefinitions[groupKind]
	require.NoError(t, applyPostSchemaMarkersToCRD(parser, &crdObj, groupKind))

	require.Len(t, crdObj.Spec.Versions, 1)
	rootSchema := crdObj.Spec.Versions[0].Schema.OpenAPIV3Schema
	require.NotNil(t, rootSchema)

	specSchema, ok := rootSchema.Properties["spec"]
	require.True(t, ok)
	trafficSchema, ok := specSchema.Properties["traffic"]
	require.True(t, ok)

	require.Len(t, trafficSchema.XValidations, 1)
	require.Equal(t, "when baz is set only foo and baz may be set", trafficSchema.XValidations[0].Message)
	require.Equal(t, "has(self.baz) ? [has(self.bar)].filter(x,x==true).size() == 0 : true", trafficSchema.XValidations[0].Rule)
}
