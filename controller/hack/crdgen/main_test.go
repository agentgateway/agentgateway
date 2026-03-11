package main

import (
	"testing"

	"github.com/stretchr/testify/require"
	"sigs.k8s.io/controller-tools/pkg/crd"
	"sigs.k8s.io/controller-tools/pkg/loader"
	"sigs.k8s.io/controller-tools/pkg/markers"
)

func TestAllJSONFieldNamesForTypeIncludesImportedInlineFields(t *testing.T) {
	const rootPath = "github.com/agentgateway/agentgateway/controller/hack/crdgen/testdata/root"

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
